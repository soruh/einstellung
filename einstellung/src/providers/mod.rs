use std::{
    fs::File,
    io::{BufReader, Cursor, Read},
    path::{Path, PathBuf},
};

#[cfg(feature = "dotenv")]
mod dotenv;
#[cfg(feature = "env")]
mod env;
#[cfg(any(feature = "json", feature = "toml", feature = "yaml"))]
mod format;
#[cfg(feature = "json")]
mod json;
#[cfg(feature = "key-value")]
mod key_value;
#[cfg(feature = "toml")]
mod toml;
#[cfg(feature = "yaml")]
mod yaml;

#[cfg(feature = "dotenv")]
pub use dotenv::DotenvProvider;
#[cfg(feature = "env")]
pub use env::{EnvProvider, EnvProviderError};
#[cfg(any(feature = "json", feature = "toml", feature = "yaml"))]
pub use format::{ConfigFormat, FormatProvider, FormatProviderError};
#[cfg(feature = "json")]
pub use json::JsonFileProvider;
#[cfg(feature = "key-value")]
pub use key_value::{KeyValueProvider, KeyValueProviderError};
#[cfg(feature = "toml")]
pub use toml::TomlFileProvider;
#[cfg(feature = "yaml")]
pub use yaml::YamlFileProvider;

use crate::ConfigError;

#[cfg(any(feature = "json", feature = "toml", feature = "yaml"))]
fn with_deserialization_path(error: ConfigError, track: serde_path_to_error::Track) -> ConfigError {
    // Use dotted segments so collection failures can find their parent field's provenance.
    // Unknown segments only identify the enclosing value, not a destination of their own.
    let mut segments = Vec::new();
    for segment in track.path().iter() {
        match segment {
            serde_path_to_error::Segment::Map { key } => segments.push(key.clone()),
            serde_path_to_error::Segment::Seq { index } => segments.push(index.to_string()),
            serde_path_to_error::Segment::Enum { variant } => segments.push(variant.clone()),
            serde_path_to_error::Segment::Unknown => break,
        }
    }
    if segments.is_empty() {
        error
    } else {
        error.with_logical_path(segments.join("."))
    }
}

/// Helper trait to read from type-erased file sources
pub trait ReaderFactory: Send + Sync {
    /// Produce a reader from this source
    fn get_reader(&self) -> Result<Box<dyn Read + '_>, ConfigError>;

    /// Clone this source in a dyn-compatible way
    fn clone_dyn(&self) -> Result<Box<dyn ReaderFactory + 'static>, ConfigError> {
        Err(ConfigError::provider(
            "reader factory",
            std::io::Error::other(
                "this reader factory is not cloneable; implement `ReaderFactory::clone_dyn` to support owned conversion",
            ),
        ))
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

/// A generic source for file contents which can be used as a reference or an owned type.
/// See [`FileContentProvider::into_owned`] and [`FileContentProvider::as_borrowed`] for conversion methods.
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
    /// call the provider to produce a reader
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

    /// Describe this source for diagnostics without exposing inline contents.
    #[cfg(any(
        feature = "json",
        feature = "toml",
        feature = "yaml",
        feature = "dotenv"
    ))]
    pub(crate) fn source_label(&self, provider: &str) -> String {
        use FileContentProvider::*;

        match self {
            InlineBorrowed(_) | InlineOwned(_) => format!("inline {provider}"),
            PathBorrowed(path) => format!("{provider} file {}", path.display()),
            PathOwned(path) => format!("{provider} file {}", path.display()),
            CustomFn(_) | CustomBoxed(_) | CustomRef(_) => format!("custom {provider} source"),
        }
    }

    /// Convert to `'static` owned data.
    pub fn into_owned(self) -> Result<FileContentProvider<'static>, ConfigError> {
        use FileContentProvider::*;
        Ok(match self {
            InlineBorrowed(s) => InlineOwned(s.to_owned()),
            PathBorrowed(p) => PathOwned(p.to_path_buf()),
            InlineOwned(s) => InlineOwned(s),
            PathOwned(p) => PathOwned(p),
            CustomFn(f) => CustomFn(f),
            CustomBoxed(f) => CustomBoxed(f),
            CustomRef(f) => CustomBoxed(f.clone_dyn()?),
        })
    }

    /// Get a reference to the provider
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

/// Any type from which file contents can be read (see [`FileContentProvider`])
pub trait IntoFileContentProvider<'i> {
    /// Open this provider
    fn into_provider(self) -> FileContentProvider<'i>;

    /// Open this provider and immediately make it static
    fn into_owned_provider(self) -> Result<FileContentProvider<'static>, ConfigError>
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

impl IntoFileContentProvider<'static> for Box<dyn ReaderFactory + 'static> {
    fn into_provider(self) -> FileContentProvider<'static> {
        FileContentProvider::CustomBoxed(self)
    }
}

// Bridge for borrowing closures and concrete reader factories.
impl<'i, F> IntoFileContentProvider<'i> for &'i F
where
    F: ReaderFactory + 'i,
{
    fn into_provider(self) -> FileContentProvider<'i> {
        FileContentProvider::CustomRef(self)
    }
}

impl<'i> IntoFileContentProvider<'i> for &'i dyn ReaderFactory {
    fn into_provider(self) -> FileContentProvider<'i> {
        FileContentProvider::CustomRef(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct BorrowedFactory;

    impl ReaderFactory for BorrowedFactory {
        fn get_reader(&self) -> Result<Box<dyn Read + '_>, ConfigError> {
            Ok(Box::new(Cursor::new("name = \"test\"")))
        }
    }

    #[test]
    fn borrowed_custom_reader_requires_explicit_clone_support() {
        let factory = BorrowedFactory;
        let provider = FileContentProvider::CustomRef(&factory);

        let error = provider.into_owned().err().expect("conversion should fail");
        assert!(error.to_string().contains("not cloneable"), "{error}");
    }

    #[derive(Clone)]
    struct CloneableFactory;

    impl ReaderFactory for CloneableFactory {
        fn get_reader(&self) -> Result<Box<dyn Read + '_>, ConfigError> {
            Ok(Box::new(Cursor::new("name = \"test\"")))
        }

        fn clone_dyn(&self) -> Result<Box<dyn ReaderFactory + 'static>, ConfigError> {
            Ok(Box::new(self.clone()))
        }
    }

    #[test]
    fn borrowed_cloneable_reader_can_be_owned() {
        let factory = CloneableFactory;
        let provider = FileContentProvider::CustomRef(&factory);
        let owned = provider.into_owned().expect("conversion should succeed");

        let contents = owned
            .with_reader(|reader| {
                let mut contents = String::new();
                reader.read_to_string(&mut contents)?;
                Ok(contents)
            })
            .expect("reader should work");
        assert_eq!(contents, "name = \"test\"");
    }

    #[test]
    fn erased_reader_factories_convert_through_normal_source_api() {
        let borrowed_factory = CloneableFactory;
        let borrowed: &dyn ReaderFactory = &borrowed_factory;
        let borrowed_provider = borrowed.into_provider();
        let borrowed_contents = borrowed_provider
            .with_reader(|reader| {
                let mut contents = String::new();
                reader.read_to_string(&mut contents)?;
                Ok(contents)
            })
            .expect("borrowed reader should work");
        assert_eq!(borrowed_contents, "name = \"test\"");

        let owned: Box<dyn ReaderFactory> = Box::new(CloneableFactory);
        let owned_provider = owned.into_provider();
        let owned_contents = owned_provider
            .with_reader(|reader| {
                let mut contents = String::new();
                reader.read_to_string(&mut contents)?;
                Ok(contents)
            })
            .expect("owned reader should work");
        assert_eq!(owned_contents, "name = \"test\"");
    }
}
