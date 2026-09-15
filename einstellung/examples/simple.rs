//! Load a complete configuration from the bundled YAML fixture.

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
    colors: Option<ColorConfig>,

    #[config(merge = "extend")]
    #[config(default = || ["root".to_string()].into_iter().collect())]
    /// Users accumulated from the supplied configuration layers.
    users: HashSet<String>,

    /// File limit demonstrating optional or frozen field behavior.
    max_open_files: Option<usize>,
}

#[derive(einstellung::Config, Debug)]
/// Color settings read from the configuration.
struct ColorConfig {
    /// Primary color in the application’s chosen text format.
    primary: String,
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

fn main() -> Result<(), ConfigError> {
    let dir = config_dir();

    let config =
        AppConfig::load_complete(&YamlFileProvider::from_path_buf(dir.join("config.yaml")))?;
    println!("loaded config: {config:#?}");
    Ok(())
}
