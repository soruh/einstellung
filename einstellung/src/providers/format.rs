use std::path::Path;

use serde::de::DeserializeOwned;

use crate::{ConfigError, ConfigProvider, FileContentProvider, IntoFileContentProvider};

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

    /// Build a provider from a filesystem path.
    pub fn from_path(format: ConfigFormat, path: &'i Path) -> Self {
        Self::new(format, FileContentProvider::PathBorrowed(path))
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
                Ok(::toml::from_str(&buffer)?)
            }),
            #[cfg(feature = "yaml")]
            ConfigFormat::Yaml => self
                .source
                .with_reader(|reader| Ok(serde_yaml::from_reader(reader)?)),
        }
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
}
