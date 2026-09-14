use std::{error::Error as StdError, fmt::Display};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use thiserror::Error;

#[cfg(feature = "derive")]
pub use einstellung_derive::Config;

#[doc(hidden)]
pub use serde;

#[cfg(all(test, feature = "derive", feature = "json"))]
pub mod tests;

mod providers;

/// Reusable dependency-free validators for common configuration invariants.
pub mod validators;

pub use providers::*;

/// Describes a Configuration which can be built from its associated `::Partial` configuration.
/// The Partial type contains optional variants of all fields of the Config.
pub trait Config: Sized {
    /// The Partial Configuration for this type. Contains all fields of this type, made optional.
    /// See the documentation for [`PartialConfig`] for merging / building partial configs.
    type Partial: PartialConfig<Complete = Self>;

    /// Load this config as a [`PartialConfig`] for merging with other partial configs.
    fn load_partial(provider: &impl ConfigProvider) -> Result<Self::Partial, ConfigError> {
        provider
            .load_partial::<Self::Partial>()
            .map_err(|error| error.with_source(provider.source()))
    }

    /// Load this config in its complete form.
    fn load_complete(provider: &impl ConfigProvider) -> Result<Self, ConfigError> {
        let source = provider.source();
        Self::load_partial(provider)?
            .build()
            .map_err(|error| error.with_source(source))
    }

    /// Start a builder for composing multiple configuration layers.
    ///
    /// Providers are merged in call order, with each later provider taking precedence.
    /// Field defaults are applied only when [`ConfigBuilder::build`] constructs the final
    /// configuration.
    fn builder() -> ConfigBuilder<Self> {
        ConfigBuilder::new()
    }
}

/// A mode-specific typed view of a complete configuration.
///
/// Views let applications keep fields optional in the shared configuration while requiring them
/// for a particular command or operating mode. Implementations receive an already built and
/// validated base config and may move values into a stricter target type.
pub trait ConfigView<C>: Sized {
    /// Convert a complete base config into this view.
    fn from_config(config: C) -> Result<Self, ConfigError>;
}

/// Require an optional value while constructing a mode-specific [`ConfigView`].
///
/// This is a convenience for the common pattern where the shared configuration keeps a field
/// optional, but one command/view requires it. The returned error records the target view type
/// and logical dotted field path.
pub fn require_for_view<V, T>(
    value: Option<T>,
    field: impl Into<String>,
) -> Result<T, ConfigError> {
    value.ok_or_else(|| ConfigError::missing_for_view::<V>(field))
}

/// Composes configuration layers before building a complete [`Config`].
///
/// Providers are merged in call order. The builder records which sources supplied each field so
/// callers can inspect provenance with [`Self::build_tracked`]. Once a layer fails, later providers
/// are not loaded.
pub struct ConfigBuilder<C: Config> {
    state: ConfigBuilderState<C::Partial>,
    provenance: ConfigProvenance,
}

enum ConfigBuilderState<P> {
    Ready(P),
    Failed(ConfigError),
}

impl<C: Config> ConfigBuilder<C> {
    /// Create an empty configuration builder.
    pub fn new() -> Self {
        Self {
            state: ConfigBuilderState::Ready(C::Partial::default()),
            provenance: ConfigProvenance::default(),
        }
    }

    /// Merge a provider as the next, higher-precedence layer.
    pub fn provider(self, provider: &impl ConfigProvider) -> Self {
        self.provider_with(|| {
            let source = provider.source();
            let next = provider.load_partial::<C::Partial>();
            (source, next)
        })
    }

    /// Merge a provider as the next layer using an explicit diagnostic/provenance label.
    ///
    /// This is useful when several providers have the same generic source identity, for example
    /// multiple inline JSON documents. The supplied label replaces the provider's default source
    /// label for this composition step and should not contain configuration values or secrets.
    pub fn provider_named(self, source: impl Into<String>, provider: &impl ConfigProvider) -> Self {
        let source = ConfigSource::new(source);
        self.provider_with(|| (source, provider.load_partial::<C::Partial>()))
    }

    /// Merge multiple providers in iterator order.
    ///
    /// Each provider is the next higher-precedence layer. If one provider fails, later providers
    /// are not loaded. Use [`Self::typed_providers`] for heterogeneous runtime-selected sources.
    pub fn providers<'a, P, I>(self, providers: I) -> Self
    where
        P: ConfigProvider + 'a,
        I: IntoIterator<Item = &'a P>,
    {
        let mut builder = self;
        let mut providers = providers.into_iter();
        while matches!(&builder.state, ConfigBuilderState::Ready(_)) {
            let Some(provider) = providers.next() else {
                break;
            };
            builder = builder.provider(provider);
        }
        builder
    }

    /// Merge an object-safe provider for this specific configuration type.
    ///
    /// This is useful when the set of providers is selected at runtime and needs to be stored as
    /// heterogeneous trait objects. Ordinary providers automatically implement
    /// [`ConfigProviderFor<C>`], so they can be boxed without writing an application-level enum.
    pub fn typed_provider<P>(self, provider: &P) -> Self
    where
        P: ConfigProviderFor<C> + ?Sized,
    {
        self.provider_with(|| {
            let source = provider.config_source();
            let next = provider.load_config_partial();
            (source, next)
        })
    }

    /// Merge an object-safe provider using an explicit diagnostic/provenance label.
    pub fn typed_provider_named<P>(self, source: impl Into<String>, provider: &P) -> Self
    where
        P: ConfigProviderFor<C> + ?Sized,
    {
        let source = ConfigSource::new(source);
        self.provider_with(|| (source, provider.load_config_partial()))
    }

    /// Merge multiple object-safe providers in iterator order.
    ///
    /// This is the heterogeneous counterpart to [`Self::providers`]. The iterator normally comes
    /// from a collection of boxed providers, for example
    /// `providers.iter().map(|provider| provider.as_ref())`.
    pub fn typed_providers<'a, I>(self, providers: I) -> Self
    where
        C: 'a,
        I: IntoIterator<Item = &'a dyn ConfigProviderFor<C>>,
    {
        let mut builder = self;
        let mut providers = providers.into_iter();
        while matches!(&builder.state, ConfigBuilderState::Ready(_)) {
            let Some(provider) = providers.next() else {
                break;
            };
            builder = builder.typed_provider(provider);
        }
        builder
    }

    fn provider_with<F>(self, load: F) -> Self
    where
        F: FnOnce() -> (ConfigSource, Result<C::Partial, ConfigError>),
    {
        let Self { state, provenance } = self;
        let ConfigBuilderState::Ready(current) = state else {
            return Self { state, provenance };
        };

        let (source, next) = load();
        let next = match next {
            Ok(next) => next,
            Err(error) => {
                return Self {
                    state: ConfigBuilderState::Failed(error.with_source(source)),
                    provenance,
                };
            }
        };
        let fields = next.provided_fields();

        match current.merge(next) {
            Ok(merged) => {
                let mut provenance = provenance;
                provenance.record(source, fields);
                Self {
                    state: ConfigBuilderState::Ready(merged),
                    provenance,
                }
            }
            Err(error) => Self {
                state: ConfigBuilderState::Failed(error.with_source(source)),
                provenance,
            },
        }
    }

    /// Merge an already-loaded partial configuration as the next layer.
    ///
    /// Use [`Self::layer_named`] when provenance for manually constructed layers matters.
    pub fn layer(self, next: C::Partial) -> Self {
        self.layer_named("manual layer", next)
    }

    /// Merge an already-loaded partial configuration with an explicit provenance label.
    pub fn layer_named(self, source: impl Into<String>, next: C::Partial) -> Self {
        let Self { state, provenance } = self;
        let ConfigBuilderState::Ready(current) = state else {
            return Self { state, provenance };
        };

        let source = ConfigSource::new(source);
        let fields = next.provided_fields();
        match current.merge(next) {
            Ok(merged) => {
                let mut provenance = provenance;
                provenance.record(source, fields);
                Self {
                    state: ConfigBuilderState::Ready(merged),
                    provenance,
                }
            }
            Err(error) => Self {
                state: ConfigBuilderState::Failed(error.with_source(source)),
                provenance,
            },
        }
    }

    /// Merge a previously tracked partial configuration as the next layer.
    ///
    /// Unlike [`Self::layer_named`], this preserves the partial's existing per-field provenance
    /// instead of replacing it with one synthetic source label. This is useful when composition is
    /// performed in stages and an intermediate [`Self::build_tracked_partial`] result is later
    /// merged into another builder.
    pub fn tracked_layer(self, tracked: TrackedConfig<C::Partial>) -> Self {
        let Self { state, provenance } = self;
        let ConfigBuilderState::Ready(current) = state else {
            return Self { state, provenance };
        };
        let TrackedConfig {
            config: next,
            provenance: next_provenance,
        } = tracked;

        match current.merge(next) {
            Ok(merged) => {
                let mut provenance = provenance;
                provenance.extend(next_provenance);
                Self {
                    state: ConfigBuilderState::Ready(merged),
                    provenance,
                }
            }
            Err(error) => {
                let source = error
                    .logical_path()
                    .as_deref()
                    .and_then(|path| next_provenance.latest_supplier(path))
                    .cloned();
                let error = match source {
                    Some(source) => error.with_source(source),
                    None => error,
                };
                Self {
                    state: ConfigBuilderState::Failed(error),
                    provenance,
                }
            }
        }
    }

    /// Return the merged partial configuration without applying field defaults or validation.
    pub fn build_partial(self) -> Result<C::Partial, ConfigError> {
        self.build_tracked_partial().map(TrackedConfig::into_inner)
    }

    /// Return the merged partial configuration together with its source provenance.
    ///
    /// Field defaults are not applied and therefore are not recorded. This is useful when a
    /// composed partial will be inspected, transformed, or merged again before final construction.
    pub fn build_tracked_partial(self) -> Result<TrackedConfig<C::Partial>, ConfigError> {
        match self.state {
            ConfigBuilderState::Failed(error) => Err(error.with_provenance(self.provenance)),
            ConfigBuilderState::Ready(config) => Ok(TrackedConfig {
                config,
                provenance: self.provenance,
            }),
        }
    }

    /// Build the final configuration, applying field defaults and validation.
    pub fn build(self) -> Result<C, ConfigError> {
        self.finish().map(|tracked| tracked.config)
    }

    /// Build the final configuration while retaining field provenance.
    pub fn build_tracked(self) -> Result<TrackedConfig<C>, ConfigError> {
        self.finish()
    }

    /// Build a mode-specific typed view of this configuration.
    pub fn build_view<V>(self) -> Result<V, ConfigError>
    where
        V: ConfigView<C>,
    {
        let tracked = self.finish()?;
        let TrackedConfig { config, provenance } = tracked;
        V::from_config(config).map_err(|error| error.with_provenance(provenance))
    }

    /// Build a mode-specific typed view while retaining field provenance.
    pub fn build_tracked_view<V>(self) -> Result<TrackedConfig<V>, ConfigError>
    where
        V: ConfigView<C>,
    {
        self.finish()?.into_view()
    }

    fn finish(self) -> Result<TrackedConfig<C>, ConfigError> {
        let Self {
            state,
            mut provenance,
        } = self;
        let partial = match state {
            ConfigBuilderState::Ready(partial) => partial,
            ConfigBuilderState::Failed(error) => return Err(error.with_provenance(provenance)),
        };

        provenance.record(ConfigSource::defaults(), partial.defaulted_fields());

        match partial.build() {
            Ok(config) => Ok(TrackedConfig { config, provenance }),
            Err(error) => Err(error.with_provenance(provenance)),
        }
    }
}

impl<C: Config> Default for ConfigBuilder<C> {
    fn default() -> Self {
        Self::new()
    }
}

/// Provenance recorded while composing configuration layers.
///
/// Values are never stored here: provenance contains only logical field paths and source labels,
/// making it safe to use for secret-bearing fields as long as provider labels themselves contain
/// no secret data.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConfigProvenance {
    fields: std::collections::BTreeMap<String, Vec<ConfigSource>>,
}

impl ConfigProvenance {
    fn record(&mut self, source: ConfigSource, fields: Vec<String>) {
        for field in fields {
            self.fields.entry(field).or_default().push(source.clone());
        }
    }

    fn extend(&mut self, other: Self) {
        for (field, mut sources) in other.fields {
            self.fields.entry(field).or_default().append(&mut sources);
        }
    }

    /// Return every source that supplied a value for `path`, in merge order.
    ///
    /// For replace semantics the last source is the winner. For extend/custom merge strategies,
    /// earlier sources may still contribute to the final value, so the complete history is kept.
    pub fn explain(&self, path: impl AsRef<str>) -> Option<&[ConfigSource]> {
        self.fields.get(path.as_ref()).map(Vec::as_slice)
    }

    /// Return the most recent source that supplied `path`.
    ///
    /// This is the winning source for the default replace strategy. Extend, custom-merge, and
    /// frozen fields can retain values from earlier layers, so callers that need the full story
    /// should use [`Self::explain`].
    pub fn latest_supplier(&self, path: impl AsRef<str>) -> Option<&ConfigSource> {
        self.explain(path).and_then(|sources| sources.last())
    }

    /// Iterate over every tracked logical field path and its source history.
    ///
    /// Values are intentionally not exposed. Paths are returned in deterministic lexical order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &[ConfigSource])> {
        self.fields
            .iter()
            .map(|(path, sources)| (path.as_str(), sources.as_slice()))
    }

    /// Return whether no field provenance has been recorded.
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }
}

/// A built configuration together with the provenance captured during composition.
#[derive(Clone, Debug)]
pub struct TrackedConfig<C> {
    config: C,
    provenance: ConfigProvenance,
}

impl<C> TrackedConfig<C> {
    /// Borrow the built configuration.
    pub fn config(&self) -> &C {
        &self.config
    }

    /// Borrow the captured provenance.
    pub fn provenance(&self) -> &ConfigProvenance {
        &self.provenance
    }

    /// Return the source history for a logical dotted field path.
    pub fn explain(&self, path: impl AsRef<str>) -> Option<&[ConfigSource]> {
        self.provenance.explain(path)
    }

    /// Convert the configuration into a mode-specific typed view while retaining provenance.
    pub fn into_view<V>(self) -> Result<TrackedConfig<V>, ConfigError>
    where
        V: ConfigView<C>,
    {
        let Self { config, provenance } = self;
        match V::from_config(config) {
            Ok(config) => Ok(TrackedConfig { config, provenance }),
            Err(error) => Err(error.with_provenance(provenance)),
        }
    }

    /// Consume the wrapper and return the configuration and its provenance separately.
    pub fn into_parts(self) -> (C, ConfigProvenance) {
        (self.config, self.provenance)
    }

    /// Consume the wrapper and return the configuration, discarding provenance.
    pub fn into_inner(self) -> C {
        self.config
    }
}

/// A Partial variant of a [`trait@Config`]. This means that every field is optional
/// allowing incremental merging of configs.
pub trait PartialConfig: Default + DeserializeOwned {
    /// The associated Complete Config
    type Complete: Config;

    /// Merge two partial configs, treating `next` as the higher-precedence layer.
    ///
    /// For the default replace strategy, a missing value in `next` leaves the value from
    /// `self` unchanged; absence does not clear an earlier value. See the derive macro for
    /// [`derive@Config`] for extend, custom, subconfig, and freeze behavior.
    fn merge(self, next: Self) -> Result<Self, ConfigError>;

    /// Build this partial config into its complete form. All required fields need to be present for this to succeed.
    /// See the derive macro for [`derive@Config`] for how to define validation strategies and field contents.
    fn build(self) -> Result<Self::Complete, ConfigError>;

    /// Return logical dotted paths for fields explicitly present in this partial.
    #[doc(hidden)]
    fn provided_fields(&self) -> Vec<String> {
        Vec::new()
    }

    /// Return logical dotted paths whose values will be supplied by field defaults.
    #[doc(hidden)]
    fn defaulted_fields(&self) -> Vec<String> {
        Vec::new()
    }
}

/// Indicates that parts of this type can be "frozen".
/// This means that these parts cannot be overwritten by merges in any way.
/// See the derive macro for [`derive@Config`] for how to mark fields as [`trait@Freezable`]
pub trait Freezable {
    /// Freeze the freezable parts of this type
    fn freeze(self) -> Self;

    /// Check if any parts of this type are frozen
    fn is_frozen(&self) -> bool;
}

/// Generic provider for loading a partial configuration.
///
/// This can be any type which can produce a `T: DeserializeOwned`. The generic
/// [`ConfigProvider::load_partial`] method intentionally makes this trait non-object-safe. Use
/// [`ConfigProviderFor<C>`] when heterogeneous providers need to be stored behind trait objects for
/// a specific configuration type, or [`FormatProvider`] when only the structured file format is
/// selected at runtime.
///
/// See the `json`, `yaml` and `toml` features and the associated [`JsonFileProvider`],
/// [`YamlFileProvider`] and [`TomlFileProvider`] types for the built-in implementations. The
/// [`FileContentProvider`] provides an ergonomic interface to specify the location or contents of
/// an input file.
pub trait ConfigProvider {
    /// Load a [`PartialConfig`] (or any other deserializable type) from this provider.
    fn load_partial<T: DeserializeOwned>(&self) -> Result<T, ConfigError>;

    /// Describe this provider for diagnostics and provenance.
    ///
    /// Implementations should identify the source without exposing configuration contents. File
    /// providers, for example, should include the path but never inline source text.
    fn source(&self) -> ConfigSource {
        ConfigSource::new(::core::any::type_name::<Self>())
    }
}

/// Object-safe provider adapter for one concrete [`Config`] type.
///
/// Every [`ConfigProvider`] automatically implements this trait for every compatible config type.
/// Fixing the target config at the trait level removes the generic method that prevents
/// [`ConfigProvider`] itself from being used as a trait object. This makes runtime composition
/// possible without changing the flexible generic provider API.
///
/// ```
/// # #[cfg(all(feature = "derive", feature = "json", feature = "toml"))] {
/// use einstellung::{Config, ConfigProviderFor, JsonFileProvider, TomlFileProvider};
///
/// #[derive(Config)]
/// struct AppConfig {
///     name: String,
/// }
///
/// let providers: Vec<Box<dyn ConfigProviderFor<AppConfig>>> = vec![
///     Box::new(JsonFileProvider::from_owned_contents(r#"{"name":"json"}"#.to_owned())),
///     Box::new(TomlFileProvider::from_owned_contents("name = \"toml\"".to_owned())),
/// ];
///
/// let config = AppConfig::builder()
///     .typed_providers(providers.iter().map(|provider| provider.as_ref()))
///     .build()
///     .unwrap();
/// assert_eq!(config.name, "toml");
/// # }
/// ```
pub trait ConfigProviderFor<C: Config> {
    /// Load the partial associated with `C`.
    fn load_config_partial(&self) -> Result<C::Partial, ConfigError>;

    /// Describe this provider for diagnostics and provenance.
    fn config_source(&self) -> ConfigSource;
}

impl<C, P> ConfigProviderFor<C> for P
where
    C: Config,
    P: ConfigProvider,
{
    fn load_config_partial(&self) -> Result<C::Partial, ConfigError> {
        self.load_partial::<C::Partial>()
    }

    fn config_source(&self) -> ConfigSource {
        ConfigProvider::source(self)
    }
}

/// Human-readable identity of a configuration source.
///
/// Source labels are intended for diagnostics and provenance. They must not contain secret values
/// or raw configuration contents.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ConfigSource {
    label: String,
}

impl ConfigSource {
    /// Create a source label.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
        }
    }

    /// Return the source label.
    pub fn label(&self) -> &str {
        &self.label
    }

    fn defaults() -> Self {
        Self::new("field default")
    }
}

impl Display for ConfigSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

/// Indicates where a configuration error occurred.
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

    /// Return the logical dotted path, omitting the Rust config type name.
    pub fn logical_path(&self) -> String {
        self.path
            .iter()
            .rev()
            .copied()
            .chain(::core::iter::once(self.field))
            .collect::<Vec<_>>()
            .join(".")
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

#[doc(hidden)]
pub fn merge_with_context<P: PartialConfig>(
    current: P,
    next: P,
    complete: &'static str,
    segment: &'static str,
) -> Result<P, ConfigError> {
    current
        .merge(next)
        .map_err(|err| context(err, complete, segment))
}

fn context(error: ConfigError, complete: &'static str, segment: &'static str) -> ConfigError {
    match error {
        ConfigError::MissingField(field) => {
            ConfigError::MissingField(field.context(complete, segment))
        }
        ConfigError::FreezeCollision(field) => {
            ConfigError::FreezeCollision(field.context(complete, segment))
        }
        ConfigError::Validation { field, reason } => ConfigError::Validation {
            field: field.context(complete, segment),
            reason,
        },
        ConfigError::CustomMerge { field, reason } => ConfigError::CustomMerge {
            field: field.context(complete, segment),
            reason,
        },
        ConfigError::Source { source, error } => ConfigError::Source {
            source,
            error: Box::new(context(*error, complete, segment)),
        },
        ConfigError::Path { path, error } => ConfigError::Path {
            path: format!("{segment}.{path}"),
            error: Box::new(context(*error, complete, segment)),
        },
        ConfigError::Composition { provenance, error } => ConfigError::Composition {
            provenance,
            error: Box::new(context(*error, complete, segment)),
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

/// One-based location in a structured configuration source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceLocation {
    line: u64,
    column: u64,
}

impl SourceLocation {
    /// Create a source location. Line and column numbers are one-based.
    pub const fn new(line: u64, column: u64) -> Self {
        Self { line, column }
    }

    /// Return the one-based line number.
    pub const fn line(self) -> u64 {
        self.line
    }

    /// Return the one-based column number.
    pub const fn column(self) -> u64 {
        self.column
    }
}

/// Thread-safe boxed error used by configuration callbacks and providers.
pub type BoxError = Box<dyn StdError + Send + Sync + 'static>;

#[doc(hidden)]
pub fn into_box_error<E>(error: E) -> BoxError
where
    E: Into<BoxError>,
{
    error.into()
}

#[cfg(feature = "json")]
/// JSON parser error with potentially sensitive values suppressed by default.
pub struct JsonError(serde_json::Error);

#[cfg(feature = "json")]
impl JsonError {
    /// Return the safe line/column location reported by the JSON parser, when available.
    ///
    /// `serde_json` uses zero line/column values for errors that are not tied to an input
    /// position, such as reader I/O failures. Those are represented as `None` rather than an
    /// invalid one-based [`SourceLocation`].
    pub fn location(&self) -> Option<SourceLocation> {
        let line = self.0.line() as u64;
        let column = self.0.column() as u64;
        (line != 0 && column != 0).then(|| SourceLocation::new(line, column))
    }

    /// Borrow the backend parser error for explicitly requested detailed diagnostics.
    pub fn parser_error(&self) -> &serde_json::Error {
        &self.0
    }
}

#[cfg(feature = "json")]
impl Display for JsonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kind = match self.0.classify() {
            serde_json::error::Category::Io => "I/O error",
            serde_json::error::Category::Syntax => "syntax error",
            serde_json::error::Category::Data => "data error",
            serde_json::error::Category::Eof => "unexpected end of input",
        };
        match self.location() {
            Some(location) => write!(
                f,
                "{kind} at line {}, column {}",
                location.line(),
                location.column()
            ),
            None => f.write_str(kind),
        }
    }
}

#[cfg(feature = "json")]
impl std::fmt::Debug for JsonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("JsonError").field(&self.to_string()).finish()
    }
}

#[cfg(feature = "json")]
impl StdError for JsonError {}

#[cfg(feature = "json")]
impl From<serde_json::Error> for JsonError {
    fn from(error: serde_json::Error) -> Self {
        Self(error)
    }
}

#[cfg(feature = "yaml")]
/// YAML parser error with source snippets suppressed in its default display.
///
/// The underlying [`serde_saphyr::Error`] remains available through [`YamlError::parser_error`]
/// so applications can opt into richer parser-specific diagnostics explicitly. It is not exposed
/// through the normal error source chain because generic error reporters commonly print that chain.
/// Suppressing snippets by default avoids logging unrelated secret-bearing lines surrounding a
/// syntax error.
pub struct YamlError(serde_saphyr::Error);

#[cfg(feature = "yaml")]
impl Display for YamlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(location) = self.0.location() {
            write!(
                f,
                "configuration parse error at line {}, column {}",
                location.line(),
                location.column()
            )
        } else {
            f.write_str("configuration parse error")
        }
    }
}

#[cfg(feature = "yaml")]
impl std::fmt::Debug for YamlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("YamlError").field(&self.to_string()).finish()
    }
}

#[cfg(feature = "yaml")]
impl YamlError {
    /// Return the safe line/column location reported by the YAML parser, when available.
    pub fn location(&self) -> Option<SourceLocation> {
        self.0
            .location()
            .map(|location| SourceLocation::new(location.line(), location.column()))
    }

    /// Borrow the backend parser error for explicitly requested detailed diagnostics.
    pub fn parser_error(&self) -> &serde_saphyr::Error {
        &self.0
    }
}

#[cfg(feature = "yaml")]
impl StdError for YamlError {}

#[cfg(feature = "yaml")]
impl From<serde_saphyr::Error> for YamlError {
    fn from(error: serde_saphyr::Error) -> Self {
        Self(error)
    }
}

#[cfg(feature = "toml")]
/// TOML parser error with source text suppressed in its default display.
pub struct TomlError {
    error: ::toml::de::Error,
    location: Option<(usize, usize)>,
}

#[cfg(feature = "toml")]
impl TomlError {
    pub(crate) fn with_input(error: ::toml::de::Error, input: &str) -> Self {
        let location = error.span().map(|span| line_column(input, span.start));
        Self { error, location }
    }

    /// Return the safe line/column location derived from the TOML parser span, when available.
    pub fn location(&self) -> Option<SourceLocation> {
        self.location
            .map(|(line, column)| SourceLocation::new(line as u64, column as u64))
    }

    /// Borrow the backend parser error for explicitly requested detailed diagnostics.
    pub fn parser_error(&self) -> &::toml::de::Error {
        &self.error
    }
}

#[cfg(feature = "toml")]
impl Display for TomlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some((line, column)) = self.location {
            write!(
                f,
                "configuration parse error at line {line}, column {column}"
            )
        } else {
            f.write_str("configuration parse error")
        }
    }
}

#[cfg(feature = "toml")]
impl std::fmt::Debug for TomlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("TomlError").field(&self.to_string()).finish()
    }
}

#[cfg(feature = "toml")]
impl StdError for TomlError {}

#[cfg(feature = "toml")]
impl From<::toml::de::Error> for TomlError {
    fn from(error: ::toml::de::Error) -> Self {
        Self {
            error,
            location: None,
        }
    }
}

#[cfg(feature = "toml")]
fn line_column(input: &str, offset: usize) -> (usize, usize) {
    let mut line = 1;
    let mut column = 1;
    for (index, ch) in input.char_indices() {
        if index >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    (line, column)
}

/// Errors which can be produced while loading, merging, or building a configuration.
#[derive(Error, Debug)]
#[non_exhaustive]
pub enum ConfigError {
    #[error("IO Error: {0}")]
    Io(#[from] std::io::Error),

    #[cfg(feature = "json")]
    #[error("JSON Parse Error: {0}")]
    Json(#[from] JsonError),

    #[cfg(feature = "yaml")]
    #[error("YAML Parse Error: {0}")]
    Yaml(#[from] YamlError),

    #[cfg(feature = "toml")]
    #[error("TOML Parse Error: {0}")]
    Toml(#[from] TomlError),

    #[error("{provider} provider error: {source}")]
    Provider {
        provider: &'static str,
        #[source]
        source: BoxError,
    },

    #[error("configuration source {source}: {error}")]
    Source {
        source: ConfigSource,
        #[source]
        error: Box<ConfigError>,
    },

    #[error("{error}")]
    Path {
        path: String,
        #[source]
        error: Box<ConfigError>,
    },

    #[error("{error}")]
    Composition {
        provenance: ConfigProvenance,
        #[source]
        error: Box<ConfigError>,
    },

    #[error("Missing configuration field '{field}' required by view '{view}'")]
    MissingForView { view: &'static str, field: String },

    #[error("Missing required configuration field: '{0}'")]
    MissingField(FieldPath),

    #[error("Attempted to merge two frozen fields: '{0}'")]
    FreezeCollision(FieldPath),

    #[error("Validation failed for field '{field}': {reason}")]
    Validation {
        field: FieldPath,
        #[source]
        reason: BoxError,
    },

    #[error("Custom Merge failed for field '{field}': {reason}")]
    CustomMerge {
        field: FieldPath,
        #[source]
        reason: BoxError,
    },
}

#[cfg(feature = "json")]
impl From<serde_json::Error> for ConfigError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error.into())
    }
}

#[cfg(feature = "yaml")]
impl From<serde_saphyr::Error> for ConfigError {
    fn from(error: serde_saphyr::Error) -> Self {
        Self::Yaml(error.into())
    }
}

#[cfg(feature = "toml")]
impl From<::toml::de::Error> for ConfigError {
    fn from(error: ::toml::de::Error) -> Self {
        Self::Toml(error.into())
    }
}

impl ConfigError {
    /// Wrap an error raised by a configuration provider while preserving its source.
    pub fn provider(provider: &'static str, source: impl StdError + Send + Sync + 'static) -> Self {
        Self::Provider {
            provider,
            source: Box::new(source),
        }
    }

    #[cfg(feature = "key-value")]
    pub(crate) fn provider_at(
        provider: &'static str,
        path: impl Into<String>,
        source: impl StdError + Send + Sync + 'static,
    ) -> Self {
        Self::provider(provider, source).with_logical_path(path)
    }

    /// Construct a mode-specific requirement error for a logical dotted field path.
    pub fn missing_for_view<V>(field: impl Into<String>) -> Self {
        Self::MissingForView {
            view: ::core::any::type_name::<V>(),
            field: field.into(),
        }
    }

    /// Attach the external configuration source responsible for this error.
    pub fn with_source(self, source: ConfigSource) -> Self {
        if self.config_source() == Some(&source) {
            self
        } else {
            Self::Source {
                source,
                error: Box::new(self),
            }
        }
    }

    /// Attach a logical dotted configuration path to this error.
    ///
    /// This is primarily useful for custom providers that know the destination field associated
    /// with a conversion or lookup failure. The path is metadata only; it does not alter the
    /// displayed error text.
    pub fn with_logical_path(self, path: impl Into<String>) -> Self {
        let path = path.into();
        if self.logical_path().as_deref() == Some(path.as_str()) {
            self
        } else {
            Self::Path {
                path,
                error: Box::new(self),
            }
        }
    }

    fn with_provenance(self, provenance: ConfigProvenance) -> Self {
        match self {
            Self::Source { source, error } => Self::Source {
                source,
                error: Box::new(error.with_provenance(provenance)),
            },
            Self::Path { path, error } => Self::Path {
                path,
                error: Box::new(error.with_provenance(provenance)),
            },
            Self::Composition {
                provenance: inner,
                error,
            } => {
                let mut provenance = provenance;
                provenance.extend(inner);
                Self::Composition { provenance, error }
            }
            error => Self::Composition {
                provenance,
                error: Box::new(error),
            },
        }
    }

    /// Return the field associated with a build or merge error, if any.
    pub fn field_path(&self) -> Option<&FieldPath> {
        match self {
            Self::MissingField(field) | Self::FreezeCollision(field) => Some(field),
            Self::Validation { field, .. } | Self::CustomMerge { field, .. } => Some(field),
            Self::Source { error, .. }
            | Self::Path { error, .. }
            | Self::Composition { error, .. } => error.field_path(),
            _ => None,
        }
    }

    /// Return the safe structured-source location associated with this error, if any.
    ///
    /// This traverses source, path, and composition wrappers without exposing raw parser input.
    pub fn source_location(&self) -> Option<SourceLocation> {
        match self {
            #[cfg(feature = "json")]
            Self::Json(error) => error.location(),
            #[cfg(feature = "yaml")]
            Self::Yaml(error) => error.location(),
            #[cfg(feature = "toml")]
            Self::Toml(error) => error.location(),
            Self::Source { error, .. }
            | Self::Path { error, .. }
            | Self::Composition { error, .. } => error.source_location(),
            _ => None,
        }
    }

    /// Return the external configuration source attached to this error, if any.
    pub fn config_source(&self) -> Option<&ConfigSource> {
        match self {
            Self::Source { source, .. } => Some(source),
            Self::Path { error, .. } | Self::Composition { error, .. } => error.config_source(),
            _ => None,
        }
    }

    /// Return provenance retained by a failed multi-layer build, if any.
    pub fn provenance(&self) -> Option<&ConfigProvenance> {
        match self {
            Self::Composition { provenance, .. } => Some(provenance),
            Self::Source { error, .. } | Self::Path { error, .. } => error.provenance(),
            _ => None,
        }
    }

    /// Return the logical dotted field path associated with this error, if any.
    ///
    /// This also covers mode-specific [`ConfigView`] requirements, which use owned logical paths
    /// rather than [`FieldPath`].
    pub fn logical_path(&self) -> Option<String> {
        match self {
            Self::MissingForView { field, .. } | Self::Path { path: field, .. } => {
                Some(field.clone())
            }
            Self::Source { error, .. } | Self::Composition { error, .. } => error.logical_path(),
            _ => self.field_path().map(FieldPath::logical_path),
        }
    }

    /// Return the successfully merged source history for the field associated with this error.
    ///
    /// A provider that triggered a merge failure is not part of this slice because its layer was
    /// never merged. Use [`Self::field_sources`] when that attempted source should be included.
    pub fn field_provenance(&self) -> Option<&[ConfigSource]> {
        let path = self.logical_path()?;
        self.provenance()?.explain(path)
    }

    /// Return every source associated with the field that failed, in precedence order.
    ///
    /// This combines successfully merged provenance with an attached provider source. For merge
    /// failures that means the final entry is the provider whose attempted layer caused the
    /// failure, even though that layer was not committed to provenance.
    pub fn field_sources(&self) -> Vec<&ConfigSource> {
        if self.logical_path().is_none() {
            return Vec::new();
        }

        let mut sources = self
            .field_provenance()
            .map(|sources| sources.iter().collect::<Vec<_>>())
            .unwrap_or_default();
        if let Some(source) = self.config_source()
            && sources.last() != Some(&source)
        {
            sources.push(source);
        }
        sources
    }

    /// Return the highest-precedence source associated with the field that failed.
    pub fn latest_field_source(&self) -> Option<&ConfigSource> {
        self.logical_path()?;
        self.config_source()
            .or_else(|| self.field_provenance().and_then(|sources| sources.last()))
    }

    /// Return the underlying configuration error beneath any source context wrappers.
    pub fn root_cause(&self) -> &ConfigError {
        match self {
            Self::Source { error, .. }
            | Self::Path { error, .. }
            | Self::Composition { error, .. } => error.root_cause(),
            error => error,
        }
    }
}

/// Function-pointer shape for validators that borrow the exact field type.
///
/// The derive macro also accepts validators reached through normal Rust argument coercions, such
/// as `fn(&str)` for a `String` field. Validator errors must be convertible into [`BoxError`]. See
/// the derive macro for [`derive@Config`] for more details on `validate`.
pub type ValidationFunction<T, E> = for<'a> fn(&'a T) -> Result<(), E>;

/// A function passed to `#[config(merge ... )]` needs to match this signature.
///
/// The error type `E` may be any type convertible into [`BoxError`]. See the derive macro for
/// [`derive@Config`] for more details on `merge`.
pub type MergeFunction<T, E> = fn(T, T) -> Result<T, E>;

/// Wrapper for secret configuration values.
///
/// `Secret<T>` deserializes transparently, but deliberately does not implement [`Serialize`] and
/// redacts its [`Debug`](std::fmt::Debug) representation. Access to the wrapped value is explicit
/// through [`Secret::expose_secret`], and no mutable accessor is provided.
///
/// This protects common logging and accidental-serialization paths. Built-in TOML, YAML, and
/// dotenv providers suppress raw source lines in their normal parse diagnostics. Callers that
/// explicitly inspect backend parser errors, or custom providers that embed input values in their
/// own errors, remain responsible for handling that diagnostic data safely.
#[derive(Clone, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[serde(transparent)]
pub struct Secret<T>(T);

impl<T> Secret<T> {
    /// Wrap a secret value.
    pub fn new(value: T) -> Self {
        Self(value)
    }

    /// Borrow the secret value explicitly.
    pub fn expose_secret(&self) -> &T {
        &self.0
    }

    /// Consume the wrapper and return the secret value.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> From<T> for Secret<T> {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

impl<T: Default> Default for Secret<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T> std::fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret([REDACTED])")
    }
}

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
