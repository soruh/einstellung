use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    fmt,
};

use serde::de::{self, DeserializeOwned, DeserializeSeed, IntoDeserializer, MapAccess, Visitor};
use thiserror::Error;

use crate::{ConfigError, ConfigProvider};

const NESTED_SEPARATOR: &str = "__";

/// Errors produced while selecting or decoding environment variables.
#[derive(Debug, Error)]
pub enum EnvProviderError {
    #[error("environment variable {variable:?} contains non-Unicode data")]
    NonUnicode { variable: String },

    #[error("environment mapping for {path:?} conflicts with another mapped value")]
    ConflictingPath { path: String },

    #[error("invalid value for configuration input {variable:?}: {message}")]
    InvalidValue { variable: String, message: String },
}

/// Loads explicitly selected process environment variables into a partial configuration.
///
/// The provider intentionally loads nothing unless at least one explicit mapping or prefix
/// is configured. This avoids accidentally exposing unrelated environment variables to a
/// configuration type.
///
/// Explicit variables use dotted configuration paths, for example
/// `with_var("DATABASE_URL", "database.url")`. A configured prefix maps matching variables
/// by removing the prefix, converting the remainder to ASCII lowercase, and treating `__`
/// as a nested field separator. For example `APP_DATABASE__URL` maps to `database.url`.
#[derive(Clone, Debug, Default)]
pub struct EnvProvider {
    vars: Vec<EnvBinding>,
    prefix: Option<String>,
}

#[derive(Clone, Debug)]
struct EnvBinding {
    variable: String,
    path: String,
}

/// Loads dotted-path string values into a partial configuration.
///
/// This is useful for CLI overrides, secret-store adapters, and other external key/value
/// sources. Values use the same typed decoding as [`EnvProvider`]: scalar fields parse from
/// their string representation, while sequences and maps use JSON syntax. Later duplicate
/// paths replace earlier ones.
#[derive(Clone, Debug)]
pub struct KeyValueProvider {
    source: crate::ConfigSource,
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
            source: crate::ConfigSource::new(source),
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
        load_mapped_values(
            "key/value",
            self.values.iter().map(|binding| MappedValue {
                path: binding.path.split('.').map(str::to_owned).collect(),
                variable: binding.path.clone(),
                value: binding.value.clone(),
            }),
        )
    }

    fn source(&self) -> crate::ConfigSource {
        self.source.clone()
    }
}

#[derive(Clone, Debug)]
struct MappedValue {
    path: Vec<String>,
    variable: String,
    value: String,
}

impl EnvProvider {
    /// Create an environment provider that loads no variables until configured.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a provider that loads only the named environment variables.
    ///
    /// Variable names are mapped to configuration paths by lowercasing them and treating `__`
    /// as a nested-field separator. For example, `API_KEY` maps to `api_key` and
    /// `DATABASE__URL` maps to `database.url`. Use [`Self::with_var`] when a variable needs an
    /// explicit mapping.
    pub fn only<I, S>(variables: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::new().with_vars(variables)
    }

    /// Add environment variables whose names map directly to configuration paths.
    ///
    /// This uses the same lowercase and `__` nesting rules as [`Self::only`].
    pub fn with_vars<I, S>(mut self, variables: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for variable in variables {
            let variable = variable.into();
            let path = env_key_path(&variable).join(".");
            self = self.with_var(variable, path);
        }
        self
    }

    /// Create a provider that loads variables under the given prefix.
    pub fn prefixed(prefix: impl Into<String>) -> Self {
        Self::new().with_prefix(prefix)
    }

    /// Add an explicit environment variable to configuration-field mapping.
    ///
    /// `config_path` uses `.` to address nested fields, such as `database.url`.
    pub fn with_var(mut self, variable: impl Into<String>, config_path: impl Into<String>) -> Self {
        self.vars.push(EnvBinding {
            variable: variable.into(),
            path: config_path.into(),
        });
        self
    }

    /// Load all variables under the given prefix.
    ///
    /// Prefix-based mappings lowercase the variable name after the prefix and use `__`
    /// as the nested-field separator. Explicit [`Self::with_var`] mappings take precedence
    /// when both sources map to the same configuration path.
    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.prefix = Some(prefix.into());
        self
    }

    pub(crate) fn load_from_vars<T>(
        &self,
        provider_name: &'static str,
        vars: impl IntoIterator<Item = (OsString, OsString)>,
    ) -> Result<T, ConfigError>
    where
        T: DeserializeOwned,
    {
        let vars: Vec<_> = vars.into_iter().collect();
        let mut mapped = Vec::new();

        if let Some(prefix) = &self.prefix {
            for (key, value) in &vars {
                let Some(key) = key.to_str() else {
                    continue;
                };
                let Some(suffix) = key.strip_prefix(prefix) else {
                    continue;
                };
                if suffix.is_empty() {
                    continue;
                }

                mapped.push(mapped_env_value(
                    provider_name,
                    env_key_path(suffix),
                    key,
                    value,
                )?);
            }
        }

        for binding in &self.vars {
            let Some((_, value)) = vars
                .iter()
                .find(|(key, _)| key.as_os_str() == OsStr::new(&binding.variable))
            else {
                continue;
            };

            mapped.push(mapped_env_value(
                provider_name,
                binding.path.split('.').map(str::to_owned).collect(),
                &binding.variable,
                value,
            )?);
        }

        load_mapped_values(provider_name, mapped)
    }
}

impl ConfigProvider for EnvProvider {
    fn load_partial<T: DeserializeOwned>(&self) -> Result<T, ConfigError> {
        self.load_from_vars("environment", std::env::vars_os())
    }

    fn source(&self) -> crate::ConfigSource {
        crate::ConfigSource::new("process environment")
    }
}

fn env_key_path(key: &str) -> Vec<String> {
    key.split(NESTED_SEPARATOR)
        .map(str::to_ascii_lowercase)
        .collect()
}

#[derive(Debug)]
enum EnvNode {
    Branch(BTreeMap<String, EnvNode>),
    Leaf { variable: String, value: String },
}

fn mapped_env_value(
    provider_name: &'static str,
    path: Vec<String>,
    variable: &str,
    value: &OsStr,
) -> Result<MappedValue, ConfigError> {
    let value = value
        .to_str()
        .ok_or_else(|| {
            ConfigError::provider(
                provider_name,
                EnvProviderError::NonUnicode {
                    variable: variable.to_owned(),
                },
            )
        })?
        .to_owned();

    Ok(MappedValue {
        path,
        variable: variable.to_owned(),
        value,
    })
}

fn load_mapped_values<T>(
    provider_name: &'static str,
    values: impl IntoIterator<Item = MappedValue>,
) -> Result<T, ConfigError>
where
    T: DeserializeOwned,
{
    let mut root = EnvNode::Branch(BTreeMap::new());
    for value in values {
        insert_mapped_value(provider_name, &mut root, value)?;
    }

    T::deserialize(EnvNodeDeserializer::new(&root))
        .map_err(|err| ConfigError::provider(provider_name, err))
}

fn insert_mapped_value(
    provider_name: &'static str,
    root: &mut EnvNode,
    mapped: MappedValue,
) -> Result<(), ConfigError> {
    let MappedValue {
        path,
        variable,
        value,
    } = mapped;

    if path.is_empty() || path.iter().any(String::is_empty) {
        return Err(ConfigError::provider(
            provider_name,
            EnvProviderError::ConflictingPath {
                path: path.join("."),
            },
        ));
    }

    let mut node = root;
    for (index, segment) in path.iter().enumerate() {
        let is_leaf = index + 1 == path.len();
        let EnvNode::Branch(children) = node else {
            return Err(ConfigError::provider(
                provider_name,
                EnvProviderError::ConflictingPath {
                    path: path.join("."),
                },
            ));
        };

        if is_leaf {
            children.insert(
                segment.clone(),
                EnvNode::Leaf {
                    variable: variable.clone(),
                    value: value.clone(),
                },
            );
            return Ok(());
        }

        node = children
            .entry(segment.clone())
            .or_insert_with(|| EnvNode::Branch(BTreeMap::new()));
    }

    Ok(())
}

struct EnvNodeDeserializer<'a> {
    node: &'a EnvNode,
}

impl<'a> EnvNodeDeserializer<'a> {
    fn new(node: &'a EnvNode) -> Self {
        Self { node }
    }

    fn leaf(&self) -> Result<(&'a str, &'a str), EnvProviderError> {
        match self.node {
            EnvNode::Leaf { variable, value } => Ok((variable, value)),
            EnvNode::Branch(_) => Err(EnvProviderError::InvalidValue {
                variable: "<nested environment mapping>".to_owned(),
                message: "expected a scalar value".to_owned(),
            }),
        }
    }

    fn invalid(variable: &str, message: impl fmt::Display) -> EnvProviderError {
        EnvProviderError::InvalidValue {
            variable: variable.to_owned(),
            message: message.to_string(),
        }
    }

    fn json_value(&self) -> Result<(&'a str, serde_json::Value), EnvProviderError> {
        let (variable, value) = self.leaf()?;
        let value = serde_json::from_str(value).map_err(|err| Self::invalid(variable, err))?;
        Ok((variable, value))
    }
}

macro_rules! deserialize_number {
    ($method:ident, $visit:ident, $ty:ty) => {
        fn $method<V>(self, visitor: V) -> Result<V::Value, Self::Error>
        where
            V: Visitor<'de>,
        {
            let (variable, value) = self.leaf()?;
            let parsed = value
                .parse::<$ty>()
                .map_err(|err| Self::invalid(variable, err))?;
            visitor.$visit(parsed)
        }
    };
}

impl<'de> de::Deserializer<'de> for EnvNodeDeserializer<'de> {
    type Error = EnvProviderError;

    fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.node {
            EnvNode::Branch(children) => visitor.visit_map(EnvMapAccess::new(children)),
            EnvNode::Leaf { value, .. } => visitor.visit_borrowed_str(value),
        }
    }

    fn deserialize_bool<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let (variable, value) = self.leaf()?;
        let parsed = value
            .parse::<bool>()
            .map_err(|err| Self::invalid(variable, err))?;
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
        let (variable, value) = self.leaf()?;
        let mut chars = value.chars();
        let Some(value) = chars.next() else {
            return Err(Self::invalid(variable, "expected one character"));
        };
        if chars.next().is_some() {
            return Err(Self::invalid(variable, "expected one character"));
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
        let (variable, value) = self.leaf()?;
        if value.is_empty() || value == "null" {
            visitor.visit_unit()
        } else {
            Err(Self::invalid(variable, "expected an empty value or `null`"))
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
        let (variable, value) = self.json_value()?;
        de::Deserializer::deserialize_seq(value, visitor)
            .map_err(|err| Self::invalid(variable, err))
    }

    fn deserialize_tuple<V>(self, _len: usize, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        let (variable, value) = self.json_value()?;
        de::Deserializer::deserialize_seq(value, visitor)
            .map_err(|err| Self::invalid(variable, err))
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
        let (variable, value) = self.json_value()?;
        de::Deserializer::deserialize_seq(value, visitor)
            .map_err(|err| Self::invalid(variable, err))
    }

    fn deserialize_map<V>(self, visitor: V) -> Result<V::Value, Self::Error>
    where
        V: Visitor<'de>,
    {
        match self.node {
            EnvNode::Branch(children) => visitor.visit_map(EnvMapAccess::new(children)),
            EnvNode::Leaf { .. } => {
                let (variable, value) = self.json_value()?;
                de::Deserializer::deserialize_map(value, visitor)
                    .map_err(|err| Self::invalid(variable, err))
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

struct EnvMapAccess<'a> {
    iter: std::collections::btree_map::Iter<'a, String, EnvNode>,
    value: Option<&'a EnvNode>,
}

impl<'a> EnvMapAccess<'a> {
    fn new(children: &'a BTreeMap<String, EnvNode>) -> Self {
        Self {
            iter: children.iter(),
            value: None,
        }
    }
}

impl<'de> MapAccess<'de> for EnvMapAccess<'de> {
    type Error = EnvProviderError;

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
        seed.deserialize(EnvNodeDeserializer::new(value))
    }
}

impl de::Error for EnvProviderError {
    fn custom<T>(message: T) -> Self
    where
        T: fmt::Display,
    {
        Self::InvalidValue {
            variable: "<environment>".to_owned(),
            message: message.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

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

    fn vars(values: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        values
            .iter()
            .map(|(key, value)| (OsString::from(key), OsString::from(value)))
            .collect()
    }

    #[test]
    fn explicit_mappings_are_allowlisted_and_typed() {
        let provider = EnvProvider::new()
            .with_var("API_KEY", "api_key")
            .with_var("SOURCE_PATH", "source_path")
            .with_var("PORT", "port")
            .with_var("ENABLED", "enabled")
            .with_var("DATABASE_URL", "database.url")
            .with_var("TAGS", "tags");

        let config = provider
            .load_from_vars::<TestConfig>(
                "environment",
                vars(&[
                    ("API_KEY", "123"),
                    ("SOURCE_PATH", "/tmp/project"),
                    ("PORT", "8080"),
                    ("ENABLED", "true"),
                    ("DATABASE_URL", "postgres://localhost/app"),
                    ("TAGS", r#"["api","worker"]"#),
                    ("IGNORED", "does-not-load"),
                ]),
            )
            .unwrap();

        assert_eq!(config.api_key, "123");
        assert_eq!(config.source_path, "/tmp/project");
        assert_eq!(config.port, 8080);
        assert!(config.enabled);
        assert_eq!(config.database.url, "postgres://localhost/app");
        assert_eq!(config.tags, ["api", "worker"]);
    }

    #[test]
    fn only_maps_selected_variables_by_name() {
        let provider = EnvProvider::only([
            "API_KEY",
            "SOURCE_PATH",
            "PORT",
            "ENABLED",
            "DATABASE__URL",
            "TAGS",
        ]);
        let config = provider
            .load_from_vars::<TestConfig>(
                "environment",
                vars(&[
                    ("API_KEY", "secret"),
                    ("SOURCE_PATH", "/srv/app"),
                    ("PORT", "443"),
                    ("ENABLED", "true"),
                    ("DATABASE__URL", "postgres://db/app"),
                    ("TAGS", r#"["one"]"#),
                    ("IGNORED", "does-not-load"),
                ]),
            )
            .unwrap();

        assert_eq!(config.api_key, "secret");
        assert_eq!(config.database.url, "postgres://db/app");
        assert_eq!(config.tags, ["one"]);
    }

    #[test]
    fn prefix_maps_nested_fields() {
        let provider = EnvProvider::prefixed("APP_");
        let config = provider
            .load_from_vars::<TestConfig>(
                "environment",
                vars(&[
                    ("APP_API_KEY", "secret"),
                    ("APP_SOURCE_PATH", "/srv/app"),
                    ("APP_PORT", "443"),
                    ("APP_ENABLED", "false"),
                    ("APP_DATABASE__URL", "postgres://db/app"),
                    ("APP_TAGS", r#"["one"]"#),
                    ("OTHER_PORT", "80"),
                ]),
            )
            .unwrap();

        assert_eq!(config.api_key, "secret");
        assert_eq!(config.port, 443);
        assert!(!config.enabled);
        assert_eq!(config.database.url, "postgres://db/app");
    }

    #[test]
    fn explicit_mapping_overrides_prefix_mapping() {
        let provider = EnvProvider::new()
            .with_prefix("APP_")
            .with_var("SPECIAL_PORT", "port");
        let config = provider
            .load_from_vars::<TestConfig>(
                "environment",
                vars(&[
                    ("APP_API_KEY", "secret"),
                    ("APP_SOURCE_PATH", "/srv/app"),
                    ("APP_PORT", "443"),
                    ("APP_ENABLED", "true"),
                    ("APP_DATABASE__URL", "postgres://db/app"),
                    ("APP_TAGS", "[]"),
                    ("SPECIAL_PORT", "8443"),
                ]),
            )
            .unwrap();

        assert_eq!(config.port, 8443);
    }

    #[test]
    fn key_value_provider_decodes_nested_typed_values() {
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
    fn key_value_provider_rejects_conflicting_paths() {
        let provider = KeyValueProvider::new()
            .with("database", "scalar")
            .with("database.url", "postgres://db/app");

        let error = provider.load_partial::<TestConfig>().unwrap_err();

        assert!(error.to_string().contains("database.url"));
    }
}
