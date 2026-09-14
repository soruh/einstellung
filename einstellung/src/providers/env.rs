use std::ffi::{OsStr, OsString};

use serde::de::DeserializeOwned;
use thiserror::Error;

use crate::{ConfigError, ConfigProvider};

use super::key_value::{KeyValueProviderError, MappedValue, load_mapped_values};

const NESTED_SEPARATOR: &str = "__";

/// Errors produced while selecting or decoding environment variables.
#[derive(Debug, Error)]
pub enum EnvProviderError {
    #[error("environment variable {variable:?} contains non-Unicode data")]
    NonUnicode { variable: String },

    #[error("environment mapping for {path:?} conflicts with another mapped value")]
    ConflictingPath { path: String },

    #[error("invalid value for environment variable {variable:?}: {message}")]
    InvalidValue { variable: String, message: String },
}

impl From<KeyValueProviderError> for EnvProviderError {
    fn from(error: KeyValueProviderError) -> Self {
        match error {
            KeyValueProviderError::ConflictingPath { path } => Self::ConflictingPath { path },
            KeyValueProviderError::InvalidValue { input, message } => Self::InvalidValue {
                variable: input,
                message,
            },
        }
    }
}

/// Loads explicitly selected process environment variables into a partial configuration.
///
/// The provider intentionally loads nothing unless at least one explicit mapping or prefix
/// is configured. This avoids accidentally exposing unrelated environment variables to a
/// configuration type.
///
/// Explicit variables use dotted configuration paths, for example
/// `with_var("DATABASE_URL", "database.url")`. A configured prefix maps matching variables
/// by removing the prefix, converting the remainder to ASCII lowercase, and treating `__`
/// as a nested field separator. For example `APP_DATABASE__URL` maps to `database.url`.
#[derive(Clone, Debug, Default)]
pub struct EnvProvider {
    vars: Vec<EnvBinding>,
    prefix: Option<String>,
}

#[derive(Clone, Debug)]
struct EnvBinding {
    variable: String,
    path: String,
}

impl EnvProvider {
    /// Create an environment provider that loads no variables until configured.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a provider that loads only the named environment variables.
    ///
    /// Variable names are mapped to configuration paths by lowercasing them and treating `__`
    /// as a nested-field separator. For example, `API_KEY` maps to `api_key` and
    /// `DATABASE__URL` maps to `database.url`. Use [`Self::with_var`] when a variable needs an
    /// explicit mapping.
    pub fn only<I, S>(variables: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::new().with_vars(variables)
    }

    /// Add environment variables whose names map directly to configuration paths.
    ///
    /// This uses the same lowercase and `__` nesting rules as [`Self::only`].
    pub fn with_vars<I, S>(mut self, variables: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for variable in variables {
            let variable = variable.into();
            let path = env_key_path(&variable).join(".");
            self = self.with_var(variable, path);
        }
        self
    }

    /// Create a provider that loads variables under the given prefix.
    pub fn prefixed(prefix: impl Into<String>) -> Self {
        Self::new().with_prefix(prefix)
    }

    /// Add an explicit environment variable to configuration-field mapping.
    ///
    /// `config_path` uses `.` to address nested fields, such as `database.url`.
    pub fn with_var(mut self, variable: impl Into<String>, config_path: impl Into<String>) -> Self {
        self.vars.push(EnvBinding {
            variable: variable.into(),
            path: config_path.into(),
        });
        self
    }

    /// Load all variables under the given prefix.
    ///
    /// Prefix-based mappings lowercase the variable name after the prefix and use `__`
    /// as the nested-field separator. Explicit [`Self::with_var`] mappings take precedence
    /// when both sources map to the same configuration path.
    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = Some(prefix.into());
        self
    }

    pub(crate) fn load_from_vars<T>(
        &self,
        provider_name: &'static str,
        vars: impl IntoIterator<Item = (OsString, OsString)>,
    ) -> Result<T, ConfigError>
    where
        T: DeserializeOwned,
    {
        let vars: Vec<_> = vars.into_iter().collect();
        let mut mapped = Vec::new();
        let mut input_paths = std::collections::BTreeMap::new();

        if let Some(prefix) = &self.prefix {
            for (key, value) in &vars {
                let Some(key) = key.to_str() else {
                    continue;
                };
                let Some(suffix) = key.strip_prefix(prefix) else {
                    continue;
                };
                if suffix.is_empty() {
                    continue;
                }

                let path = env_key_path(suffix);
                input_paths.insert(key.to_owned(), path.join("."));
                mapped.push(mapped_env_value(provider_name, path, key, value)?);
            }
        }

        for binding in &self.vars {
            let Some((_, value)) = vars
                .iter()
                .find(|(key, _)| key.as_os_str() == OsStr::new(&binding.variable))
            else {
                continue;
            };

            let path = binding.path.split('.').map(str::to_owned).collect();
            input_paths.insert(binding.variable.clone(), binding.path.clone());
            mapped.push(mapped_env_value(
                provider_name,
                path,
                &binding.variable,
                value,
            )?);
        }

        load_mapped_values(mapped).map_err(|error| {
            let path = match &error {
                KeyValueProviderError::ConflictingPath { path } => path.clone(),
                KeyValueProviderError::InvalidValue { input, .. } => input_paths
                    .get(input)
                    .cloned()
                    .unwrap_or_else(|| input.clone()),
            };
            ConfigError::provider_at(provider_name, path, EnvProviderError::from(error))
        })
    }
}

impl ConfigProvider for EnvProvider {
    fn load_partial<T: DeserializeOwned>(&self) -> Result<T, ConfigError> {
        self.load_from_vars("environment", std::env::vars_os())
    }

    fn source(&self) -> crate::ConfigSource {
        crate::ConfigSource::new("process environment")
    }
}

fn env_key_path(key: &str) -> Vec<String> {
    key.split(NESTED_SEPARATOR)
        .map(str::to_ascii_lowercase)
        .collect()
}

fn mapped_env_value(
    provider_name: &'static str,
    path: Vec<String>,
    variable: &str,
    value: &OsStr,
) -> Result<MappedValue, ConfigError> {
    let value = value
        .to_str()
        .ok_or_else(|| {
            ConfigError::provider_at(
                provider_name,
                path.join("."),
                EnvProviderError::NonUnicode {
                    variable: variable.to_owned(),
                },
            )
        })?
        .to_owned();

    Ok(MappedValue::new(path, variable.to_owned(), value))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use serde::Deserialize;

    use super::*;

    #[derive(Debug, Deserialize, PartialEq)]
    struct TestConfig {
        api_key: String,
        source_path: String,
        port: u16,
        enabled: bool,
        database: DatabaseConfig,
        tags: Vec<String>,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct DatabaseConfig {
        url: String,
    }

    fn vars(values: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        values
            .iter()
            .map(|(key, value)| (OsString::from(key), OsString::from(value)))
            .collect()
    }

    #[test]
    fn explicit_mappings_are_allowlisted_and_typed() {
        let provider = EnvProvider::new()
            .with_var("API_KEY", "api_key")
            .with_var("SOURCE_PATH", "source_path")
            .with_var("PORT", "port")
            .with_var("ENABLED", "enabled")
            .with_var("DATABASE_URL", "database.url")
            .with_var("TAGS", "tags");

        let config = provider
            .load_from_vars::<TestConfig>(
                "environment",
                vars(&[
                    ("API_KEY", "123"),
                    ("SOURCE_PATH", "/tmp/project"),
                    ("PORT", "8080"),
                    ("ENABLED", "true"),
                    ("DATABASE_URL", "postgres://localhost/app"),
                    ("TAGS", r#"["api","worker"]"#),
                    ("IGNORED", "does-not-load"),
                ]),
            )
            .unwrap();

        assert_eq!(config.api_key, "123");
        assert_eq!(config.source_path, "/tmp/project");
        assert_eq!(config.port, 8080);
        assert!(config.enabled);
        assert_eq!(config.database.url, "postgres://localhost/app");
        assert_eq!(config.tags, ["api", "worker"]);
    }

    #[test]
    fn only_maps_selected_variables_by_name() {
        let provider = EnvProvider::only([
            "API_KEY",
            "SOURCE_PATH",
            "PORT",
            "ENABLED",
            "DATABASE__URL",
            "TAGS",
        ]);
        let config = provider
            .load_from_vars::<TestConfig>(
                "environment",
                vars(&[
                    ("API_KEY", "secret"),
                    ("SOURCE_PATH", "/srv/app"),
                    ("PORT", "443"),
                    ("ENABLED", "true"),
                    ("DATABASE__URL", "postgres://db/app"),
                    ("TAGS", r#"["one"]"#),
                    ("IGNORED", "does-not-load"),
                ]),
            )
            .unwrap();

        assert_eq!(config.api_key, "secret");
        assert_eq!(config.database.url, "postgres://db/app");
        assert_eq!(config.tags, ["one"]);
    }

    #[test]
    fn prefix_maps_nested_fields() {
        let provider = EnvProvider::prefixed("APP_");
        let config = provider
            .load_from_vars::<TestConfig>(
                "environment",
                vars(&[
                    ("APP_API_KEY", "secret"),
                    ("APP_SOURCE_PATH", "/srv/app"),
                    ("APP_PORT", "443"),
                    ("APP_ENABLED", "false"),
                    ("APP_DATABASE__URL", "postgres://db/app"),
                    ("APP_TAGS", r#"["one"]"#),
                    ("OTHER_PORT", "80"),
                ]),
            )
            .unwrap();

        assert_eq!(config.api_key, "secret");
        assert_eq!(config.port, 443);
        assert!(!config.enabled);
        assert_eq!(config.database.url, "postgres://db/app");
    }

    #[test]
    fn explicit_mapping_overrides_prefix_mapping() {
        let provider = EnvProvider::new()
            .with_prefix("APP_")
            .with_var("SPECIAL_PORT", "port");
        let config = provider
            .load_from_vars::<TestConfig>(
                "environment",
                vars(&[
                    ("APP_API_KEY", "secret"),
                    ("APP_SOURCE_PATH", "/srv/app"),
                    ("APP_PORT", "443"),
                    ("APP_ENABLED", "true"),
                    ("APP_DATABASE__URL", "postgres://db/app"),
                    ("APP_TAGS", "[]"),
                    ("SPECIAL_PORT", "8443"),
                ]),
            )
            .unwrap();

        assert_eq!(config.port, 8443);
    }
    #[test]
    fn conversion_errors_report_destination_path() {
        #[derive(Debug, Deserialize)]
        #[allow(dead_code)]
        struct Config {
            database: Database,
        }

        #[derive(Debug, Deserialize)]
        #[allow(dead_code)]
        struct Database {
            port: u16,
        }

        let provider = EnvProvider::new().with_var("DATABASE_PORT", "database.port");
        let error = provider
            .load_from_vars::<Config>(
                "environment",
                [(
                    OsString::from("DATABASE_PORT"),
                    OsString::from("super-secret-value"),
                )],
            )
            .unwrap_err();

        assert_eq!(error.logical_path().as_deref(), Some("database.port"));
        assert!(!error.to_string().contains("super-secret-value"));
    }
}
