//! Runtime selection among enabled structured configuration formats.

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

use serde::de::DeserializeOwned;
use thiserror::Error;

use crate::{ConfigError, ConfigProvider, FileContentProvider, IntoFileContentProvider};

/// Errors produced while selecting a built-in configuration format at runtime.
#[derive(Debug, Error)]
pub enum FormatProviderError {
    #[error("configuration path {path:?} has no file extension")]
    /// The path has no extension from which to select a format.
    MissingExtension {
        /// Destination path or filesystem path involved in the failure.
        path: PathBuf,
    },

    #[error("unsupported configuration file extension {extension:?}")]
    /// No enabled parser recognizes the path’s extension.
    UnsupportedExtension {
        /// Unrecognized file extension.
        extension: String,
    },
}

/// Built-in structured configuration formats available to [`FormatProvider`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ConfigFormat {
    #[cfg(feature = "json")]
    /// Decode the source as JSON.
    Json,
    #[cfg(feature = "toml")]
    /// Decode the source as TOML.
    Toml,
    #[cfg(feature = "yaml")]
    /// Decode the source as YAML.
    Yaml,
}

impl ConfigFormat {
    /// Detect an enabled configuration format from a file extension.
    ///
    /// The leading `.` is optional and matching is ASCII case-insensitive. YAML accepts both
    /// `yaml` and `yml`.
    pub fn from_extension(extension: &OsStr) -> Option<Self> {
        let extension = extension.to_str()?.trim_start_matches('.');

        #[cfg(feature = "json")]
        if extension.eq_ignore_ascii_case("json") {
            return Some(Self::Json);
        }

        #[cfg(feature = "toml")]
        if extension.eq_ignore_ascii_case("toml") {
            return Some(Self::Toml);
        }

        #[cfg(feature = "yaml")]
        if extension.eq_ignore_ascii_case("yaml") || extension.eq_ignore_ascii_case("yml") {
            return Some(Self::Yaml);
        }

        None
    }

    /// Detect an enabled configuration format from a path's extension.
    pub fn from_path(path: &Path) -> Option<Self> {
        Self::from_extension(path.extension()?)
    }
}

/// Runtime-dispatched provider for the built-in structured file formats.
///
/// [`ConfigProvider`] itself has a generic method and therefore is not object-safe. This provider
/// gives applications a single concrete type when the input format is only known at runtime.
pub struct FormatProvider<'i> {
    /// Selected structured format used when loading the source.
    format: ConfigFormat,
    /// Underlying source retained for loading or contextual diagnostics.
    source: FileContentProvider<'i>,
}

impl<'i> FormatProvider<'i> {
    /// Build a provider from a runtime format and any supported file-content source.
    pub fn new(format: ConfigFormat, source: impl IntoFileContentProvider<'i>) -> Self {
        Self {
            format,
            source: source.into_provider(),
        }
    }

    /// Build a provider from inline configuration contents.
    pub fn from_contents(format: ConfigFormat, source: &'i str) -> Self {
        Self::new(format, FileContentProvider::InlineBorrowed(source))
    }

    /// Build a provider from a filesystem path and an explicitly selected format.
    pub fn from_path(format: ConfigFormat, path: &'i Path) -> Self {
        Self::new(format, FileContentProvider::PathBorrowed(path))
    }

    /// Build a provider from a filesystem path, detecting the format from its extension.
    ///
    /// Only formats enabled by the crate's Cargo features are recognized.
    ///
    /// # Errors
    ///
    /// Returns an error when the path has no extension or no enabled format recognizes it.
    pub fn from_path_detect(path: &'i Path) -> Result<Self, ConfigError> {
        Ok(Self::from_path(detect_format(path)?, path))
    }

    /// Convert borrowed source data to owned data.
    ///
    /// # Errors
    ///
    /// Returns an error if a borrowed custom reader factory cannot be cloned.
    /// Built-in inline and filesystem sources convert without performing I/O.
    pub fn into_owned(self) -> Result<FormatProvider<'static>, ConfigError> {
        Ok(FormatProvider {
            format: self.format,
            source: self.source.into_owned()?,
        })
    }
}

impl FormatProvider<'static> {
    /// Build an owned provider from inline configuration contents.
    pub fn from_owned_contents(format: ConfigFormat, source: String) -> Self {
        Self::new(format, FileContentProvider::InlineOwned(source))
    }

    /// Build an owned provider from a filesystem path and an explicitly selected format.
    pub fn from_path_buf(format: ConfigFormat, path: PathBuf) -> Self {
        Self::new(format, FileContentProvider::PathOwned(path))
    }

    /// Build an owned provider from a filesystem path, detecting the format from its extension.
    ///
    /// # Errors
    ///
    /// Returns an error when the path has no extension or no enabled format recognizes it.
    pub fn from_path_buf_detect(path: PathBuf) -> Result<Self, ConfigError> {
        let format = detect_format(&path)?;
        Ok(Self::from_path_buf(format, path))
    }
}

/// Select an enabled parser from a path extension, reporting unsupported paths.
///
/// # Errors
///
/// Returns an error if the path lacks an extension or no enabled parser recognizes it.
fn detect_format(path: &Path) -> Result<ConfigFormat, ConfigError> {
    let extension = path.extension().ok_or_else(|| {
        ConfigError::provider(
            "format",
            FormatProviderError::MissingExtension {
                path: path.to_path_buf(),
            },
        )
    })?;

    ConfigFormat::from_extension(extension).ok_or_else(|| {
        ConfigError::provider(
            "format",
            FormatProviderError::UnsupportedExtension {
                extension: extension.to_string_lossy().into_owned(),
            },
        )
    })
}

impl ConfigProvider for FormatProvider<'_> {
    fn load_partial<T: DeserializeOwned>(&self) -> Result<T, ConfigError> {
        match self.format {
            #[cfg(feature = "json")]
            ConfigFormat::Json => super::json::load_json(&self.source),
            #[cfg(feature = "toml")]
            ConfigFormat::Toml => super::toml::load_toml(&self.source),
            #[cfg(feature = "yaml")]
            ConfigFormat::Yaml => super::yaml::load_yaml(&self.source),
        }
    }

    fn source(&self) -> crate::ConfigSource {
        let format = match self.format {
            #[cfg(feature = "json")]
            ConfigFormat::Json => "json",
            #[cfg(feature = "toml")]
            ConfigFormat::Toml => "toml",
            #[cfg(feature = "yaml")]
            ConfigFormat::Yaml => "yaml",
        };
        crate::ConfigSource::new(self.source.source_label(format))
    }
}

#[cfg(test)]
#[allow(
    missing_docs,
    clippy::missing_docs_in_private_items,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    reason = "Test fixtures model user input rather than library APIs."
)]
mod tests {
    use serde::Deserialize;

    use super::*;

    #[derive(Debug, Deserialize, Eq, PartialEq)]
    struct TestConfig {
        name: String,
        port: u16,
    }

    #[test]
    fn structured_errors_retain_nested_paths_and_safe_diagnostics() {
        #[derive(Debug, Deserialize)]
        #[allow(dead_code)]
        struct Nested {
            #[serde(rename = "service-nodes")]
            nodes: Vec<TestConfig>,
        }

        let cases = [
            #[cfg(feature = "json")]
            (
                ConfigFormat::Json,
                r#"{"service-nodes":[{"name":"api","port":"super-secret"}]}"#,
            ),
            #[cfg(feature = "toml")]
            (
                ConfigFormat::Toml,
                "[[service-nodes]]\nname = \"api\"\nport = \"super-secret\"\n",
            ),
            #[cfg(feature = "yaml")]
            (
                ConfigFormat::Yaml,
                "service-nodes:\n  - name: api\n    port: super-secret\n",
            ),
        ];
        for (format, contents) in cases {
            let error = FormatProvider::from_contents(format, contents)
                .load_partial_with_source::<Nested>()
                .unwrap_err();
            assert_eq!(
                error.logical_path().as_deref(),
                Some("service-nodes.0.port")
            );
            assert!(error.source_location().is_some());
            assert_eq!(error.field_sources().len(), 1);
            assert!(!error.to_string().contains("super-secret"));
            assert!(!format!("{error:?}").contains("super-secret"));
            assert!(error.root_cause().logical_path().is_none());
        }
    }

    #[cfg(feature = "json")]
    #[test]
    fn loads_json_selected_at_runtime() {
        let format = ConfigFormat::Json;
        let provider = FormatProvider::from_contents(format, r#"{"name":"api","port":8080}"#);
        let config = provider.load_partial::<TestConfig>().unwrap();

        assert_eq!(
            config,
            TestConfig {
                name: "api".to_owned(),
                port: 8080,
            }
        );
    }

    #[cfg(feature = "toml")]
    #[test]
    fn loads_toml_selected_at_runtime() {
        let format = ConfigFormat::Toml;
        let provider = FormatProvider::from_contents(format, "name = \"api\"\nport = 8080\n");
        let config = provider.load_partial::<TestConfig>().unwrap();

        assert_eq!(config.port, 8080);
    }

    #[cfg(feature = "yaml")]
    #[test]
    fn loads_yaml_selected_at_runtime() {
        let format = ConfigFormat::Yaml;
        let provider = FormatProvider::from_contents(format, "name: api\nport: 8080\n");
        let config = provider.load_partial::<TestConfig>().unwrap();

        assert_eq!(config.name, "api");
    }

    #[test]
    fn detects_enabled_formats_from_extension() {
        #[cfg(feature = "json")]
        assert_eq!(
            ConfigFormat::from_extension(OsStr::new("JSON")),
            Some(ConfigFormat::Json)
        );
        #[cfg(feature = "toml")]
        assert_eq!(
            ConfigFormat::from_extension(OsStr::new(".toml")),
            Some(ConfigFormat::Toml)
        );
        #[cfg(feature = "yaml")]
        {
            assert_eq!(
                ConfigFormat::from_extension(OsStr::new("yaml")),
                Some(ConfigFormat::Yaml)
            );
            assert_eq!(
                ConfigFormat::from_extension(OsStr::new("YML")),
                Some(ConfigFormat::Yaml)
            );
        }
    }

    #[test]
    fn path_detection_rejects_unknown_extensions() {
        let err = FormatProvider::from_path_detect(Path::new("config.ini"))
            .err()
            .expect("unsupported extension should fail");

        assert!(
            err.to_string()
                .contains("unsupported configuration file extension")
        );
    }

    #[test]
    fn path_detection_rejects_extensionless_paths() {
        let err = FormatProvider::from_path_detect(Path::new("config"))
            .err()
            .expect("extensionless path should fail");

        assert!(err.to_string().contains("has no file extension"));
    }

    #[cfg(feature = "json")]
    #[test]
    fn owned_constructors_preserve_runtime_format_and_source_kind() {
        let provider = FormatProvider::from_owned_contents(
            ConfigFormat::Json,
            r#"{"name":"api","port":8080}"#.to_owned(),
        );
        let config = provider.load_partial::<TestConfig>().unwrap();
        assert_eq!(config.port, 8080);
        assert_eq!(provider.source().label(), "inline json");

        let provider = FormatProvider::from_path_buf_detect(PathBuf::from("config.JSON")).unwrap();
        assert_eq!(provider.format, ConfigFormat::Json);
        assert_eq!(provider.source().label(), "json file config.JSON");
    }
}
