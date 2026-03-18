use std::fmt::Display;

use serde::{Deserialize, Serialize, de::DeserializeOwned};

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

    fn merge(self, next: Self) -> Result<Self, ConfigError>;
    fn build(self) -> Result<Self::Complete, ConfigError>;
}

pub trait Freezable {
    fn freeze(self) -> Self;
    fn is_frozen(&self) -> bool;
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
        ConfigError::CustomMerge { field, reason } => ConfigError::CustomMerge {
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

    #[error("Attempted to merge two frozen fields: '{0}'")]
    FreezeCollision(FieldPath),

    #[error("Validation failed for field '{field}': {reason}")]
    Validation {
        field: FieldPath,
        reason: Box<dyn std::error::Error>,
    },

    #[error("Custom Merge failed for field '{field}': {reason}")]
    CustomMerge {
        field: FieldPath,
        reason: Box<dyn std::error::Error>,
    },
}

pub type ValidationFunction<T, E> = for<'a> fn(&'a T) -> Result<(), E>;
pub type MergeFunction<T, E> = fn(T, T) -> Result<T, E>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Freeze<T> {
    Free(T),
    Frozen(T),
}

impl<T: Serialize> Serialize for Freeze<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Freeze::Free(x) => T::serialize(x, serializer),
            Freeze::Frozen(x) => T::serialize(x, serializer),
        }
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Freeze<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(Self::Free(T::deserialize(deserializer)?))
    }
}

impl<T> Default for Freeze<T>
where
    T: Default,
{
    fn default() -> Self {
        Self::Free(T::default())
    }
}

pub enum FreezeCombination<T> {
    BothFree(T, T),
    OneFrozen(T),
    BothFrozen,
}

impl<T: Freezable> FreezeCombination<T> {
    pub fn of(a: T, b: T) -> FreezeCombination<T> {
        match (a.is_frozen(), b.is_frozen()) {
            (false, false) => FreezeCombination::BothFree(a, b),
            (true, false) => FreezeCombination::OneFrozen(a),
            (false, true) => FreezeCombination::OneFrozen(b),
            (true, true) => FreezeCombination::BothFrozen,
        }
    }
}

impl<T> FreezeCombination<T> {
    pub fn of_freeze(a: Freeze<T>, b: Freeze<T>) -> FreezeCombination<T> {
        match (a, b) {
            (Freeze::Free(a), Freeze::Free(b)) => FreezeCombination::BothFree(a, b),
            (Freeze::Frozen(a), Freeze::Free(_)) => FreezeCombination::OneFrozen(a),
            (Freeze::Free(_), Freeze::Frozen(b)) => FreezeCombination::OneFrozen(b),
            (Freeze::Frozen(_), Freeze::Frozen(_)) => FreezeCombination::BothFrozen,
        }
    }
}

impl<T> Freeze<T> {
    pub fn into_inner(self) -> T {
        match self {
            Freeze::Free(x) => x,
            Freeze::Frozen(x) => x,
        }
    }
}

impl<T> Freezable for Freeze<T> {
    fn freeze(self) -> Self {
        let (Freeze::Frozen(value) | Freeze::Free(value)) = self;
        Freeze::Frozen(value)
    }
    fn is_frozen(&self) -> bool {
        matches!(self, Freeze::Frozen(_))
    }
}
