use crate::{ConfigProvider, FileContentProvider, IntoFileContentProvider};

use super::*;

pub struct JsonFileProvider<'i>(pub FileContentProvider<'i>);

impl<'i> JsonFileProvider<'i> {
    pub fn new(src: impl IntoFileContentProvider<'i>) -> Self {
        Self(src.into_provider())
    }

    pub fn into_owned(self) -> JsonFileProvider<'static> {
        JsonFileProvider(self.0.into_owned())
    }
}

impl<'i> ConfigProvider for JsonFileProvider<'i> {
    fn load_partial<T: serde::de::DeserializeOwned>(&self) -> Result<T, ConfigError> {
        self.0
            .with_reader(|reader| Ok(serde_json::from_reader(reader)?))
    }
}
