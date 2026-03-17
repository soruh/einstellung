use serde::de::DeserializeOwned;

use thiserror::Error;

#[cfg(feature = "derive")]
pub use einstellung_derive::Config;

#[doc(hidden)]
pub use serde;

#[cfg(test)]
pub mod tests;

mod file_provider;

#[cfg(feature = "json")]
pub mod json;

#[cfg(feature = "yaml")]
pub mod yaml;

#[cfg(feature = "toml")]
pub mod toml;

pub trait Config: Sized {
    type Partial: PartialConfig<Complete = Self>;

    fn load_partial(provider: &impl ConfigProvider) -> Result<Self::Partial, ConfigError> {
        provider.load_partial::<Self::Partial>()
    }

    fn load_complete(provider: &impl ConfigProvider) -> Result<Self, ConfigError> {
        Self::load_partial(provider)?.build()
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

#[derive(Error, Debug)]
#[non_exhaustive]
pub enum ConfigError {
    #[error("IO Error: {0}")]
    Io(#[from] std::io::Error),

    #[cfg(feature = "json")]
    #[error("JSON Parse Error: {0}")]
    Json(#[from] serde_json::Error),

    #[cfg(feature = "yaml")]
    #[error("YAML Parse Error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    #[cfg(feature = "toml")]
    #[error("TOML Parse Error: {0}")]
    Toml(#[from] ::toml::de::Error),

    #[error("Missing required configuration field: '{0}'")]
    MissingField(&'static str),

    #[error("Validation failed for field '{field}': {reason}")]
    Validation {
        field: &'static str,
        reason: Box<dyn std::error::Error>,
    },
}
