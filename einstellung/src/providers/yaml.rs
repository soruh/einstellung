use super::*;
use crate::{ConfigProvider, FileContentProvider, IntoFileContentProvider};

pub struct YamlFileProvider<'i>(pub FileContentProvider<'i>);

impl<'i> YamlFileProvider<'i> {
    pub fn new(src: impl IntoFileContentProvider<'i>) -> Self {
        Self(src.into_provider())
    }

    pub fn into_owned(self) -> YamlFileProvider<'static> {
        YamlFileProvider(self.0.into_owned())
    }
}

impl<'i> ConfigProvider for YamlFileProvider<'i> {
    fn load_partial<T: serde::de::DeserializeOwned>(&self) -> Result<T, ConfigError> {
        self.0
            .with_reader(|reader| Ok(serde_yaml::from_reader(reader)?))
    }
}
