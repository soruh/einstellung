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
    MissingExtension { path: PathBuf },

    #[error("unsupported configuration file extension {extension:?}")]
    UnsupportedExtension { extension: String },
}

/// Built-in structured configuration formats available to [`FormatProvider`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ConfigFormat {
    #[cfg(feature = "json")]
    Json,
    #[cfg(feature = "toml")]
    Toml,
    #[cfg(feature = "yaml")]
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
    format: ConfigFormat,
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
    pub fn from_path_detect(path: &'i Path) -> Result<Self, ConfigError> {
        let extension = path.extension().ok_or_else(|| {
            ConfigError::provider(
                "format",
                FormatProviderError::MissingExtension {
                    path: path.to_path_buf(),
                },
            )
        })?;

        let format = ConfigFormat::from_extension(extension).ok_or_else(|| {
            ConfigError::provider(
                "format",
                FormatProviderError::UnsupportedExtension {
                    extension: extension.to_string_lossy().into_owned(),
                },
            )
        })?;

        Ok(Self::from_path(format, path))
    }

    /// Convert borrowed source data to owned data.
    pub fn into_owned(self) -> FormatProvider<'static> {
        FormatProvider {
            format: self.format,
            source: self.source.into_owned(),
        }
    }
}

impl ConfigProvider for FormatProvider<'_> {
    fn load_partial<T: DeserializeOwned>(&self) -> Result<T, ConfigError> {
        match self.format {
            #[cfg(feature = "json")]
            ConfigFormat::Json => self
                .source
                .with_reader(|reader| Ok(serde_json::from_reader(reader)?)),
            #[cfg(feature = "toml")]
            ConfigFormat::Toml => self.source.with_reader(|reader| {
                let mut buffer = String::new();
                reader.read_to_string(&mut buffer)?;
                ::toml::from_str(&buffer)
                    .map_err(|error| crate::TomlError::with_input(error, &buffer).into())
            }),
            #[cfg(feature = "yaml")]
            ConfigFormat::Yaml => self
                .source
                .with_reader(|reader| Ok(serde_saphyr::from_reader(reader)?)),
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
mod tests {
    use serde::Deserialize;

    use super::*;

    #[derive(Debug, Deserialize, Eq, PartialEq)]
    struct TestConfig {
        name: String,
        port: u16,
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
}
