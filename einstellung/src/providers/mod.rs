use std::{
    fs::File,
    io::{BufReader, Cursor, Read},
    path::{Path, PathBuf},
};

#[cfg(feature = "json")]
mod json;
#[cfg(feature = "toml")]
mod toml;
#[cfg(feature = "yaml")]
mod yaml;

#[cfg(feature = "json")]
pub use json::JsonFileProvider;
#[cfg(feature = "toml")]
pub use toml::TomlFileProvider;
#[cfg(feature = "yaml")]
pub use yaml::YamlFileProvider;

use crate::ConfigError;

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

#[non_exhaustive]
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
    pub fn with_reader<R>(
        &self,
        f: impl FnOnce(&mut dyn Read) -> Result<R, ConfigError>,
    ) -> Result<R, ConfigError> {
        use FileContentProvider::*;

        match self {
            InlineBorrowed(s) => f(&mut Cursor::new(*s)),
            InlineOwned(s) => f(&mut Cursor::new(s.as_str())),
            PathBorrowed(p) => f(&mut BufReader::new(File::open(p)?)),
            PathOwned(p) => f(&mut BufReader::new(File::open(p)?)),
            CustomBoxed(factory) => f(factory.get_reader()?.as_mut()),
            CustomRef(factory) => f(factory.get_reader()?.as_mut()),
            CustomFn(func) => f(func()?.as_mut()),
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
