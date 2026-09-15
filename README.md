# Einstellung

[![Crates.io](https://img.shields.io/crates/v/einstellung.svg)](https://crates.io/crates/einstellung)
[![Docs.rs](https://docs.rs/einstellung/badge.svg)](https://docs.rs/einstellung)
[![Build Status](https://img.shields.io/github/actions/workflow/status/soruh/einstellung/.github/workflows/rust.yml?branch=main)](https://github.com/soruh/einstellung/actions)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)

`einstellung` is a configuration parser for Rust based on `serde`. It allows you
to define your application's configuration in a flexible but ergonomic way using
strongly-typed structs.

By providing a `#[derive(Config)]` macro, `einstellung` automatically generates
the necessary boilerplate to parse, validate, and merge configurations from
multiple sources, including JSON, TOML, YAML, and hardcoded defaults, into a
single final config.

---

## Overview

- **Strongly Typed**: Define your configuration using standard Rust types.
- **Layered Configurations**: Merge configurations from multiple layers, such as
  hardcoded defaults, global files, and user-specific overrides.
- **Format Agnostic**: Flexible storage providers backed by `serde`. Built in
  support for JSON, TOML, YAML, selected environment variables, and `.env` files.
- **Granular Merging**: Choose to extend collections (like `HashSet` or `Vec`),
  replace fields entirely, or write custom merge logic.
- **Freezable Fields**: Lock specific configuration layers to prevent downstream
  overrides.
- **Validation**: Run custom validation logic on fields during the loading
  process to ensure data integrity.

---

## Installation

Upgrading from 0.1.x? See the [0.2.0 migration notes](https://github.com/soruh/einstellung/blob/main/CHANGELOG.md).

Add `einstellung` to your `Cargo.toml`:

```toml
[dependencies]
einstellung = "0.2.0"
```

### Feature Flags

You can customize enabled features to reduce compilation time or binary size:

- `derive` (default): Enables the `#[derive(Config)]` macro.
- `json` (default): Enables `JsonFileProvider`.
- `toml` (default): Enables `TomlFileProvider`.
- `yaml` (default): Enables `YamlFileProvider`, backed by the pure-Rust `serde-saphyr` parser (Rust 1.89+).
- `key-value`: Enables `KeyValueProvider` for dotted-path string overrides.
- `env`: Enables the allowlist-first `EnvProvider` and `key-value`.
- `dotenv`: Enables `DotenvProvider` and `env`. Dotenv files are parsed without
  mutating the process environment.
- `full`: Enables every provider and the derive macro.

### Rust version support

Rust requirements are feature-dependent rather than being raised globally by optional providers:

- The `einstellung` core (`--no-default-features`) supports Rust 1.85+, the minimum for edition 2024.
- The `derive` feature requires Rust 1.88+ because of its proc-macro dependencies.
- The `yaml` feature requires Rust 1.89+ because `serde-saphyr` declares that MSRV.
- The current default feature set includes both `derive` and `yaml`, so a default build requires Rust 1.89+.

The crate metadata records the core MSRV. CI checks the higher feature-specific floors separately.

---

## Examples

### Simple Configuration

Loading a complete configuration from a single YAML file.

```rust,no_run
use std::net::IpAddr;
use einstellung::{Config, YamlFileProvider};

#[derive(einstellung::serde::Deserialize, Debug)]
enum LogLevel { Error, Warning, Info, Debug, Trace }

#[derive(Config, Debug)]
struct AppConfig {
    app_name: String,

    #[config(default = LogLevel::Warning)]
    log_level: LogLevel,

    #[config(subconfig)]
    listen: ListenConfig,
}

#[derive(Config, Debug)]
struct ListenConfig {
    address: IpAddr,
    #[config(default = 443)]
    port: u16,
}

fn main() {
    let provider = YamlFileProvider::from_path(std::path::Path::new("config.yaml"));
    
    match AppConfig::load_complete(&provider) {
        Ok(config) => println!("Loaded config: {config:#?}"),
        Err(err) => eprintln!("Failed to load config: {err}"),
    }
}
```

### Layered & Frozen Configuration

Combining hardcoded defaults with external files while protecting specific
fields.

```rust,no_run
use einstellung::{Config, ConfigError, ConfigProvider, Freezable, JsonFileProvider, PartialConfig, TomlFileProvider};

#[derive(Config, Debug)]
struct AppConfig {
    app_name: String,
    
    #[config(merge = "extend")]
    users: std::collections::HashSet<String>,

    #[config(freezable)]
    max_open_files: usize,
}

fn load_config() -> Result<AppConfig, ConfigError> {
    const DEFAULTS: &str = r#"{ "app_name": "MyApp", "users": ["root"], "max_open_files": 100 }"#;

    // Load defaults and "freeze" them to protect `max_open_files` from later changes
    let base_layer = AppConfig::load_partial(&JsonFileProvider::from_contents(DEFAULTS))?.freeze();
    
    // Load an external override
    let user_layer = TomlFileProvider::from_path(std::path::Path::new("config.toml"))
        .load_partial()?;

    base_layer
        .merge(user_layer)?
        .build()
}
```

---

### Environment and `.env` overlays

Environment providers are opt-in and load nothing unless a mapping or prefix is
configured. This makes it possible to reserve `.env` for secrets and
machine-local paths without accidentally importing unrelated settings.

```rust,no_run
use einstellung::{Config, DotenvProvider, EnvProvider, TomlFileProvider};

#[derive(Config)]
struct AppConfig {
    api_key: String,
    source_path: String,
    model: String,
}

fn load_config() -> Result<AppConfig, einstellung::ConfigError> {
    let local = EnvProvider::only(["API_KEY", "SOURCE_PATH"]);

    AppConfig::builder()
        .provider(&TomlFileProvider::from_path(std::path::Path::new("config.toml")))
        .provider(
            &DotenvProvider::from_path(std::path::Path::new(".env"))
                .with_env_provider(local.clone()),
        )
        .provider(&local)
        .build()
}
```

Providers are merged left-to-right by `Config::builder()`, so the selected process
environment variables override the selected `.env` values, while portable
settings such as `model` continue to come from the shared TOML file. A missing
field in a later layer does not clear an earlier value; the later layer must
actually contain a value to replace it.

`Config::builder()` is the recommended composition API when loading several
sources. Each `.provider(...)` is the next higher-precedence layer. Field-level
`#[config(default = ...)]` defaults are applied only once, when `.build()`
constructs the final configuration. Use `.layer(...)` when a layer has already
been loaded or transformed (for example, frozen).

When several providers would otherwise have the same generic source label (for
example, multiple inline JSON layers), use `.provider_named("base defaults",
&provider)` or `.typed_provider_named(...)` to give that composition step a
non-secret diagnostic/provenance identity.

`EnvProvider` and `DotenvProvider` deliberately have no "load everything"
default. `EnvProvider::only(...)` is convenient when environment names map
directly to lowercase field names; `EnvProvider::prefixed(...)` additionally
supports `__` for nested fields. Explicit-only process-environment mappings query
just the configured variable names rather than enumerating unrelated environment
values; prefix mode necessarily enumerates the environment to discover matching
names. Treat these selections as a trust boundary: explicitly expose only the
secrets and machine-local values that should enter the typed configuration. Dotenv selection limits which final keys are loaded, but standard dotenv
substitution can still read process-environment variables while evaluating a selected value. Use
`DotenvProvider::without_substitution()` when a local `.env` file must be isolated from process
environment expansion; escaped and single-quoted dollar signs remain literal.

For CLI flags, secret stores, or other already-selected string key/value inputs,
`KeyValueProvider` accepts dotted logical paths directly and uses the same typed
decoding as environment providers. Give it a non-secret source label when
provenance matters, for example `KeyValueProvider::named("CLI overrides")`.

For numeric, boolean, or collection fields behind Serde flattening or untagged
enums, explicitly encode selected values as JSON with `KeyValueProvider::with_json`
or `EnvProvider::with_json_var` / `DotenvProvider::with_json_var`:

```rust,no_run
use einstellung::{Config, KeyValueProvider};

#[derive(Config)]
struct Listen {
    port: u16,
    label: String,
}

#[derive(Config)]
struct App {
    #[config(subconfig)]
    #[config(serde(flatten))]
    listen: Option<Listen>,
}

let config = App::load_complete(
    &KeyValueProvider::new()
        .with_json("port", "8080")
        .with("label", "123"),
)?;
assert_eq!(config.listen.unwrap().port, 8080);
# Ok::<(), einstellung::ConfigError>(())
```

Ordinary string mappings parse scalars when Serde requests a concrete target
type. Flattening and untagged enums buffer values before that type is known,
so ordinary mapped values remain strings there. An untagged numeric-or-string
enum will therefore select its string variant for an ordinary `"8080"` mapping;
`with_json("value", "8080")` supplies a number. JSON-valued mappings require valid
JSON, including quotes around JSON strings. Dotenv quoting is processed first:
for example, `LABEL='"123"'` preserves the quotes needed by a JSON string mapping.
Non-unit enums can likewise use an explicit JSON representation. Invalid supplied
flattened values produce an error, including when the subconfig is optional.

For secret-bearing fields, use `Secret<T>`. It deserializes transparently, redacts
its `Debug` and `Display` representations, deliberately does not implement
`Serialize`, and requires an explicit `expose_secret()` call to borrow the value. Keep secret
fields private as well if the complete config should not allow replacement after
construction. `#[config(freezable)]` can prevent a value from being overwritten
by later configuration layers, but it is a merge policy rather than a secrecy
mechanism.

When a JSON/TOML/YAML format is chosen at runtime, use `FormatProvider`. For a
borrowed filesystem path, `FormatProvider::from_path_detect` recognizes enabled `json`,
`toml`, `yaml`, and `yml` extensions; `from_path_buf_detect` provides the owned equivalent.
`from_owned_contents` and `from_path_buf` mirror the explicit owned constructors on the
format-specific providers.

`ConfigProvider::load_partial_with_source(...)` is available when using a provider directly
and source-aware diagnostics are desired without going through `Config::load_partial(...)` or a
builder. The object-safe adapter provides the equivalent
`ConfigProviderFor::load_config_partial_with_source(...)`.

When the *provider set* itself is chosen at runtime, use `ConfigProviderFor<C>`.
It is an object-safe adapter for one concrete config type, automatically implemented
by every `ConfigProvider`, so heterogeneous providers can be stored as
`Box<dyn ConfigProviderFor<AppConfig>>` and fed to
`ConfigBuilder::typed_provider(...)`. This avoids an application-level provider enum
without weakening the generic `ConfigProvider` API.

For user-edited structured files, `#[config(deny_unknown_fields)]` opts a config
type into strict key checking so misspellings fail instead of silently falling
back to defaults. The policy is per config type; nested subconfigs opt in
independently. Environment providers remain allowlist-driven before
deserialization, so unrelated process variables are never treated as config
keys.

### Provenance and diagnostics

Use `build_tracked()` when you need to explain where a final setting came from:

```rust,no_run
# use einstellung::{Config, EnvProvider, TomlFileProvider};
# #[derive(Config)]
# struct AppConfig { api_key: String }
# fn main() -> Result<(), einstellung::ConfigError> {
let tracked = AppConfig::builder()
    .provider(&TomlFileProvider::from_path(std::path::Path::new("config.toml")))
    .provider(&EnvProvider::only(["API_KEY"]))
    .build_tracked()?;

for source in tracked.explain("api_key").unwrap_or_default() {
    eprintln!("api_key was supplied by {source}");
}

for (path, sources) in tracked.provenance().iter() {
    eprintln!("{path}: {} source(s)", sources.len());
}
# Ok(())
# }
```

Provenance stores only logical field paths and source labels, never configuration
values. Sources are listed in merge order. For normal replacement the last
supplier is the winner; `extend`, custom merge functions, and frozen fields can
retain data from earlier layers, so `explain()` deliberately preserves the full
supply history rather than pretending there is always one winner. Values filled
by `#[config(default ...)]` are attributed to `field default`.

`provenance().layers()` separately lists every successfully merged configuration
layer, including empty layers that supplied no concrete field values.
`explain_nearest(path)` first looks for the exact field, then its nearest supplied
subconfig ancestor, and finally the merged layer history. This lets missing-field
diagnostics still answer which sources were considered when the missing leaf was
never supplied by any layer.

Builder errors retain the provenance accumulated before the failure, including
`build_partial()` failures. Use `build_tracked_partial()` when a successfully
composed partial must keep provenance for later transformation or merging. Feed that
result into another builder with `tracked_layer(...)` to preserve its original
per-field source histories instead of collapsing the staged partial to one synthetic
layer label. Nested/composite provider errors likewise merge their internal provenance
with the caller's earlier layers in precedence order.

This is particularly useful for final validation errors, where there is no single
parser failure to identify the source directly:

```rust,no_run
# use einstellung::{Config, EnvProvider, TomlFileProvider};
# #[derive(Config)]
# struct AppConfig { port: u16 }
# fn main() -> Result<(), einstellung::ConfigError> {
match AppConfig::builder()
    .provider(&TomlFileProvider::from_path(std::path::Path::new("config.toml")))
    .provider(&EnvProvider::only(["PORT"]))
    .build()
{
    Ok(config) => use_config(config),
    Err(error) => {
        if let Some(path) = error.logical_path() {
            eprintln!("{path} was associated with: {:?}", error.field_sources());
        }
        return Err(error);
    }
}
# Ok(())
# }
# fn use_config<T>(_config: T) {}
```

`ConfigError::root_cause()` unwraps source/provenance context when code needs to
inspect the concrete error variant. `config_source()` reports the external source
that triggered a load/merge error, `logical_path()` returns a dotted field path,
and `field_sources()` combines successfully merged provenance with the provider
whose attempted layer caused a field-specific failure.

JSON, TOML, and YAML providers retain deserialization paths through
`serde_path_to_error`, including nested fields, map keys, and collection indices
(for example, `servers.0.port`). These paths participate in provenance lookup and
honor Serde's input field names. Errors without a known destination, such as
document-level syntax errors, may have no logical path.
Serde-buffered representations such as flattened subconfigs and untagged enums
can also lose the exact field path during deserialization. Their errors still
propagate with provider context; an unknown path is not reported as an empty string.

Custom providers that know which destination field failed can attach the same
metadata with `ConfigError::with_logical_path("model.remote.api_url")`. Path context
is transparent in the displayed error but is available through `logical_path()` and
participates in the same source/provenance accessors as derive-generated errors.

Provider parse errors and single-provider build errors include the provider
source. File providers identify the path; inline providers identify only the
format and never embed their contents in diagnostics. Built-in TOML, YAML, and
dotenv parse errors also avoid rendering raw configuration lines by default,
which prevents nearby secrets from leaking through ordinary error logging.
`ConfigError::source_location()` exposes safe structured-parser line/column metadata
without exposing source text. `JsonError::parser_error()`, `TomlError::parser_error()`,
and `YamlError::parser_error()` provide explicit access to backend diagnostics when
detailed parser output is intentionally needed.

### Mode-specific configuration views

Keep settings that are not required by every command optional in the shared
configuration, then convert to a stricter typed view for modes that require them.
For example, a validation command can build `AppConfig` with `remote: None`, while
an execution command uses `build_view::<RemoteMode>()` and rejects a missing
remote section:

```rust,no_run
use einstellung::{Config, ConfigError, ConfigView, require_for_view};

#[derive(Config)]
struct AppConfig {
    #[config(subconfig)]
    remote: Option<RemoteConfig>,
}

#[derive(Config)]
struct RemoteConfig {
    api_url: String,
}

struct RemoteMode {
    remote: RemoteConfig,
}

impl ConfigView<AppConfig> for RemoteMode {
    fn from_config(config: AppConfig) -> Result<Self, ConfigError> {
        let remote = require_for_view::<Self, _>(config.remote, "remote")?;
        Ok(Self { remote })
    }
}
```

`build_tracked_view()` performs the same conversion while retaining source
provenance. Views operate after normal defaults and validators, so they add
mode-specific requirements without changing the shared merge semantics.

---

## Layering Features

The flexibility of `einstellung` comes from its partial configuration system.
When you derive `Config`, the macro generates a companion "Partial" struct where
all fields are optional.

- `.merge()`: Combines two partial configurations. By default, values in the
  "newer" layer overwrite the "older" layer.
- `merge = "extend"`: Instead of overwriting, this strategy uses the `Extend`
  trait to combine collections like `Vec` or `BTreeSet`.
- `.freeze()`: Marks a partial configuration as frozen. Any fields tagged with
  `#[config(freezable)]` in a frozen layer cannot be modified by subsequent
  merges.

### Optional subconfigs and defaults

An `Option<SubConfig>` marked with `#[config(subconfig)]` stays `None` when no
layer mentions that subconfig. Once any layer supplies the subconfig, its partial
value is built normally: required nested fields must be present and nested
defaults are applied. This distinction is useful for mode-specific sections:
absence means “feature not configured,” while partial presence means “feature
configured, so validate it completely.”

Serde-flattened subconfigs are also supported with
`#[config(subconfig)] #[config(serde(flatten))]`. Their nested keys remain flat in
diagnostics and provenance: a nested `api_key` is tracked as `api_key`, not
`credentials.api_key`. Optional flattened subconfigs stay `None` when none of
their nested keys are present. An outer `#[config(default)]` is intentionally
rejected on flattened subconfigs because Serde represents an absent flattened
object as an empty partial, making outer-field absence ambiguous; put defaults
on the nested fields instead.
Required flattened subconfigs also apply nested defaults when built without any
provider. Optional flattened subconfigs that remain absent contribute no field
defaults or field provenance.

Field defaults are a final construction fallback, not an implicit merge layer.
Providers are merged first; only then does `.build()` fill still-missing fields
from `#[config(default ...)]` and run validators. This is why a later provider
that omits a field never erases an earlier value and never forces its default.

---

## Customizability

- **Validation**: Use `#[config(validate = path::to::func)]` or a closure expression to
  ensure fields meet specific criteria before the final config is built. Normal reference
  coercions apply, so `String` fields can use idiomatic `fn(&str)` validators. The
  dependency-free `einstellung::validators` module includes `non_empty`, `non_blank`, and
  `non_empty_slice`; closures are convenient for parameterized checks such as numeric ranges.
- **Custom Merging**: Implement custom merge logic via
  `#[config(merge(function = "path"))]`.
- **Attribute Forwarding**: Attributes like `#[config(partial(...))]` are
  forwarded to the generated partial struct. There is a shorthand syntax
  `#[config(serde(...))]` which is interpreted as
  `#[config(partial(serde(...)))]`. Serde deserialization `rename`/`rename_all`
  names also become the canonical logical paths used by errors and provenance.
- **Subconfigs**: Nest `Config` structs using the `#[config(subconfig)]`
  attribute to keep your data organized.
- **Generic structs**: `#[derive(Config)]` currently requires a non-generic struct.
  Generic derives are rejected explicitly rather than emitting invalid generated Rust;
  supporting them correctly requires propagating deserialization and subconfig bounds.

---

## Documentation

- **Main Crate Documentation**: Visit the
  [einstellung docs](https://docs.rs/einstellung) for detailed information on
  the `Config`, `PartialConfig`, and `ConfigProvider` traits. See the
  documentation of the `Config` derive macro for full documentation on supported
  attributes.

---

## Contributing

Please feel free to open an Issue or submit a PR at
[https://github.com/soruh/einstellung](https://github.com/soruh/einstellung).

Before publishing, run `scripts/verify-packages.sh` from a clean checkout. For a
local pre-commit check, pass `--allow-dirty`. It verifies the packaged
`einstellung_derive` artifact first, then verifies the packaged `einstellung` crate
against that exact derive artifact rather than an older registry release.
It also compiles README doctests and runs both examples from the extracted archive.

The root `README.md` is the documentation source; the crate READMEs link to it.
All Rust snippets are compiled by
`cargo test --workspace --all-features --doc`. Package verification also checks
that the README and license copies match their workspace originals.

Publish `einstellung_derive` first, wait until it is available from crates.io, then
publish `einstellung`. Tag the verified release commit as `v0.2.0`.

(This includes confusing/incorrect documentation, bad error messages and missing
features)
