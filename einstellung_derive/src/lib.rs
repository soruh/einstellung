use proc_macro::TokenStream;

#[cfg(test)]
mod test;

mod derive_config;

/// Derives the `Config` trait for a struct and generates a companion `Partial` struct.
///
/// `einstellung` uses a layered configuration pattern. Instead of loading an entire
/// configuration at once, this macro generates a partial representation of your struct
/// where all fields are optional. This allows you to load fragments of configuration
/// from multiple sources (like hardcoded defaults, JSON, YAML, or TOML), merge them
/// together, and finally build the fully populated configuration struct.
///
/// # The Generated `Partial` Type
///
/// When you derive `Config` on a struct named `AppConfig`, the macro generates a
/// companion struct named `AppConfigPartial`.
///
/// The generated partial type:
/// * Wraps the fields of the complete type to make them optional
/// * Implements `Default`, `serde::Deserialize`, and `einstellung::PartialConfig`.
/// * Inherits all `#[config(partial(...))]` attributes as `#[...]`
/// * Is also accessable as `<AppConfig as Config>::Partial`
/// * References the complete type as `<AppConfigPartial as PartialConfig>::Complete`
///
/// If any field (or the struct) is marked as `freezable`, the partial struct will also
/// implement the `einstellung::Freezable` trait, allowing layers to be locked against
/// downstream mutations.
///
/// # Struct Attributes
///
/// Attributes applied to the struct itself via `#[config(...)]`.
///
/// * `#[config(freezable)]`
///   Marks *all* fields within the struct as freezable. A frozen configuration layer
///   prevents subsequent merged layers from overwriting these values.
///   
/// * `#[config(partial(...))]`
///   Forwards attributes directly to the generated `Partial` struct. This is primarily
///   useful for adding common derives to the partial struct.
///   *Example:* `#[config(partial(derive(Clone, Debug)))]`
///
/// * `#[config(crate = "path::to::einstellung")]`
///   Overrides the path to the `einstellung` crate. Useful if you are re-exporting the
///   crate or using it from within a workspace where the name might differ.
///
/// # Field Attributes
///
/// Attributes applied to individual fields via `#[config(...)]`.
///
/// ### Fallback & Defaults (`default`)
/// Determines what happens during the `.build()` phase if a field is still missing
/// after all layers have been merged. If no default attribute is specified, the field is
/// **required** and will return a `ConfigError::MissingField` if left unpopulated
/// (unless the base type is an `Option<T>`, in which case it simply defaults to `None`).
///
/// * `#[config(default)]`
///   Falls back to `Default::default()` for the field's type.
/// * `#[config(default = value)]`
///   Falls back to a specific value or expression (e.g., `#[config(default = 8080)]` or
///   `#[config(default = LogLevel::Info)]`).
/// * `#[config(default = path::to::function())]` or `#[config(default = || "localhost".to_string())]`
///   Calls a function or closure to dynamically generate the default value at runtime.
///   Note that non-closure functions need to be called with zero arguments to distinguish them from enum variants
///
/// ### Sub-configurations (`subconfig`)
/// * `#[config(subconfig)]`
///   Marks a field as a nested configuration struct that also derives `Config`. Instead
///   of treating the field as an opaque `Option<T>`, the generated partial struct will
///   treat it as an `Option<T::Partial>`. When merged, both partial subconfigs will be
///   recursively merged. *(Note: Merge strategies cannot be applied to a subconfig).*
///
/// ### Merging Strategies (`merge`)
/// Defines how values from a newer configuration layer interact with values from an
/// older layer.
///
/// * `#[config(merge = "replace")]` *(Default)*
///   If the newer layer has a `Some(value)`, it replaces the older layer's value.
/// * `#[config(merge = "extend")]`
///   Instead of replacing, values are combined using the standard library's `Extend`
///   trait. This can be used to join collections like `Vec`, `HashSet`, or `HashMap`.
/// * `#[config(merge(function = "path::to::function"))]`
///   Defines a custom merge function. The function must conform to the signature:
///   `fn(Option<T>, Option<T>) -> Result<Option<T>, E>`. The error will be mapped to a
///   `ConfigError::CustomMerge`.
///
/// ### Data Integrity (`validate`, `freezable`)
/// * `#[config(validate = path::to::function)]`
///   Runs a custom validation function on the final, fully-merged value during the
///   `.build()` phase. The function must conform to the signature `fn(&T) -> Result<(), E>`.
///   If it returns an `Err`, `.build()` halts and returns a `ConfigError::Validation`.
/// * If you want a custom validation function for every instance of a
///   given type it may be more practical to write a custom `serde::Deserialze` implementation.
/// * `#[config(freezable)]`
///   Marks an individual field as freezable. If a partial layer is `.freeze()`d, this field
///   will reject overwrite attempts from subsequent layers always keeping the frozen value.
///   If two frozen configs are attempted to be merged, a `ConfigError::FreezeCollision` will be returned instead
///
/// ### Serde Forwarding (`serde`)
/// * `#[config(serde(...))]`
///   Because the `Partial` struct drives the actual parsing of files (JSON, TOML, YAML),
///   standard `#[serde(...)]` tags on the main struct won't work out of the box. Use
///   this attribute to forward serde rules to the underlying partial field.
///   *Example:* `#[config(serde(rename = "server_port", alias = "port"))]`
///
/// # Example
///
/// ```rust
/// use einstellung::Config;
///
/// #[derive(Config, Debug)]
/// #[config(partial(derive(Clone)))]
/// pub struct ServerConfig {
///     // Required field: will fail at .build() if not provided by any layer.
///     pub host: String,
///
///     // Falls back to 8080 if not provided. Forwarded serde rename.
///     #[config(default = 8080)]
///     #[config(serde(rename = "server_port"))]
///     pub port: u16,
///     
///     // Protects default admins from being overwritten by user configs.
///     // Combines arrays rather than overwriting them.
///     #[config(freezable, merge = "extend")]
///     pub admins: Vec<String>,
///
///     // Runs a custom validation function on the final struct.
///     #[config(validate = validate_timeout)]
///     pub timeout_ms: u32,
///
///     // Recursively merges nested `Config` structs.
///     #[config(subconfig)]
///     pub tls: TlsConfig,
/// }
///
/// fn validate_timeout(val: &u32) -> Result<(), &'static str> {
///     if *val < 100 { Err("Timeout must be at least 100ms") } else { Ok(()) }
/// }
/// ```
#[proc_macro_derive(Config, attributes(config))]
pub fn derive_config(input: TokenStream) -> TokenStream {
    derive_config::derive(input.into()).into()
}
