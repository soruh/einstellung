use crate::{ConfigProvider, FileContentProvider, IntoFileContentProvider};

use super::*;

/// [`ConfigProvider`] which interperts the file contents as JSON
pub struct JsonFileProvider<'i>(pub FileContentProvider<'i>);

impl<'i> JsonFileProvider<'i> {
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

    pub fn into_owned(self) -> Result<JsonFileProvider<'static>, ConfigError> {
        Ok(JsonFileProvider(self.0.into_owned()?))
    }
}

impl JsonFileProvider<'static> {
    /// Build an owned provider from inline configuration text.
    pub fn from_owned_contents(src: String) -> Self {
        Self(FileContentProvider::InlineOwned(src))
    }

    /// Build an owned provider from a filesystem path.
    pub fn from_path_buf(path: std::path::PathBuf) -> Self {
        Self(FileContentProvider::PathOwned(path))
    }
}

impl<'i> ConfigProvider for JsonFileProvider<'i> {
    fn load_partial<T: serde::de::DeserializeOwned>(&self) -> Result<T, ConfigError> {
        self.0
            .with_reader(|reader| Ok(serde_json::from_reader(reader)?))
    }

    fn source(&self) -> crate::ConfigSource {
        crate::ConfigSource::new(self.0.source_label("json"))
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    struct Config {
        retries: u16,
    }

    #[test]
    fn data_errors_do_not_render_offending_values() {
        let err = JsonFileProvider::from_contents(r#"{ "retries": "super-secret" }"#)
            .load_partial::<Config>()
            .unwrap_err();
        let display = err.to_string();
        let debug = format!("{err:?}");

        assert!(!display.contains("super-secret"), "{display}");
        assert!(!debug.contains("super-secret"), "{debug}");
        assert!(display.contains("data error"), "{display}");
    }
}
