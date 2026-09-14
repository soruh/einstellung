use std::{collections::BTreeMap, fmt};

use serde::de::{self, DeserializeOwned, DeserializeSeed, MapAccess, Visitor};
use thiserror::Error;

use crate::{ConfigError, ConfigProvider, ConfigSource};

/// Errors produced while decoding dotted-path string configuration values.
#[derive(Debug, Error)]
pub enum KeyValueProviderError {
    #[error("configuration path {path:?} conflicts with another mapped value")]
    ConflictingPath { path: String },

    #[error("invalid value for configuration input {input:?}: {message}")]
    InvalidValue { input: String, message: String },
}

#[derive(Debug)]
pub(super) struct MappedValueError {
    path: String,
    source: KeyValueProviderError,
}

impl MappedValueError {
    fn new(path: impl Into<String>, source: KeyValueProviderError) -> Self {
        Self {
            path: path.into(),
            source,
        }
    }

    fn with_path_if_unknown(mut self, path: impl Into<String>) -> Self {
        if self.path == "<key/value>" {
            self.path = path.into();
        }
        self
    }

    pub(super) fn into_parts(self) -> (String, KeyValueProviderError) {
        (self.path, self.source)
    }
}

impl fmt::Display for MappedValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.source.fmt(f)
    }
}

impl std::error::Error for MappedValueError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

impl de::Error for MappedValueError {
    fn custom<T>(_message: T) -> Self
    where
        T: fmt::Display,
    {
        Self::new(
            "<key/value>",
            KeyValueProviderError::InvalidValue {
                input: "<key/value>".to_owned(),
                message: "value does not match target type".to_owned(),
            },
        )
    }
}

/// Loads dotted-path string values into a partial configuration.
///
/// This is useful for CLI overrides, secret-store adapters, and other external key/value
/// sources. Scalar fields parse from their string representation, while sequences and maps use
/// JSON syntax. Later duplicate paths replace earlier ones.
#[derive(Clone)]
pub struct KeyValueProvider {
    source: ConfigSource,
    values: Vec<KeyValueBinding>,
}

#[derive(Clone, Debug)]
struct KeyValueBinding {
    path: String,
    value: String,
}

impl fmt::Debug for KeyValueProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let paths = self
            .values
            .iter()
            .map(|binding| binding.path.as_str())
            .collect::<Vec<_>>();
        f.debug_struct("KeyValueProvider")
            .field("source", &self.source)
            .field("paths", &paths)
            .finish()
    }
}

impl Default for KeyValueProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyValueProvider {
    /// Create an empty provider with a generic provenance label.
    pub fn new() -> Self {
        Self::named("key/value overrides")
    }

    /// Create an empty provider with a custom provenance label.
    ///
    /// The label should identify the source without including secret values.
    pub fn named(source: impl Into<String>) -> Self {
        Self {
            source: ConfigSource::new(source),
            values: Vec::new(),
        }
    }

    /// Add a dotted configuration path and its string value.
    pub fn with(mut self, path: impl Into<String>, value: impl Into<String>) -> Self {
        self.values.push(KeyValueBinding {
            path: path.into(),
            value: value.into(),
        });
        self
    }

    /// Add multiple dotted configuration paths and string values.
    pub fn with_pairs<I, K, V>(mut self, values: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        self.values
            .extend(values.into_iter().map(|(path, value)| KeyValueBinding {
                path: path.into(),
                value: value.into(),
            }));
        self
    }
}

impl ConfigProvider for KeyValueProvider {
    fn load_partial<T: DeserializeOwned>(&self) -> Result<T, ConfigError> {
        load_mapped_values(self.values.iter().map(|binding| {
            MappedValue::new(
                binding.path.split('.').map(str::to_owned).collect(),
                binding.path.clone(),
                binding.value.clone(),
            )
        }))
        .map_err(|error| {
            let (path, source) = error.into_parts();
            ConfigError::provider_at("key/value", path, source)
        })
    }

    fn source(&self) -> ConfigSource {
        self.source.clone()
    }
}

#[derive(Clone, Debug)]
pub(super) struct MappedValue {
    path: Vec<String>,
    input: String,
    value: String,
}

impl MappedValue {
    pub(super) fn new(path: Vec<String>, input: String, value: String) -> Self {
        Self { path, input, value }
    }
}

#[derive(Debug)]
enum ValueNode {
    Branch {
        path: String,
        children: BTreeMap<String, ValueNode>,
    },
    Leaf {
        path: String,
        input: String,
        value: String,
    },
}

impl ValueNode {
    fn path(&self) -> &str {
        match self {
            Self::Branch { path, .. } | Self::Leaf { path, .. } => path,
        }
    }
}

pub(super) fn load_mapped_values<T>(
    values: impl IntoIterator<Item = MappedValue>,
) -> Result<T, MappedValueError>
where
    T: DeserializeOwned,
{
    let mut root = ValueNode::Branch {
        path: String::new(),
        children: BTreeMap::new(),
    };
    for value in values {
        insert_mapped_value(&mut root, value)?;
    }

    T::deserialize(ValueNodeDeserializer::new(&root))
}

fn insert_mapped_value(root: &mut ValueNode, mapped: MappedValue) -> Result<(), MappedValueError> {
    let MappedValue { path, input, value } = mapped;
    let logical_path = path.join(".");
    let conflict = || {
        MappedValueError::new(
            logical_path.clone(),
            KeyValueProviderError::ConflictingPath {
                path: logical_path.clone(),
            },
        )
    };

    if path.is_empty() || path.iter().any(String::is_empty) {
        return Err(conflict());
    }

    let mut node = root;
    for (index, segment) in path.iter().enumerate() {
        let is_leaf = index + 1 == path.len();
        let ValueNode::Branch { children, .. } = node else {
            return Err(conflict());
        };

        if is_leaf {
            match children.get(segment) {
                Some(ValueNode::Branch { .. }) => return Err(conflict()),
                Some(ValueNode::Leaf { .. }) | None => {
                    children.insert(
                        segment.clone(),
                        ValueNode::Leaf {
                            path: logical_path.clone(),
                            input: input.clone(),
                            value: value.clone(),
                        },
                    );
                    return Ok(());
                }
            }
        }

        let branch_path = path[..=index].join(".");
        node = children
            .entry(segment.clone())
            .or_insert_with(|| ValueNode::Branch {
                path: branch_path,
                children: BTreeMap::new(),
            });
    }

    Ok(())
}

struct ValueNodeDeserializer<'a> {
    node: &'a ValueNode,
}

impl<'a> ValueNodeDeserializer<'a> {
    fn new(node: &'a ValueNode) -> Self {
        Self { node }
    }

    fn leaf(&self) -> Result<(&'a str, &'a str, &'a str), MappedValueError> {
        match self.node {
            ValueNode::Leaf { path, input, value } => Ok((path, input, value)),
            ValueNode::Branch { path, .. } => Err(MappedValueError::new(
                path.clone(),
                KeyValueProviderError::InvalidValue {
                    input: "<nested mapping>".to_owned(),
                    message: "expected a scalar value".to_owned(),
                },
            )),
        }
    }

    fn invalid(path: &str, input: &str, message: &'static str) -> MappedValueError {
        MappedValueError::new(
            path.to_owned(),
            KeyValueProviderError::InvalidValue {
                input: input.to_owned(),
                message: message.to_owned(),
            },
        )
    }

    fn json_value(&self) -> Result<(&'a str, &'a str, serde_json::Value), MappedValueError> {
        let (path, input, value) = self.leaf()?;
        let value = serde_json::from_str(value)
            .map_err(|_| Self::invalid(path, input, "invalid JSON value"))?;
        Ok((path, input, value))
    }
}

macro_rules! deserialize_number {
    ($method:ident, $visit:ident, $ty:ty) => {
        fn $method<V>(self, visitor: V) -> Result<V::Value, Self::Error>
        where
            V: Visitor<'de>,
        {
            let (path, input, value) = self.leaf()?;
            let parsed = value
                .parse::<$ty>()
                .map_err(|_| Self::invalid(path, input, concat!("expected ", stringify!($ty))))?;
            visitor
                .$visit::<MappedValueError>(parsed)
                .map_err(|error| error.with_path_if_unknown(path))
        }
    };
}

impl<'de> de::Deserializer<'de> for ValueNodeDeserializer<'de> {
    type Error = MappedValueError;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.node {
            ValueNode::Branch { path, children } => visitor
                .visit_map(ValueMapAccess::new(path, children))
                .map_err(|error| error.with_path_if_unknown(path)),
            ValueNode::Leaf { path, value, .. } => visitor
                .visit_borrowed_str::<MappedValueError>(value)
                .map_err(|error| error.with_path_if_unknown(path)),
        }
    }

    fn deserialize_bool<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let (path, input, value) = self.leaf()?;
        let parsed = value
            .parse::<bool>()
            .map_err(|_| Self::invalid(path, input, "expected a boolean"))?;
        visitor
            .visit_bool::<MappedValueError>(parsed)
            .map_err(|error| error.with_path_if_unknown(path))
    }

    deserialize_number!(deserialize_i8, visit_i8, i8);
    deserialize_number!(deserialize_i16, visit_i16, i16);
    deserialize_number!(deserialize_i32, visit_i32, i32);
    deserialize_number!(deserialize_i64, visit_i64, i64);
    deserialize_number!(deserialize_i128, visit_i128, i128);
    deserialize_number!(deserialize_u8, visit_u8, u8);
    deserialize_number!(deserialize_u16, visit_u16, u16);
    deserialize_number!(deserialize_u32, visit_u32, u32);
    deserialize_number!(deserialize_u64, visit_u64, u64);
    deserialize_number!(deserialize_u128, visit_u128, u128);
    deserialize_number!(deserialize_f32, visit_f32, f32);
    deserialize_number!(deserialize_f64, visit_f64, f64);

    fn deserialize_char<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let (path, input, value) = self.leaf()?;
        let mut chars = value.chars();
        let Some(value) = chars.next() else {
            return Err(Self::invalid(path, input, "expected one character"));
        };
        if chars.next().is_some() {
            return Err(Self::invalid(path, input, "expected one character"));
        }
        visitor
            .visit_char::<MappedValueError>(value)
            .map_err(|error| error.with_path_if_unknown(path))
    }

    fn deserialize_str<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let (path, _, value) = self.leaf()?;
        visitor
            .visit_borrowed_str::<MappedValueError>(value)
            .map_err(|error| error.with_path_if_unknown(path))
    }

    fn deserialize_string<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let (path, _, value) = self.leaf()?;
        visitor
            .visit_string::<MappedValueError>(value.to_owned())
            .map_err(|error| error.with_path_if_unknown(path))
    }

    fn deserialize_bytes<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let (path, _, value) = self.leaf()?;
        visitor
            .visit_borrowed_bytes::<MappedValueError>(value.as_bytes())
            .map_err(|error| error.with_path_if_unknown(path))
    }

    fn deserialize_byte_buf<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let (path, _, value) = self.leaf()?;
        visitor
            .visit_byte_buf::<MappedValueError>(value.as_bytes().to_vec())
            .map_err(|error| error.with_path_if_unknown(path))
    }

    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let path = self.node.path().to_owned();
        visitor
            .visit_some(self)
            .map_err(|error| error.with_path_if_unknown(path))
    }

    fn deserialize_unit<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let (path, input, value) = self.leaf()?;
        if value.is_empty() || value == "null" {
            visitor
                .visit_unit::<MappedValueError>()
                .map_err(|error| error.with_path_if_unknown(path))
        } else {
            Err(Self::invalid(
                path,
                input,
                "expected an empty value or `null`",
            ))
        }
    }

    fn deserialize_unit_struct<V>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_unit(visitor)
    }

    fn deserialize_newtype_struct<V>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let path = self.node.path().to_owned();
        visitor
            .visit_newtype_struct(self)
            .map_err(|error| error.with_path_if_unknown(path))
    }

    fn deserialize_seq<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let (path, input, value) = self.json_value()?;
        de::Deserializer::deserialize_seq(value, visitor)
            .map_err(|_| Self::invalid(path, input, "value does not match expected sequence"))
    }

    fn deserialize_tuple<V>(self, _len: usize, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let (path, input, value) = self.json_value()?;
        de::Deserializer::deserialize_seq(value, visitor)
            .map_err(|_| Self::invalid(path, input, "value does not match expected sequence"))
    }

    fn deserialize_tuple_struct<V>(
        self,
        _name: &'static str,
        _len: usize,
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let (path, input, value) = self.json_value()?;
        de::Deserializer::deserialize_seq(value, visitor)
            .map_err(|_| Self::invalid(path, input, "value does not match expected sequence"))
    }

    fn deserialize_map<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.node {
            ValueNode::Branch { path, children } => visitor
                .visit_map(ValueMapAccess::new(path, children))
                .map_err(|error| error.with_path_if_unknown(path)),
            ValueNode::Leaf { .. } => {
                let (path, input, value) = self.json_value()?;
                de::Deserializer::deserialize_map(value, visitor)
                    .map_err(|_| Self::invalid(path, input, "value does not match expected map"))
            }
        }
    }

    fn deserialize_struct<V>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_map(visitor)
    }

    fn deserialize_enum<V>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let (path, input, value) = self.leaf()?;
        visitor
            .visit_enum(de::value::StrDeserializer::<MappedValueError>::new(value))
            .map_err(|_| Self::invalid(path, input, "value does not match expected enum"))
    }

    fn deserialize_identifier<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        self.deserialize_str(visitor)
    }

    fn deserialize_ignored_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_unit::<MappedValueError>()
    }
}

struct ValueMapAccess<'a> {
    path: &'a str,
    iter: std::collections::btree_map::Iter<'a, String, ValueNode>,
    value: Option<&'a ValueNode>,
}

impl<'a> ValueMapAccess<'a> {
    fn new(path: &'a str, children: &'a BTreeMap<String, ValueNode>) -> Self {
        Self {
            path,
            iter: children.iter(),
            value: None,
        }
    }

    fn child_path(&self, key: &str) -> String {
        if self.path.is_empty() {
            key.to_owned()
        } else {
            format!("{}.{}", self.path, key)
        }
    }
}

impl<'de> MapAccess<'de> for ValueMapAccess<'de> {
    type Error = MappedValueError;

    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, Self::Error>
    where
        K: DeserializeSeed<'de>,
    {
        let Some((key, value)) = self.iter.next() else {
            return Ok(None);
        };
        self.value = Some(value);
        let path = self.child_path(key);
        seed.deserialize(de::value::StrDeserializer::<MappedValueError>::new(
            key.as_str(),
        ))
        .map(Some)
        .map_err(|error| error.with_path_if_unknown(path))
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, Self::Error>
    where
        V: DeserializeSeed<'de>,
    {
        let value = self.value.take().ok_or_else(|| {
            MappedValueError::new(
                self.path.to_owned(),
                KeyValueProviderError::InvalidValue {
                    input: "<nested mapping>".to_owned(),
                    message: "map value requested before map key".to_owned(),
                },
            )
        })?;
        seed.deserialize(ValueNodeDeserializer::new(value))
            .map_err(|error| error.with_path_if_unknown(value.path()))
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    #[derive(Debug, Deserialize, PartialEq)]
    struct TestConfig {
        api_key: String,
        source_path: String,
        port: u16,
        enabled: bool,
        database: DatabaseConfig,
        tags: Vec<String>,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct DatabaseConfig {
        url: String,
    }

    #[test]
    fn decodes_nested_typed_values() {
        let provider = KeyValueProvider::named("CLI overrides").with_pairs([
            ("api_key", "secret"),
            ("source_path", "/srv/app"),
            ("port", "8443"),
            ("enabled", "true"),
            ("database.url", "postgres://db/app"),
            ("tags", r#"["cli","worker"]"#),
        ]);

        let config = provider.load_partial::<TestConfig>().unwrap();

        assert_eq!(config.port, 8443);
        assert!(config.enabled);
        assert_eq!(config.database.url, "postgres://db/app");
        assert_eq!(config.tags, ["cli", "worker"]);
        assert_eq!(provider.source().label(), "CLI overrides");
    }

    #[test]
    fn nested_type_errors_do_not_expose_values() {
        #[derive(Debug, Deserialize)]
        #[allow(dead_code)]
        struct Config {
            ports: Vec<u16>,
        }

        let error = KeyValueProvider::new()
            .with("ports", r#"["super-secret"]"#)
            .load_partial::<Config>()
            .unwrap_err();
        let display = error.to_string();
        let debug = format!("{error:?}");

        assert!(!display.contains("super-secret"), "{display}");
        assert!(!debug.contains("super-secret"), "{debug}");
        assert_eq!(error.logical_path().as_deref(), Some("ports"));
    }

    #[test]
    fn rejects_conflicting_paths() {
        let provider = KeyValueProvider::new()
            .with("database", "scalar")
            .with("database.url", "postgres://db/app");

        let error = provider.load_partial::<TestConfig>().unwrap_err();

        assert!(error.to_string().contains("database.url"));
    }

    #[test]
    fn rejects_conflicting_paths_in_reverse_order() {
        let provider = KeyValueProvider::new()
            .with("database.url", "postgres://db/app")
            .with("database", "scalar");

        let error = provider.load_partial::<TestConfig>().unwrap_err();

        assert!(error.to_string().contains("database"));
    }

    #[test]
    fn later_duplicate_paths_replace_earlier_values() {
        let provider = KeyValueProvider::new()
            .with("api_key", "old")
            .with("api_key", "new")
            .with("source_path", "/srv/app")
            .with("port", "443")
            .with("enabled", "true")
            .with("database.url", "postgres://db/app")
            .with("tags", "[]");

        let config = provider.load_partial::<TestConfig>().unwrap();

        assert_eq!(config.api_key, "new");
    }
    #[test]
    fn debug_does_not_expose_values() {
        let provider = KeyValueProvider::new().with("api_key", "super-secret");
        let debug = format!("{provider:?}");

        assert!(debug.contains("api_key"));
        assert!(!debug.contains("super-secret"));
    }

    #[test]
    fn conversion_errors_report_logical_path_without_value() {
        #[derive(Debug, Deserialize)]
        #[allow(dead_code)]
        struct PortConfig {
            database: DatabaseConfig,
        }

        #[derive(Debug, Deserialize)]
        #[allow(dead_code)]
        struct DatabaseConfig {
            port: u16,
        }

        let error = KeyValueProvider::new()
            .with("database.port", "not-a-port")
            .load_partial::<PortConfig>()
            .unwrap_err();

        assert_eq!(error.logical_path().as_deref(), Some("database.port"));
        assert!(!error.to_string().contains("not-a-port"));
        assert!(!format!("{error:?}").contains("not-a-port"));
    }

    #[test]
    fn unknown_nested_keys_report_their_destination_path() {
        #[derive(Debug, Deserialize)]
        #[serde(deny_unknown_fields)]
        #[allow(dead_code)]
        struct Config {
            database: StrictDatabase,
        }

        #[derive(Debug, Deserialize)]
        #[serde(deny_unknown_fields)]
        #[allow(dead_code)]
        struct StrictDatabase {
            url: String,
        }

        let error = KeyValueProvider::new()
            .with("database.typo", "secret")
            .load_partial::<Config>()
            .unwrap_err();

        assert_eq!(error.logical_path().as_deref(), Some("database.typo"));
        assert!(!error.to_string().contains("secret"));
    }
}
