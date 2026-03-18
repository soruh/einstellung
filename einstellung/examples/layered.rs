#![allow(unused)]

use std::{collections::HashSet, net::IpAddr, path::Path};

use einstellung::{
    Config, ConfigProvider, PartialConfig, json::JsonFileProvider, yaml::YamlFileProvider,
};

#[derive(einstellung::Config, Debug)]
struct AppConfig {
    app_name: String,

    #[config(subconfig)]
    listen: ListenConfig,

    #[config(merge = "extend")]
    users: HashSet<String>,
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

    let user_config = YamlFileProvider::new(Path::new("./config.yaml"))
        .load_partial()
        .unwrap();

    let config = hard_coded
        .merge(user_config)
        .merge(AppConfigPartial {
            listen: Some(listen_config),
            ..Default::default()
        })
        .build();

    dbg!(&config);
}
