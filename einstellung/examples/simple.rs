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
struct UserConfig2 {
    #[config(merge = "extend")]
    users: std::collections::BTreeSet<String>,
}

#[derive(einstellung::serde::Deserialize, Debug)]
enum LogLevel {
    Error,
    Warning,
    Info,
    Debug,
    Trace,
}

#[derive(einstellung::Config, Debug)]
struct AppConfig {
    app_name: String,

    #[config(default = LogLevel::Warning)]
    log_level: LogLevel,

    #[config(subconfig)]
    listen: ListenConfig,

    #[config(subconfig)]
    colors: Option<ColorConfig>,

    #[config(merge = "extend")]
    #[config(default = || ["root".to_string()].into_iter().collect())]
    users: HashSet<String>,

    max_open_files: Option<usize>,
}

#[derive(einstellung::Config, Debug)]
struct ColorConfig {
    primary: String,
    secondary: String,
}

#[derive(einstellung::Config, Debug)]
struct ListenConfig {
    address: IpAddr,
    #[config(default = 443)]
    port: u16,
}

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
