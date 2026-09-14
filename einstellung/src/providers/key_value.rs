use std::{collections::BTreeMap, fmt};

use serde::de::{self, DeserializeOwned, DeserializeSeed, IntoDeserializer, MapAccess, Visitor};
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

/// Loads dotted-path string values into a partial configuration.
///
/// This is useful for CLI overrides, secret-store adapters, and other external key/value
/// sources. Scalar fields parse from their string representation, while sequences and maps use
/// JSON syntax. Later duplicate paths replace earlier ones.
#[derive(Clone, Debug)]
pub struct KeyValueProvider {
    source: ConfigSource,
    values: Vec<KeyValueBinding>,
}

#[derive(Clone, Debug)]
struct KeyValueBinding {
    path: String,
    value: String,
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
        .map_err(|error| ConfigError::provider("key/value", error))
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
    Branch(BTreeMap<String, ValueNode>),
    Leaf { input: String, value: String },
}

pub(super) fn load_mapped_values<T>(
    values: impl IntoIterator<Item = MappedValue>,
) -> Result<T, KeyValueProviderError>
where
    T: DeserializeOwned,
{
    let mut root = ValueNode::Branch(BTreeMap::new());
    for value in values {
        insert_mapped_value(&mut root, value)?;
    }

    T::deserialize(ValueNodeDeserializer::new(&root))
}

fn insert_mapped_value(
    root: &mut ValueNode,
    mapped: MappedValue,
) -> Result<(), KeyValueProviderError> {
    let MappedValue { path, input, value } = mapped;

    if path.is_empty() || path.iter().any(String::is_empty) {
        return Err(KeyValueProviderError::ConflictingPath {
            path: path.join("."),
        });
    }

    let mut node = root;
    for (index, segment) in path.iter().enumerate() {
        let is_leaf = index + 1 == path.len();
        let ValueNode::Branch(children) = node else {
            return Err(KeyValueProviderError::ConflictingPath {
                path: path.join("."),
            });
        };

        if is_leaf {
            match children.get(segment) {
                Some(ValueNode::Branch(_)) => {
                    return Err(KeyValueProviderError::ConflictingPath {
                        path: path.join("."),
                    });
                }
                Some(ValueNode::Leaf { .. }) | None => {
                    children.insert(
                        segment.clone(),
                        ValueNode::Leaf {
                            input: input.clone(),
                            value: value.clone(),
                        },
                    );
                    return Ok(());
                }
            }
        }

        node = children
            .entry(segment.clone())
            .or_insert_with(|| ValueNode::Branch(BTreeMap::new()));
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

    fn leaf(&self) -> Result<(&'a str, &'a str), KeyValueProviderError> {
        match self.node {
            ValueNode::Leaf { input, value } => Ok((input, value)),
            ValueNode::Branch(_) => Err(KeyValueProviderError::InvalidValue {
                input: "<nested mapping>".to_owned(),
                message: "expected a scalar value".to_owned(),
            }),
        }
    }

    fn invalid(input: &str, message: impl fmt::Display) -> KeyValueProviderError {
        KeyValueProviderError::InvalidValue {
            input: input.to_owned(),
            message: message.to_string(),
        }
    }

    fn json_value(&self) -> Result<(&'a str, serde_json::Value), KeyValueProviderError> {
        let (input, value) = self.leaf()?;
        let value = serde_json::from_str(value).map_err(|err| Self::invalid(input, err))?;
        Ok((input, value))
    }
}

macro_rules! deserialize_number {
    ($method:ident, $visit:ident, $ty:ty) => {
        fn $method<V>(self, visitor: V) -> Result<V::Value, Self::Error>
        where
            V: Visitor<'de>,
        {
            let (input, value) = self.leaf()?;
            let parsed = value
                .parse::<$ty>()
                .map_err(|err| Self::invalid(input, err))?;
            visitor.$visit(parsed)
        }
    };
}

impl<'de> de::Deserializer<'de> for ValueNodeDeserializer<'de> {
    type Error = KeyValueProviderError;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.node {
            ValueNode::Branch(children) => visitor.visit_map(ValueMapAccess::new(children)),
            ValueNode::Leaf { value, .. } => visitor.visit_borrowed_str(value),
        }
    }

    fn deserialize_bool<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let (input, value) = self.leaf()?;
        let parsed = value
            .parse::<bool>()
            .map_err(|err| Self::invalid(input, err))?;
        visitor.visit_bool(parsed)
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
        let (input, value) = self.leaf()?;
        let mut chars = value.chars();
        let Some(value) = chars.next() else {
            return Err(Self::invalid(input, "expected one character"));
        };
        if chars.next().is_some() {
            return Err(Self::invalid(input, "expected one character"));
        }
        visitor.visit_char(value)
    }

    fn deserialize_str<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let (_, value) = self.leaf()?;
        visitor.visit_borrowed_str(value)
    }

    fn deserialize_string<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let (_, value) = self.leaf()?;
        visitor.visit_string(value.to_owned())
    }

    fn deserialize_bytes<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let (_, value) = self.leaf()?;
        visitor.visit_borrowed_bytes(value.as_bytes())
    }

    fn deserialize_byte_buf<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let (_, value) = self.leaf()?;
        visitor.visit_byte_buf(value.as_bytes().to_vec())
    }

    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        visitor.visit_some(self)
    }

    fn deserialize_unit<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let (input, value) = self.leaf()?;
        if value.is_empty() || value == "null" {
            visitor.visit_unit()
        } else {
            Err(Self::invalid(input, "expected an empty value or `null`"))
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
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_seq<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let (input, value) = self.json_value()?;
        de::Deserializer::deserialize_seq(value, visitor).map_err(|err| Self::invalid(input, err))
    }

    fn deserialize_tuple<V>(self, _len: usize, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let (input, value) = self.json_value()?;
        de::Deserializer::deserialize_seq(value, visitor).map_err(|err| Self::invalid(input, err))
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
        let (input, value) = self.json_value()?;
        de::Deserializer::deserialize_seq(value, visitor).map_err(|err| Self::invalid(input, err))
    }

    fn deserialize_map<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.node {
            ValueNode::Branch(children) => visitor.visit_map(ValueMapAccess::new(children)),
            ValueNode::Leaf { .. } => {
                let (input, value) = self.json_value()?;
                de::Deserializer::deserialize_map(value, visitor)
                    .map_err(|err| Self::invalid(input, err))
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
        let (_, value) = self.leaf()?;
        visitor.visit_enum(value.into_deserializer())
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
        visitor.visit_unit()
    }
}

struct ValueMapAccess<'a> {
    iter: std::collections::btree_map::Iter<'a, String, ValueNode>,
    value: Option<&'a ValueNode>,
}

impl<'a> ValueMapAccess<'a> {
    fn new(children: &'a BTreeMap<String, ValueNode>) -> Self {
        Self {
            iter: children.iter(),
            value: None,
        }
    }
}

impl<'de> MapAccess<'de> for ValueMapAccess<'de> {
    type Error = KeyValueProviderError;

    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, Self::Error>
    where
        K: DeserializeSeed<'de>,
    {
        let Some((key, value)) = self.iter.next() else {
            return Ok(None);
        };
        self.value = Some(value);
        seed.deserialize(key.as_str().into_deserializer()).map(Some)
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, Self::Error>
    where
        V: DeserializeSeed<'de>,
    {
        let value = self
            .value
            .take()
            .expect("serde requested a map value before a map key");
        seed.deserialize(ValueNodeDeserializer::new(value))
    }
}

impl de::Error for KeyValueProviderError {
    fn custom<T>(message: T) -> Self
    where
        T: fmt::Display,
    {
        Self::InvalidValue {
            input: "<key/value>".to_owned(),
            message: message.to_string(),
        }
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
}
