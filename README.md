# Einstellung

[![Crates.io](https://img.shields.io/crates/v/einstellung.svg)](https://crates.io/crates/einstellung)
[![Docs.rs](https://docs.rs/einstellung/badge.svg)](https://docs.rs/einstellung)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)

**Einstellung** is a flexible configuration parser for Rust based on `serde`.
[cite_start]It allows you to define your application's configuration securely
and ergonomically using strongly-typed structs[cite: 1].

[cite_start]By providing a `#[derive(Config)]` macro, **einstellung**
automatically generates the necessary boilerplate to parse, validate, and merge
configurations from multiple sources—including JSON, TOML, YAML, and hardcoded
defaults—into a single, cohesive application state[cite: 1].

---

## Overview

- [cite_start]**Strongly Typed**: Define your configuration using standard Rust
  structs and enums[cite: 1].
- **Layered Configurations**: Merge configurations from multiple layers, such as
  hardcoded defaults, global files, and user-specific overrides.
- **Format Agnostic**: Flexible storage providers backed by `serde`. Built in
  support for JSON, TOML, and YAML.
- [cite_start]**Granular Merging**: Choose to extend collections (like `HashSet`
  or `Vec`), replace fields entirely, or write custom merge logic[cite: 1].
- **Freezable Fields**: Lock specific configuration layers to prevent downstream
  overrides.
- **Validation**: Run custom validation logic on fields during the build process
  to ensure data integrity.

---

## Installation

Add **einstellung** to your `Cargo.toml`:

```toml
[dependencies]
einstellung = "0.1.0"
```

### Feature Flags

You can customize enabled features to reduce compilation time or binary size:

- **`derive`** (default): Enables the `#[derive(Config)]` macro.
- **`json`** (default): Enables `JsonFileProvider`.
- **`toml`** (default): Enables `TomlFileProvider`.
- **`yaml`** (default): Enables `YamlFileProvider`.
- **`full`** (default): Enables all format providers and the derive macro.

---

## Examples

### Simple Configuration

[cite_start]Loading a complete configuration from a single YAML file[cite: 1].

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

The core power of **einstellung** lies in its partial configuration system. When
you derive `Config`, the macro generates a companion "Partial" struct where all
fields are optional.

- **`.merge()`**: Combines two partial configurations. By default, values in the
  "newer" layer overwrite the "older" layer.
- [cite_start]**`merge = "extend"`**: Instead of overwriting, this strategy uses
  the `Extend` trait to combine collections like `Vec` or `BTreeSet`[cite: 1].
- **`.freeze()`**: Marks a partial configuration as frozen. Any fields tagged
  with `#[config(freezable)]` in a frozen layer cannot be modified by subsequent
  merges.

---

## Customizability

- **Validation**: Use `#[config(validate = path::to::func)]` to ensure fields
  meet specific criteria before the final config is built.
- **Custom Merging**: Implement custom merge logic via
  `#[config(merge(function = "path"))]`.
- **Serde Forwarding**: Attributes like `#[config(serde(rename = "..."))]` or
  `alias` are forwarded to the generated partial structs to maintain consistent
  naming across formats.
- [cite_start]**Subconfigs**: Nest `Config` structs using the
  `#[config(subconfig)]` attribute to keep your data organized[cite: 1].

---

## Documentation

- **Main Crate Documentation**: Visit the
  [einstellung docs](https://docs.rs/einstellung) for detailed information on
  the `Config`, `PartialConfig`, and `ConfigProvider` traits.
- **Derive Macro Reference**: See the
  [einstellung_derive docs](https://docs.rs/einstellung_derive) for a full list
  of supported `#[config(...)]` attributes.

---

## Contributing

Please feel free to open an Issue or submit a PR at
[https://github.com/soruh/einstellung](https://github.com/soruh/einstellung).
