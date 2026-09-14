use std::{
    io::Read,
    path::{Path, PathBuf},
};

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

    #[error("dotenv environment lookup failed: variable is not present")]
    EnvVarNotPresent,

    #[error("dotenv environment lookup failed: variable contains non-Unicode data")]
    EnvVarNotUnicode,

    #[error("dotenv variable substitution is disabled at input index {index}")]
    SubstitutionDisabled { index: usize },

    #[error("dotenv parse error")]
    Other,
}

impl From<dotenvy::Error> for DotenvReadError {
    fn from(error: dotenvy::Error) -> Self {
        match error {
            dotenvy::Error::LineParse(_, index) => Self::Syntax { index },
            dotenvy::Error::Io(error) => Self::Io(error),
            dotenvy::Error::EnvVar(std::env::VarError::NotPresent) => Self::EnvVarNotPresent,
            dotenvy::Error::EnvVar(std::env::VarError::NotUnicode(_)) => Self::EnvVarNotUnicode,
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
    allow_substitution: bool,
}

impl<'i> DotenvProvider<'i> {
    /// Build a dotenv provider from any supported file-content source.
    pub fn new(src: impl IntoFileContentProvider<'i>) -> Self {
        Self {
            source: src.into_provider(),
            selection: EnvProvider::new(),
            allow_substitution: true,
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

    /// Reject dotenv variable substitution such as `$HOME` and `${HOME}`.
    ///
    /// `dotenvy` normally resolves substitutions from the process environment first and then from
    /// earlier dotenv entries. Selection/allowlisting controls which final keys enter the config,
    /// not which variables may participate in those substitutions. Use this mode when the dotenv
    /// file must be isolated from the process environment. Single-quoted dollar signs and escaped
    /// dollar signs remain literal and are allowed.
    pub fn without_substitution(mut self) -> Self {
        self.allow_substitution = false;
        self
    }

    /// Convert borrowed source data to owned data.
    pub fn into_owned(self) -> Result<DotenvProvider<'static>, ConfigError> {
        Ok(DotenvProvider {
            source: self.source.into_owned()?,
            selection: self.selection,
            allow_substitution: self.allow_substitution,
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

fn collect_vars(
    reader: impl Read,
) -> Result<Vec<(std::ffi::OsString, std::ffi::OsString)>, ConfigError> {
    dotenvy::from_read_iter(reader)
        .map(|result| {
            result
                .map(|(key, value)| (key.into(), value.into()))
                .map_err(|error| ConfigError::provider("dotenv", DotenvReadError::from(error)))
        })
        .collect()
}

fn substitution_index(input: &str) -> Option<usize> {
    #[derive(Clone, Copy, Eq, PartialEq)]
    enum Quote {
        None,
        Single,
        Double,
    }

    let mut offset = 0;
    for line in input.split_inclusive('\n') {
        let content = line.strip_suffix('\n').unwrap_or(line);
        let trimmed = content.trim_start_matches(char::is_whitespace);
        if trimmed.is_empty() || trimmed.starts_with('#') {
            offset += line.len();
            continue;
        }

        // dotenv keys cannot contain `$`, so only inspect the value. If the line is malformed and
        // has no `=`, let dotenvy report its ordinary syntax error.
        let Some(equal) = content.find('=') else {
            offset += line.len();
            continue;
        };

        let raw_value_start = equal + 1;
        let raw_value = &content[raw_value_start..];
        let value = raw_value.trim_start_matches(char::is_whitespace);
        let value_start = raw_value_start + (raw_value.len() - value.len());
        let mut quote = Quote::None;
        let mut escaped = false;
        let mut expecting_end = false;

        for (index, ch) in value.char_indices() {
            if escaped {
                escaped = false;
                continue;
            }

            match quote {
                Quote::Single => {
                    if ch == '\'' {
                        quote = Quote::None;
                    }
                }
                Quote::Double => match ch {
                    '\\' => escaped = true,
                    '"' => quote = Quote::None,
                    '$' => return Some(offset + value_start + index),
                    _ => {}
                },
                Quote::None => {
                    if expecting_end {
                        match ch {
                            ' ' | '\t' => continue,
                            '#' => break,
                            // dotenvy will reject any other token after trailing whitespace. A `$`
                            // here therefore cannot be a successful substitution.
                            _ => break,
                        }
                    }

                    match ch {
                        '\\' => escaped = true,
                        '\'' => quote = Quote::Single,
                        '"' => quote = Quote::Double,
                        ' ' | '\t' => expecting_end = true,
                        '$' => return Some(offset + value_start + index),
                        _ => {}
                    }
                }
            }
        }

        offset += line.len();
    }

    None
}

impl ConfigProvider for DotenvProvider<'_> {
    fn load_partial<T: DeserializeOwned>(&self) -> Result<T, ConfigError> {
        self.source.with_reader(|reader| {
            let vars = if self.allow_substitution {
                collect_vars(reader)?
            } else {
                let mut input = String::new();
                reader.read_to_string(&mut input)?;
                if let Some(index) = substitution_index(&input) {
                    return Err(ConfigError::provider(
                        "dotenv",
                        DotenvReadError::SubstitutionDisabled { index },
                    ));
                }
                collect_vars(input.as_bytes())?
            };

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
    fn isolated_mode_rejects_variable_substitution() {
        let provider = DotenvProvider::from_contents("API_KEY=${HOME}\nSOURCE_PATH=/srv/project\n")
            .with_vars(["API_KEY", "SOURCE_PATH"])
            .without_substitution();

        let error = provider.load_partial::<LocalConfig>().unwrap_err();
        let message = error.to_string();

        assert!(
            message.contains("variable substitution is disabled"),
            "{message}"
        );
        assert!(!message.contains("HOME"), "{message}");
    }

    #[test]
    fn isolated_mode_rejects_substitution_after_value_whitespace() {
        let provider =
            DotenvProvider::from_contents("API_KEY=   $HOME\nSOURCE_PATH=/srv/project\n")
                .with_vars(["API_KEY", "SOURCE_PATH"])
                .without_substitution();

        let error = provider.load_partial::<LocalConfig>().unwrap_err();
        assert!(
            error
                .to_string()
                .contains("variable substitution is disabled"),
            "{error}"
        );
    }

    #[test]
    fn isolated_mode_allows_literal_dollar_signs() {
        let quoted = DotenvProvider::from_contents("API_KEY='$HOME'\nSOURCE_PATH=/srv/project\n")
            .with_vars(["API_KEY", "SOURCE_PATH"])
            .without_substitution()
            .load_partial::<LocalConfig>()
            .unwrap();
        assert_eq!(quoted.api_key, "$HOME");

        let escaped = DotenvProvider::from_contents("API_KEY=\\$HOME\nSOURCE_PATH=/srv/project\n")
            .with_vars(["API_KEY", "SOURCE_PATH"])
            .without_substitution()
            .load_partial::<LocalConfig>()
            .unwrap();
        assert_eq!(escaped.api_key, "$HOME");
    }

    #[test]
    fn isolated_mode_ignores_substitution_syntax_in_comments() {
        let config = DotenvProvider::from_contents(
            "# $HOME is documentation\nAPI_KEY=secret # $HOME is also a comment\nSOURCE_PATH=/srv/project\n",
        )
        .with_vars(["API_KEY", "SOURCE_PATH"])
        .without_substitution()
        .load_partial::<LocalConfig>()
        .unwrap();

        assert_eq!(config.api_key, "secret");
    }

    #[test]
    fn environment_lookup_errors_do_not_expose_values() {
        let error = DotenvReadError::from(dotenvy::Error::EnvVar(std::env::VarError::NotUnicode(
            std::ffi::OsString::from("super-secret-value"),
        )));
        let display = error.to_string();
        let debug = format!("{error:?}");

        assert!(!display.contains("super-secret-value"), "{display}");
        assert!(!debug.contains("super-secret-value"), "{debug}");
        assert!(display.contains("non-Unicode"), "{display}");
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
