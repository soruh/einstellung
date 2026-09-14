# Einstellung

[![Crates.io](https://img.shields.io/crates/v/einstellung.svg)](https://crates.io/crates/einstellung)
[![Docs.rs](https://docs.rs/einstellung/badge.svg)](https://docs.rs/einstellung)
[![Build Status](https://img.shields.io/github/actions/workflow/status/soruh/einstellung/.github/workflows/rust.yml?branch=main)](https://github.com/soruh/einstellung/actions)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)

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

Add `einstellung` to your `Cargo.toml`:

```toml
[dependencies]
einstellung = "0.1.6"
```

### Feature Flags

You can customize enabled features to reduce compilation time or binary size:

- `derive` (default): Enables the `#[derive(Config)]` macro.
- `json` (default): Enables `JsonFileProvider`.
- `toml` (default): Enables `TomlFileProvider`.
- `yaml` (default): Enables `YamlFileProvider`, backed by the maintained `serde_yaml_ng` fork.
- `env`: Enables the allowlist-first `EnvProvider`.
- `dotenv`: Enables `DotenvProvider` and `env`. Dotenv files are parsed without
  mutating the process environment.
- `full`: Enables every provider and the derive macro.

---

## Examples

### Simple Configuration

Loading a complete configuration from a single YAML file.

```rust
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

```rust
use einstellung::{Config, ConfigError, Freezable, JsonFileProvider, PartialConfig, TomlFileProvider};

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

```rust
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

`EnvProvider` and `DotenvProvider` deliberately have no "load everything"
default. `EnvProvider::only(...)` is convenient when environment names map
directly to lowercase field names; `EnvProvider::prefixed(...)` additionally
supports `__` for nested fields. Treat these selections as a trust boundary:
explicitly expose only the secrets and machine-local values that should enter
the typed configuration.
For secret-bearing fields, use `Secret<T>`. It deserializes transparently, redacts
its `Debug` representation, deliberately does not implement `Serialize`, and
requires an explicit `expose_secret()` call to borrow the value. Keep secret
fields private as well if the complete config should not allow replacement after
construction. `#[config(freezable)]` can prevent a value from being overwritten
by later configuration layers, but it is a merge policy rather than a secrecy
mechanism.

When a JSON/TOML/YAML format is chosen at runtime, use `FormatProvider`. For a
filesystem path, `FormatProvider::from_path_detect` recognizes enabled `json`,
`toml`, `yaml`, and `yml` extensions, avoiding an application-side format match.

For user-edited structured files, `#[config(deny_unknown_fields)]` opts a config
type into strict key checking so misspellings fail instead of silently falling
back to defaults. The policy is per config type; nested subconfigs opt in
independently. Environment providers remain allowlist-driven before
deserialization, so unrelated process variables are never treated as config
keys.

### Mode-specific configuration views

Keep settings that are not required by every command optional in the shared
configuration, then convert to a stricter typed view for modes that require them.
For example, a validation command can build `AppConfig` with `remote: None`, while
an execution command uses `build_view::<RemoteMode>()` and rejects a missing
remote section:

```rust
use einstellung::{Config, ConfigError, ConfigView};

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
        let remote = config
            .remote
            .ok_or_else(|| ConfigError::missing_for_view::<Self>("remote"))?;
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

---

## Customizability

- **Validation**: Use `#[config(validate = path::to::func)]` to ensure fields
  meet specific criteria before the final config is built. Normal reference
  coercions apply, so `String` fields can use idiomatic `fn(&str)` validators.
- **Custom Merging**: Implement custom merge logic via
  `#[config(merge(function = "path"))]`.
- **Attribute Forwarding**: Attributes like `#[config(partial(...))]` are
  forwarded to the generated partial struct. There is a shorthand syntax
  `#[config(serde(...))]` which is interpreted as
  `#[config(partial(serde(...)))]`
- **Subconfigs**: Nest `Config` structs using the `#[config(subconfig)]`
  attribute to keep your data organized.

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

(This includes confusing/incorrect documentation, bad error messages and missing
features)
