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
    colors: ColorConfig,

    #[config(merge = "extend")]
    users: HashSet<String>,

    #[config(freezable)]
    max_open_files: usize,
}

#[derive(einstellung::Config, Debug)]
struct ColorConfig {
    primary: String,
    #[config(default = || "#0ff".to_string())]
    secondary: String,
}

#[derive(einstellung::Config, Debug)]
struct ListenConfig {
    address: IpAddr,
    #[config(default = 443)]
    port: u16,
}

fn config_dir() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let relative = Path::new(file!()).iter().skip(1).collect::<PathBuf>();

    manifest_dir
        .join(relative)
        .parent()
        .unwrap()
        .canonicalize()
        .unwrap()
}

fn load_config(dir: &Path) -> Result<AppConfig, ConfigError> {
    const LISTEN_CONFIG: &str = r#"{ "address": "127.0.0.1" }"#;
    const HARD_CODED_CONFIG: &str = r#"{ "users": ["root"], "max_open_files": 10 }"#;

    let hard_coded = AppConfig::load_partial(&JsonFileProvider::new(HARD_CODED_CONFIG))?;
    let user_config1 = YamlFileProvider::new(dir.join("config.yaml")).load_partial()?;
    let user_config2 = TomlFileProvider::new(dir.join("config.toml")).load_partial()?;
    let listen_config = ListenConfig::load_partial(&JsonFileProvider::new(LISTEN_CONFIG)).unwrap();
    let listen_config = AppConfigPartial {
        listen: Some(listen_config),
        ..Default::default()
    };

    hard_coded
        .freeze()
        .merge(user_config1)?
        .merge(user_config2)?
        .merge(listen_config)?
        .build()
}

fn main() {
    let dir = config_dir();

    match load_config(&dir) {
        Ok(config) => println!("loaded config: {config:#?}"),
        Err(err) => eprintln!("failed to load config: {err}"),
    }
}
