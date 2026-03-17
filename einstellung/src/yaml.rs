use super::*;
use crate::file_provider::{FileContentProvider, IntoFileContentProvider};

pub struct YamlFileProvider<'i>(FileContentProvider<'i>);

impl<'i> YamlFileProvider<'i> {
    pub fn new(src: impl IntoFileContentProvider<'i>) -> Self {
        Self(src.into_provider())
    }

    pub fn into_owned(self) -> YamlFileProvider<'static> {
        YamlFileProvider(self.0.into_owned())
    }
}

impl<'i> super::ConfigProvider for YamlFileProvider<'i> {
    fn load_partial<T: serde::de::DeserializeOwned>(&self) -> Result<T, ConfigError> {
        self.0.with_reader(|reader| {
            // serde_yaml handles Readers directly
            Ok(serde_yaml::from_reader(reader)?)
        })
    }
}
