use std::fmt::Display;

use serde::de::DeserializeOwned;

use thiserror::Error;

#[cfg(feature = "derive")]
pub use einstellung_derive::Config;

#[doc(hidden)]
pub use serde;

#[cfg(test)]
pub mod tests;

mod providers;

pub use providers::*;

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

#[derive(Debug)]
pub struct FieldPath {
    base_type: &'static str,
    field: &'static str,
    path: Vec<&'static str>,
}

impl FieldPath {
    pub fn new(base_type: &'static str, field: &'static str) -> Self {
        Self {
            base_type,
            field,
            path: Vec::new(),
        }
    }
    pub fn context(mut self, complete: &'static str, segment: &'static str) -> Self {
        self.base_type = complete;
        self.path.push(segment);
        self
    }
}

#[doc(hidden)]
pub fn build_with_context<P: PartialConfig>(
    partial: P,
    complete: &'static str,
    segment: &'static str,
) -> Result<P::Complete, ConfigError> {
    partial
        .build()
        .map_err(|err| context(err, complete, segment))
}

fn context(error: ConfigError, complete: &'static str, segment: &'static str) -> ConfigError {
    match error {
        ConfigError::MissingField(field) => {
            ConfigError::MissingField(field.context(complete, segment))
        }
        ConfigError::Validation { field, reason } => ConfigError::Validation {
            field: field.context(complete, segment),
            reason,
        },
        x => x,
    }
}

impl Display for FieldPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}::", self.base_type)?;
        for item in self.path.iter().rev() {
            write!(f, "{item}::")?;
        }
        write!(f, "{}", self.field)
    }
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
    MissingField(FieldPath),

    #[error("Validation failed for field '{field}': {reason}")]
    Validation {
        field: FieldPath,
        reason: Box<dyn std::error::Error>,
    },
}

pub type ValidationFunction<T> =
    for<'a> fn(&'a T) -> Result<(), Box<dyn ::core::error::Error + 'static>>;
