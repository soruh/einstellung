# Einstellung

[![Crates.io](https://img.shields.io/crates/v/einstellung.svg)](https://crates.io/crates/einstellung)
[![Docs.rs](https://docs.rs/einstellung/badge.svg)](https://docs.rs/einstellung)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)

`einstellung` is a configuration parser for Rust based on `serde`. It
allows you to define your application's configuration in a flexible but ergonomic way using strongly-typed structs.

By providing a `#[derive(Config)]` macro, `einstellung` automatically
generates the necessary boilerplate to parse, validate, and merge configurations
from multiple sources, including JSON, TOML, YAML, and hardcoded defaults, into a
single final config.

---

## Overview

- **Strongly Typed**: Define your configuration using standard Rust types.
- **Layered Configurations**: Merge configurations from multiple layers, such as
  hardcoded defaults, global files, and user-specific overrides.
- **Format Agnostic**: Flexible storage providers backed by `serde`. Built in
  support for JSON, TOML, and YAML.
- **Granular Merging**: Choose to extend collections (like `HashSet` or `Vec`),
  replace fields entirely, or write custom merge logic.
- **Freezable Fields**: Lock specific configuration layers to prevent downstream
  overrides.
- **Validation**: Run custom validation logic on fields during the loading process
  to ensure data integrity.

---

## Installation

Add `einstellung` to your `Cargo.toml`:

```toml
[dependencies]
einstellung = "0.1.5"
```

### Feature Flags

You can customize enabled features to reduce compilation time or binary size:

- `derive` (default): Enables the `#[derive(Config)]` macro.
- `json` (default): Enables `JsonFileProvider`.
- `toml` (default): Enables `TomlFileProvider`.
- `yaml` (default): Enables `YamlFileProvider`.
- `full` (default): Enables all format providers and the derive macro.

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
    let provider = YamlFileProvider::new("config.yaml");
    
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
    let base_layer = AppConfig::load_partial(&JsonFileProvider::new(DEFAULTS))?.freeze();
    
    // Load an external override
    let user_layer = TomlFileProvider::new("config.toml").load_partial()?;

    base_layer
        .merge(user_layer)?
        .build()
}
```

---

## Layering Features

The flexibility of `einstellung` comes from its partial configuration system. When
you derive `Config`, the macro generates a companion "Partial" struct where all
fields are optional.

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
  meet specific criteria before the final config is built.
- **Custom Merging**: Implement custom merge logic via
  `#[config(merge(function = "path"))]`.
- **Attribute Forwarding**: Attributes like `#[config(partial(...))]` are forwarded to the generated partial struct.
  There is a shorthand syntax `#[config(serde(...))]` which is interpreted as `#[config(partial(serde(...)))]`
- **Subconfigs**: Nest `Config` structs using the `#[config(subconfig)]`
  attribute to keep your data organized.

---

## Documentation

- **Main Crate Documentation**: Visit the
  [einstellung docs](https://docs.rs/einstellung) for detailed information on
  the `Config`, `PartialConfig`, and `ConfigProvider` traits.
  See the documentation of the `Config` derive macro for full documentation on supported attributes.

---

## Contributing

Please feel free to open an Issue or submit a PR at
[https://github.com/soruh/einstellung](https://github.com/soruh/einstellung).
