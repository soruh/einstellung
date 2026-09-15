//! Combine file layers, nested defaults, and a frozen field.

#![allow(unused)]

use std::{
    collections::HashSet,
    default,
    net::IpAddr,
    path::{Path, PathBuf},
};

use einstellung::{
    Config, ConfigError, ConfigProvider, Freezable, JsonFileProvider, PartialConfig,
    TomlFileProvider, YamlFileProvider,
};
use serde::{Deserialize, Serialize};

#[derive(einstellung::Config, Debug)]
/// Example of extending an ordered set across layers.
struct UserConfig2 {
    #[config(merge = "extend")]
    /// Users accumulated from the supplied configuration layers.
    users: std::collections::BTreeSet<String>,
}

#[derive(einstellung::serde::Deserialize, Debug)]
/// Logging verbosity accepted by the example configuration.
enum LogLevel {
    /// Report failures only.
    Error,
    /// Report warnings and failures.
    Warning,
    /// Include normal operational messages.
    Info,
    /// Include diagnostic messages.
    Debug,
    /// Include detailed execution traces.
    Trace,
}

#[derive(einstellung::Config, Debug)]
/// Complete application configuration assembled by the example.
struct AppConfig {
    /// Application name read from the file.
    app_name: String,

    #[config(default = LogLevel::Warning)]
    /// Logging verbosity, with a fallback when omitted.
    log_level: LogLevel,

    #[config(subconfig)]
    /// Nested socket address and port settings.
    listen: ListenConfig,

    #[config(subconfig)]
    /// Color settings demonstrating nested configuration construction.
    colors: ColorConfig,

    #[config(merge = "extend")]
    /// Users accumulated from the supplied configuration layers.
    users: HashSet<String>,

    #[config(freezable)]
    /// File limit demonstrating optional or frozen field behavior.
    max_open_files: usize,
}

#[derive(einstellung::Config, Debug)]
/// Color settings read from the configuration.
struct ColorConfig {
    /// Primary color in the application’s chosen text format.
    primary: String,
    #[config(default = || "#0ff".to_string())]
    /// Secondary color in the application’s chosen text format.
    secondary: String,
}

#[derive(einstellung::Config, Debug)]
/// Address and port on which the example application would listen.
struct ListenConfig {
    /// IP address read from the configuration.
    address: IpAddr,
    #[config(default = 443)]
    /// Listening port, defaulting to 443.
    port: u16,
}

/// Locate example fixtures in either a workspace or a packaged crate.
fn config_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("examples")
}

/// Combine defaults and file layers while preserving the frozen file limit.
///
/// # Errors
///
/// Returns an error if a fixture cannot be loaded, a merge fails, or final
/// configuration construction fails.
fn load_config(dir: &Path) -> Result<AppConfig, ConfigError> {
    const LISTEN_CONFIG: &str = r#"{ "address": "127.0.0.1" }"#;
    const HARD_CODED_CONFIG: &str = r#"{ "users": ["root"], "max_open_files": 10 }"#;

    let hard_coded = AppConfig::load_partial(&JsonFileProvider::from_contents(HARD_CODED_CONFIG))?;
    let user_config1 = YamlFileProvider::from_path_buf(dir.join("config.yaml")).load_partial()?;
    let user_config2 = TomlFileProvider::from_path_buf(dir.join("config.toml")).load_partial()?;
    let listen_config =
        ListenConfig::load_partial(&JsonFileProvider::from_contents(LISTEN_CONFIG))?;
    let listen_config = AppConfigPartial {
        listen: Some(listen_config),
        ..Default::default()
    };

    AppConfig::builder()
        .layer(hard_coded.freeze())
        .layer(user_config1)
        .layer(user_config2)
        .layer(listen_config)
        .build()
}

fn main() -> Result<(), ConfigError> {
    let dir = config_dir();

    let config = load_config(&dir)?;
    println!("loaded config: {config:#?}");
    Ok(())
}
