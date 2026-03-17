use serde::de::DeserializeOwned;
use std::path::Path;
use thiserror::Error;

#[cfg(feature = "derive")]
pub use einstellung_derive::Config;

#[doc(hidden)]
pub use serde;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("IO Error: {0}")]
    Io(#[from] std::io::Error),

    #[cfg(feature = "json")]
    #[error("JSON Parse Error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Missing required configuration field: '{0}'")]
    MissingField(&'static str),

    #[error("Validation failed for field '{field}': {reason}")]
    Validation {
        field: &'static str,
        reason: Box<dyn std::error::Error>,
    },
}

pub trait Config: Sized {
    type Partial: PartialConfig<Complete = Self>;

    fn load_from(provider: &impl ConfigProvider) -> Result<Self, ConfigError> {
        provider.load_partial::<Self::Partial>()?.build()
    }
}

pub trait PartialConfig: Default + DeserializeOwned {
    type Complete;

    fn merge(self, next: Self) -> Self;
    fn build(self) -> Result<Self::Complete, ConfigError>;
}

pub trait ConfigProvider {
    fn load_partial<T: DeserializeOwned>(&self) -> Result<T, ConfigError>;
}

#[cfg(feature = "json")]
pub mod json {
    use super::*;
    use std::fs::File;
    use std::io::BufReader;

    pub struct JsonFileProvider<'a>(pub &'a Path);

    impl<'a> ConfigProvider for JsonFileProvider<'a> {
        fn load_partial<T: DeserializeOwned>(&self) -> Result<T, ConfigError> {
            Ok(serde_json::from_reader(BufReader::new(File::open(
                self.0,
            )?))?)
        }
    }
}
