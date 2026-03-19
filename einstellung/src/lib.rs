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

/// Describes a Configuration which can be built from its associated `::Partial` configuration.
/// The Partial type contains optional variants of all fields of the Config.
pub trait Config: Sized {
    /// The Partial Configuration for this type. Contains all fields of this type, made optional.
    /// See the documentation for [`PartialConfig`] for merging / building partial configs.
    type Partial: PartialConfig<Complete = Self>;

    /// Load this config as a [`PartialConfig`] for merging with other partial configs.
    fn load_partial(provider: &impl ConfigProvider) -> Result<Self::Partial, ConfigError> {
        provider.load_partial::<Self::Partial>()
    }

    /// Load this config in its complete form.
    fn load_complete(provider: &impl ConfigProvider) -> Result<Self, ConfigError> {
        Self::load_partial(provider)?.build()
    }
}

/// A Partial variant of a [`trait@Config`]. This means that every field is optional
/// allowing incremental merging of configs.
pub trait PartialConfig: Default + DeserializeOwned {
    /// The associated Complete Config
    type Complete: Config;

    /// Merge two partial configs.
    /// See the derive macro for [`derive@Config`] for how to define merging stategies
    fn merge(self, next: Self) -> Result<Self, ConfigError>;

    /// Build this partial config into its complete form. All required fields need to be present for this to succeed.
    /// See the derive macro for [`derive@Config`] for how to define validation stategies and field contents.
    fn build(self) -> Result<Self::Complete, ConfigError>;
}

/// Indicates that parts of this type can be "frozen".
/// This means that these parts can not be overwriten by merges in any way.
/// See the derive macro for [`derive@Config`] for how to mark fields as [`trait@Freezable`]
pub trait Freezable {
    /// Freeze the freezable parts of this type
    fn freeze(self) -> Self;

    /// Check if any parts of this type are frozen
    fn is_frozen(&self) -> bool;
}

/// Generic provider for loading a partial configuration.
/// This can be any type which can produce a `T: DeserializeOwned`
/// See the `json`, `yaml` and `toml` features and the associated
/// [`JsonFileProvider`], [`YamlFileProvider`] and [`TomlFileProvider`] types for the built-in implementations.
/// The [`FileContentProvider`] provides an ergonomic interface to specifiy the location/data of an input file.
pub trait ConfigProvider {
    /// Load a [`PartialConfig`] (or really an deserializeable type) from this provider
    fn load_partial<T: DeserializeOwned>(&self) -> Result<T, ConfigError>;
}

/// This types indicates the location an error occured.
#[derive(Debug)]
pub struct FieldPath {
    /// Name of the [`PartialConfig`] type on which failing method was called.
    pub base_type: &'static str,
    /// Field which produced the error
    pub field: &'static str,
    /// Subconfig fields leading from the `base_type` to `field`
    pub path: Vec<&'static str>,
}

impl FieldPath {
    /// Produce a path pointing to the error location without any outer context
    pub fn new(base_type: &'static str, field: &'static str) -> Self {
        Self {
            base_type,
            field,
            path: Vec::new(),
        }
    }
    /// Push a field as context for this path. This means that the error passed through `complete::field`
    pub fn context(mut self, complete: &'static str, field: &'static str) -> Self {
        self.base_type = complete;
        self.path.push(field);
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

/// Possible errors which can be produces when loading a config
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

/// A function passed to `#[config(validate ... )]` needs to match this signature. See the derive macro for [`derive@Config`] for more details on `validate`.
pub type ValidationFunction<T, E> = for<'a> fn(&'a T) -> Result<(), E>;

/// A function passed to `#[config(merge ... )]` needs to match this signature. See the derive macro for [`derive@Config`] for more details on `merge`.
pub type MergeFunction<T, E> = fn(T, T) -> Result<T, E>;

/// Wraps a type to make it [`trait@Freezable`].
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

/// Interaction of two [`Freezable`] types
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
