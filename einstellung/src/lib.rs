use serde::de::DeserializeOwned;
use std::{
    borrow::Cow,
    fs::File,
    io::{BufReader, Read},
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

pub enum FileContentProvider<'i> {
    Inline(Cow<'i, str>),
    File(Cow<'i, PathBuf>),
    Custom(Box<dyn Fn() -> Result<Box<dyn Read + 'i>, ConfigError> + 'i>),
}

pub enum FileContentReader<'i> {
    Inline(std::io::Cursor<&'i str>),
    File(BufReader<File>),
    Generic(Box<dyn Read + 'i>),
}

impl<'i> FileContentProvider<'i> {
    pub fn open(&self) -> Result<FileContentReader<'i>, ConfigError> {
        Ok(match self {
            FileContentProvider::Inline(inline) => {
                FileContentReader::Inline(std::io::Cursor::new(inline))
            }
            FileContentProvider::File(path) => {
                FileContentReader::File(File::open(&**path).map(BufReader::new)?)
            }
            FileContentProvider::Custom(producer) => FileContentReader::Generic(producer()?),
        })
    }
}

#[cfg(feature = "json")]
pub mod json {
    use super::*;

    pub struct JsonFileProvider<'i>(FileContentProvider<'i>);

    impl<'i> JsonFileProvider<'i> {
        pub fn inline(content: impl Into<Cow<'i, str>>) -> Self {
            Self(FileContentProvider::Inline(content.into()))
        }
        pub fn path(path: impl Into<Cow<'i, PathBuf>>) -> Self {
            Self(FileContentProvider::File(path.into()))
        }
    }

    impl<'i> ConfigProvider for JsonFileProvider<'i> {
        fn load_partial<T: DeserializeOwned>(&self) -> Result<T, ConfigError> {
            match self.0.open()? {
                FileContentReader::Inline(cursor) => Ok(serde_json::from_reader(cursor)?),
                FileContentReader::File(buf_reader) => Ok(serde_json::from_reader(buf_reader)?),
                FileContentReader::Generic(reader) => Ok(serde_json::from_reader(reader)?),
            }
        }
    }
}
