use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use thiserror::Error;

use crate::{ConfigError, ConfigProvider, FileContentProvider, IntoFileContentProvider};

use super::EnvProvider;

#[derive(Debug, Error)]
enum DotenvReadError {
    #[error("dotenv syntax error at input index {index}")]
    Syntax { index: usize },

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    EnvVar(#[from] std::env::VarError),

    #[error("dotenv parse error")]
    Other,
}

impl From<dotenvy::Error> for DotenvReadError {
    fn from(error: dotenvy::Error) -> Self {
        match error {
            dotenvy::Error::LineParse(_, index) => Self::Syntax { index },
            dotenvy::Error::Io(error) => Self::Io(error),
            dotenvy::Error::EnvVar(error) => Self::EnvVar(error),
            _ => Self::Other,
        }
    }
}

/// Loads selected values from dotenv-formatted input without modifying the process environment.
///
/// Like [`EnvProvider`], this provider loads no variables until mappings or a prefix are configured.
/// This makes it suitable for keeping `.env` files limited to secrets and machine-local values.
pub struct DotenvProvider<'i> {
    source: FileContentProvider<'i>,
    selection: EnvProvider,
}

impl<'i> DotenvProvider<'i> {
    /// Build a dotenv provider from any supported file-content source.
    pub fn new(src: impl IntoFileContentProvider<'i>) -> Self {
        Self {
            source: src.into_provider(),
            selection: EnvProvider::new(),
        }
    }

    /// Build a dotenv provider from inline contents.
    pub fn from_contents(src: &'i str) -> Self {
        Self::new(FileContentProvider::InlineBorrowed(src))
    }

    /// Build a dotenv provider from a filesystem path.
    pub fn from_path(path: &'i Path) -> Self {
        Self::new(FileContentProvider::PathBorrowed(path))
    }

    /// Use an existing [`EnvProvider`] as the selection/mapping policy.
    pub fn with_env_provider(mut self, provider: EnvProvider) -> Self {
        self.selection = provider;
        self
    }

    /// Add an explicit dotenv variable to configuration-field mapping.
    pub fn with_var(mut self, variable: impl Into<String>, config_path: impl Into<String>) -> Self {
        self.selection = self.selection.with_var(variable, config_path);
        self
    }

    /// Add dotenv variables whose names map directly to configuration paths.
    ///
    /// Variable names are lowercased and `__` denotes nested fields, matching
    /// [`EnvProvider::only`].
    pub fn with_vars<I, S>(mut self, variables: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.selection = self.selection.with_vars(variables);
        self
    }

    /// Load all dotenv variables under the given prefix.
    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.selection = self.selection.with_prefix(prefix);
        self
    }

    /// Convert borrowed source data to owned data.
    pub fn into_owned(self) -> Result<DotenvProvider<'static>, ConfigError> {
        Ok(DotenvProvider {
            source: self.source.into_owned()?,
            selection: self.selection,
        })
    }
}

impl DotenvProvider<'static> {
    /// Build an owned provider from inline dotenv contents.
    pub fn from_owned_contents(src: String) -> Self {
        Self::new(FileContentProvider::InlineOwned(src))
    }

    /// Build an owned provider from a filesystem path.
    pub fn from_path_buf(path: PathBuf) -> Self {
        Self::new(FileContentProvider::PathOwned(path))
    }
}

impl ConfigProvider for DotenvProvider<'_> {
    fn load_partial<T: DeserializeOwned>(&self) -> Result<T, ConfigError> {
        self.source.with_reader(|reader| {
            let vars = dotenvy::from_read_iter(reader)
                .map(|result| {
                    result
                        .map(|(key, value)| (key.into(), value.into()))
                        .map_err(|err| ConfigError::provider("dotenv", DotenvReadError::from(err)))
                })
                .collect::<Result<Vec<_>, _>>()?;

            self.selection.load_from_vars("dotenv", vars)
        })
    }

    fn source(&self) -> crate::ConfigSource {
        crate::ConfigSource::new(self.source.source_label("dotenv"))
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    #[derive(Debug, Deserialize, PartialEq)]
    struct LocalConfig {
        api_key: String,
        source_path: String,
        #[serde(default)]
        model: Option<String>,
    }

    #[test]
    fn loads_only_selected_dotenv_keys() {
        let provider = DotenvProvider::from_contents(
            "API_KEY=secret\nSOURCE_PATH=/srv/project\nMODEL=should-not-load\n",
        )
        .with_vars(["API_KEY", "SOURCE_PATH"]);

        let config = provider.load_partial::<LocalConfig>().unwrap();

        assert_eq!(config.api_key, "secret");
        assert_eq!(config.source_path, "/srv/project");
        assert_eq!(config.model, None);
    }

    #[test]
    fn repeated_selected_variables_use_the_last_value() {
        let provider = DotenvProvider::from_contents(
            "API_KEY=first\nAPI_KEY=second\nSOURCE_PATH=/srv/project\n",
        )
        .with_vars(["API_KEY", "SOURCE_PATH"]);

        let config = provider.load_partial::<LocalConfig>().unwrap();

        assert_eq!(config.api_key, "second");
    }

    #[test]
    fn parse_errors_do_not_expose_dotenv_lines() {
        let provider =
            DotenvProvider::from_contents("API_KEY='super-secret\n").with_vars(["API_KEY"]);

        let error = provider.load_partial::<LocalConfig>().unwrap_err();
        let message = error.to_string();

        assert!(!message.contains("super-secret"), "{message}");
        assert!(message.contains("dotenv syntax error"), "{message}");
    }

    #[test]
    fn dotenv_parsing_does_not_write_process_environment() {
        const KEY: &str = "EINSTELLUNG_DOTENV_TEST_DO_NOT_SET";
        assert!(std::env::var_os(KEY).is_none());

        let provider = DotenvProvider::from_contents(
            "EINSTELLUNG_DOTENV_TEST_DO_NOT_SET=secret\nAPI_KEY=secret\nSOURCE_PATH=/tmp\n",
        )
        .with_vars(["API_KEY", "SOURCE_PATH"]);

        let _ = provider.load_partial::<LocalConfig>().unwrap();
        assert!(std::env::var_os(KEY).is_none());
    }
    #[test]
    fn mapped_conversion_errors_report_destination_path() {
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

        let error = DotenvProvider::from_contents("DATABASE_PORT=super-secret-value\n")
            .with_var("DATABASE_PORT", "database.port")
            .load_partial::<Config>()
            .unwrap_err();

        assert_eq!(error.logical_path().as_deref(), Some("database.port"));
        assert!(!error.to_string().contains("super-secret-value"));
    }
}
