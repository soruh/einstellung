#![allow(unused)]

use std::{
    collections::HashSet,
    default,
    net::IpAddr,
    path::{Path, PathBuf},
};

use einstellung::{
    Config, ConfigProvider, JsonFileProvider, PartialConfig, TomlFileProvider, YamlFileProvider,
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
    colors: ColorConfig,

    #[config(merge = "extend")]
    users: HashSet<String>,
}

#[derive(einstellung::Config, Debug)]
struct ColorConfig {
    primary: String,
    #[config(default = "#0ff".to_string())]
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
fn main() {
    const LISTEN_CONFIG: &str = r#"{ "address": "127.0.0.1" }"#;
    const HARD_CODED_CONFIG: &str = r#"{ "users": ["root"] }"#;

    let path = config_dir();

    let listen_config = ListenConfig::load_partial(&JsonFileProvider::new(LISTEN_CONFIG)).unwrap();

    let hard_coded = AppConfig::load_partial(&JsonFileProvider::new(HARD_CODED_CONFIG)).unwrap();

    let user_config1 = YamlFileProvider::new(path.join("config.yaml"))
        .load_partial()
        .unwrap();

    let user_config2 = TomlFileProvider::new(path.join("config.toml"))
        .load_partial()
        .unwrap();

    let config = hard_coded
        .merge(user_config1)
        .merge(user_config2)
        .merge(AppConfigPartial {
            listen: Some(listen_config),
            ..Default::default()
        })
        .build();

    dbg!(&config);
}
