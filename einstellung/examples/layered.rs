#![allow(unused)]

use std::{collections::HashSet, net::IpAddr, path::Path};

use einstellung::{
    Config, ConfigProvider, JsonFileProvider, PartialConfig, TomlFileProvider, YamlFileProvider,
};

#[derive(einstellung::Config, Debug)]
struct AppConfig {
    app_name: String,

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
    secondary: String,
}

#[derive(einstellung::Config, Debug)]
struct ListenConfig {
    address: IpAddr,
    #[config(default = "443")]
    port: u16,
}

fn main() {
    const LISTEN_CONFIG: &str = r#"{ "address": "127.0.0.1" }"#;
    const HARD_CODED_CONFIG: &str = r#"{ "users": ["root"] }"#;

    let listen_config = ListenConfig::load_partial(&JsonFileProvider::new(LISTEN_CONFIG)).unwrap();

    let hard_coded = AppConfig::load_partial(&JsonFileProvider::new(HARD_CODED_CONFIG)).unwrap();

    let user_config1 = YamlFileProvider::new(Path::new("./config.yaml"))
        .load_partial()
        .unwrap();

    let user_config2 = TomlFileProvider::new(Path::new("./config.toml"))
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
