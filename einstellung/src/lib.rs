use serde::de::DeserializeOwned;
use std::{
    fs::File,
    io::{BufReader, Cursor, Read},
    path::{Path, PathBuf},
};
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

pub trait ReaderFactory: Send + Sync {
    fn get_reader(&self) -> Result<Box<dyn Read + '_>, ConfigError>;

    fn clone_dyn(&self) -> Box<dyn ReaderFactory + 'static> {
        panic!(
            "this reader factory is cloneable. Implement your own `ReaderFactory` for a custom cloneable FileProvider"
        )
    }
}

// Blanket implementation for closures that return 'static readers
impl<F> ReaderFactory for F
where
    F: Fn() -> Result<Box<dyn Read + 'static>, ConfigError> + Send + Sync,
{
    fn get_reader(&self) -> Result<Box<dyn Read + '_>, ConfigError> {
        self().map(|r| r as Box<dyn Read + '_>)
    }
}

pub enum FileContentProvider<'i> {
    InlineBorrowed(&'i str),
    InlineOwned(String),

    PathBorrowed(&'i Path),
    PathOwned(PathBuf),

    CustomFn(fn() -> Result<Box<dyn Read + 'static>, ConfigError>),

    CustomBoxed(Box<dyn ReaderFactory + 'static>),
    CustomRef(&'i dyn ReaderFactory),
}

impl<'i> FileContentProvider<'i> {
    /// Internal: call the provider to produce a reader
    fn with_reader<R>(
        &self,
        f: impl FnOnce(&mut dyn Read) -> Result<R, ConfigError>,
    ) -> Result<R, ConfigError> {
        match self {
            FileContentProvider::InlineBorrowed(s) => f(&mut Cursor::new(*s)),
            FileContentProvider::InlineOwned(s) => f(&mut Cursor::new(s.as_str())),
            FileContentProvider::PathBorrowed(p) => f(&mut BufReader::new(File::open(p)?)),
            FileContentProvider::PathOwned(p) => f(&mut BufReader::new(File::open(p)?)),
            FileContentProvider::CustomBoxed(factory) => f(factory.get_reader()?.as_mut()),
            FileContentProvider::CustomRef(factory) => f(factory.get_reader()?.as_mut()),
            FileContentProvider::CustomFn(func) => f(func()?.as_mut()),
        }
    }

    /// Convert to `'static` owned data.
    /// Only valid for Inline/Path variants.
    pub fn into_owned(self) -> FileContentProvider<'static> {
        use FileContentProvider::*;
        match self {
            InlineBorrowed(s) => InlineOwned(s.to_owned()),
            PathBorrowed(p) => PathOwned(p.to_path_buf()),
            InlineOwned(s) => InlineOwned(s),
            PathOwned(p) => PathOwned(p),
            CustomFn(f) => CustomFn(f),
            CustomBoxed(f) => CustomBoxed(f),
            CustomRef(f) => CustomBoxed(f.clone_dyn()),
        }
    }

    pub fn as_borrowed<'s>(&'s self) -> FileContentProvider<'s> {
        use FileContentProvider::*;
        match self {
            InlineOwned(s) => InlineBorrowed(s.as_str()),
            PathOwned(p) => PathBorrowed(p.as_path()),
            InlineBorrowed(s) => InlineBorrowed(s),
            PathBorrowed(p) => PathBorrowed(p),
            CustomFn(f) => CustomFn(*f),
            CustomBoxed(f) => CustomRef(&**f),
            CustomRef(f) => CustomRef(*f),
        }
    }
}

pub trait IntoFileContentProvider<'i> {
    fn into_provider(self) -> FileContentProvider<'i>;

    fn into_owned_provider(self) -> FileContentProvider<'static>
    where
        Self: Sized,
    {
        self.into_provider().into_owned()
    }
}

impl<'i> IntoFileContentProvider<'i> for FileContentProvider<'i> {
    fn into_provider(self) -> FileContentProvider<'i> {
        self
    }
}

impl<'i> IntoFileContentProvider<'i> for &'i str {
    fn into_provider(self) -> FileContentProvider<'i> {
        FileContentProvider::InlineBorrowed(self)
    }
}
impl IntoFileContentProvider<'static> for String {
    fn into_provider(self) -> FileContentProvider<'static> {
        FileContentProvider::InlineOwned(self)
    }
}

impl<'i> IntoFileContentProvider<'i> for &'i Path {
    fn into_provider(self) -> FileContentProvider<'i> {
        FileContentProvider::PathBorrowed(self)
    }
}
impl IntoFileContentProvider<'static> for PathBuf {
    fn into_provider(self) -> FileContentProvider<'static> {
        FileContentProvider::PathOwned(self)
    }
}

impl IntoFileContentProvider<'static> for fn() -> Result<Box<dyn Read + 'static>, ConfigError> {
    fn into_provider(self) -> FileContentProvider<'static> {
        FileContentProvider::CustomFn(self)
    }
}

impl<F> IntoFileContentProvider<'static> for Box<F>
where
    F: ReaderFactory + 'static,
{
    fn into_provider(self) -> FileContentProvider<'static> {
        FileContentProvider::CustomBoxed(self)
    }
}

// Bridge for borrowing closures
impl<'i, F> IntoFileContentProvider<'i> for &'i F
where
    F: ReaderFactory + 'i,
{
    fn into_provider(self) -> FileContentProvider<'i> {
        FileContentProvider::CustomRef(self)
    }
}

#[cfg(feature = "json")]
pub mod json {
    use super::*;

    pub struct JsonFileProvider<'i>(FileContentProvider<'i>);

    impl<'i> JsonFileProvider<'i> {
        /// Generic constructor via IntoFileContentProvider
        pub fn new(src: impl IntoFileContentProvider<'i>) -> Self {
            Self(src.into_provider())
        }

        /// Upgrade to owned (`'static`) for Inline/Path variants
        pub fn into_owned(self) -> JsonFileProvider<'static> {
            JsonFileProvider(self.0.into_owned())
        }
    }

    impl<'i> super::ConfigProvider for JsonFileProvider<'i> {
        fn load_partial<T: serde::de::DeserializeOwned>(&self) -> Result<T, ConfigError> {
            self.0
                .with_reader(|reader| Ok(serde_json::from_reader(reader)?))
        }
    }
}
