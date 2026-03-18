#![allow(unused)]

use crate::{Config, ConfigError, JsonFileProvider};
use std::net::IpAddr;

fn not_loopback(address: &IpAddr) -> Result<(), Box<dyn std::error::Error>> {
    if address.is_loopback() {
        return Err("Address must not be a multicast address".into());
    }
    Ok(())
}

#[derive(Config, Debug)]
#[config(crate = crate)]
struct AppConfig {
    app_name: String,

    #[config(subconfig)]
    network: NetworkConfig,
}

#[derive(Config, Debug)]
#[config(crate = crate)]
struct NetworkConfig {
    #[config(subconfig)]
    listen: ListenConfig,
}

#[derive(Config, Debug)]
#[config(crate = crate)]
struct ListenConfig {
    #[config(validate = not_loopback)]
    address: IpAddr,
    #[config(default = "443")]
    port: u16,
}

fn print_res(res: Result<AppConfig, ConfigError>) -> String {
    match res {
        Ok(res) => format!("pass:\n{res:#?}"),
        Err(err) => format!("error:\n{err}\n{err:#?}"),
    }
}

macro_rules! snapshot {
    ($s:expr) => {{
        let res = AppConfig::load_complete(&JsonFileProvider::new($s));
        insta::assert_snapshot!(print_res(res));
    }};
}

#[test]
fn missing_field() {
    snapshot!(r#"{ "network": { "listen": { "address": "192.168.0.1" } } }"#);
}

#[test]
fn missing_nested_field() {
    snapshot!(r#"{ "app_name": "foo" }"#);
}

#[test]
fn validation_fail() {
    snapshot!(r#"{ "app_name": "foo", "network": { "listen": { "address": "127.0.0.1" } } }"#);
}

#[test]
fn success() {
    snapshot!(r#"{ "app_name": "foo", "network": { "listen": { "address": "192.168.0.1" } } }"#);
}
