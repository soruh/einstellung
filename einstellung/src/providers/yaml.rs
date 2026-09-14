use super::*;
use crate::{ConfigProvider, FileContentProvider, IntoFileContentProvider};

/// [`ConfigProvider`] which interperts the file contents as YAML
pub struct YamlFileProvider<'i>(pub FileContentProvider<'i>);

impl<'i> YamlFileProvider<'i> {
    /// Build a provider from any supported file-content source.
    ///
    /// Prefer [`Self::from_contents`] or [`Self::from_path`] when the source type is known,
    /// as they make the distinction between inline contents and filesystem paths explicit.
    pub fn new(src: impl IntoFileContentProvider<'i>) -> Self {
        Self(src.into_provider())
    }

    /// Build a provider from inline configuration text.
    pub fn from_contents(src: &'i str) -> Self {
        Self(FileContentProvider::InlineBorrowed(src))
    }

    /// Build a provider from a filesystem path.
    pub fn from_path(path: &'i std::path::Path) -> Self {
        Self(FileContentProvider::PathBorrowed(path))
    }

    pub fn into_owned(self) -> YamlFileProvider<'static> {
        YamlFileProvider(self.0.into_owned())
    }
}

impl YamlFileProvider<'static> {
    /// Build an owned provider from inline configuration text.
    pub fn from_owned_contents(src: String) -> Self {
        Self(FileContentProvider::InlineOwned(src))
    }

    /// Build an owned provider from a filesystem path.
    pub fn from_path_buf(path: std::path::PathBuf) -> Self {
        Self(FileContentProvider::PathOwned(path))
    }
}

impl<'i> ConfigProvider for YamlFileProvider<'i> {
    fn load_partial<T: serde::de::DeserializeOwned>(&self) -> Result<T, ConfigError> {
        self.0
            .with_reader(|reader| Ok(serde_yaml::from_reader(reader)?))
    }

    fn source(&self) -> crate::ConfigSource {
        crate::ConfigSource::new(self.0.source_label("yaml"))
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    #[derive(Debug, Deserialize, PartialEq)]
    struct TypedConfig {
        enabled: bool,
        retries: u16,
        ratio: f64,
        label: String,
        items: Vec<String>,
    }

    #[test]
    fn preserves_yaml_scalar_and_collection_types() {
        let provider = YamlFileProvider::from_contents(
            "enabled: true\nretries: 3\nratio: 0.5\nlabel: api\nitems:\n  - one\n  - two\n",
        );

        let config = provider.load_partial::<TypedConfig>().unwrap();

        assert_eq!(
            config,
            TypedConfig {
                enabled: true,
                retries: 3,
                ratio: 0.5,
                label: "api".to_owned(),
                items: vec!["one".to_owned(), "two".to_owned()],
            }
        );
    }

    #[test]
    fn rejects_duplicate_mapping_keys() {
        let err = YamlFileProvider::from_contents("---\nthing: true\nthing: false\n")
            .load_partial::<serde_yaml::Value>()
            .unwrap_err();
        let message = err.to_string();

        assert!(message.contains("duplicate entry with key \"thing\""));
        assert!(message.contains("line 2 column 1"));
    }

    #[test]
    fn deserializes_yaml_enum_tags() {
        #[derive(Debug, Deserialize, PartialEq)]
        enum Profile {
            ClassValidator { class_name: String },
        }

        #[derive(Debug, Deserialize, PartialEq)]
        struct Config {
            profile: Profile,
        }

        let config = YamlFileProvider::from_contents(
            "profile: !ClassValidator\n  class_name: ApplicationConfig\n",
        )
        .load_partial::<Config>()
        .unwrap();

        assert_eq!(
            config,
            Config {
                profile: Profile::ClassValidator {
                    class_name: "ApplicationConfig".to_owned(),
                },
            }
        );
    }

    #[test]
    fn parse_errors_retain_location_context() {
        #[derive(Debug, Deserialize)]
        #[allow(dead_code)]
        struct Config {
            retries: u16,
        }

        let err = YamlFileProvider::from_contents("retries: [\n")
            .load_partial::<Config>()
            .unwrap_err();
        let message = err.to_string();

        assert!(message.starts_with("YAML Parse Error:"));
        assert!(message.contains("line 1 column 10"));
    }
}
