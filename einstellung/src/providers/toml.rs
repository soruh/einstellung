use super::*;
use crate::{ConfigProvider, FileContentProvider, IntoFileContentProvider};

/// [`ConfigProvider`] which interperts the file contents as TOML
pub struct TomlFileProvider<'i>(pub FileContentProvider<'i>);

impl<'i> TomlFileProvider<'i> {
    pub fn new(src: impl IntoFileContentProvider<'i>) -> Self {
        Self(src.into_provider())
    }

    pub fn into_owned(self) -> TomlFileProvider<'static> {
        TomlFileProvider(self.0.into_owned())
    }
}

impl<'i> ConfigProvider for TomlFileProvider<'i> {
    fn load_partial<T: serde::de::DeserializeOwned>(&self) -> Result<T, ConfigError> {
        self.0.with_reader(|reader| {
            let mut buffer = String::new();
            reader.read_to_string(&mut buffer)?;
            Ok(::toml::from_str(&buffer)?)
        })
    }
}
