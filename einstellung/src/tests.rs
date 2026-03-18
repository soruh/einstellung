#![allow(unused)]

use crate::{Config, ConfigError, JsonFileProvider};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fmt::Debug,
    net::IpAddr,
};

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
    #[config(default = 443)]
    port: u16,
}

fn print_res<T: Debug>(res: Result<T, ConfigError>) -> String {
    match res {
        Ok(res) => format!("pass:\n{res:#?}"),
        Err(err) => format!("error:\n{err}\n{err:#?}"),
    }
}

macro_rules! snapshot {
    ($s:expr) => {
        snapshot!(AppConfig, $s);
    };
    ($t: ty, $s:expr) => {{
        let res = <$t>::load_complete(&JsonFileProvider::new($s));
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

#[derive(Config, Debug)]
#[config(crate = crate)]
struct UserConfig {
    #[config(merge = "extend", default)]
    users: BTreeSet<String>,
}

#[test]
fn user_config() {
    snapshot!(UserConfig, r#"{ "users": ["root", "bob"] }"#);
}

#[test]
fn user_config_allowed_empty() {
    snapshot!(UserConfig, r#"{ }"#);
}

#[derive(Config, Debug)]
#[config(crate = crate)]
struct UserConfig2 {
    #[config(merge = "extend")]
    users: BTreeSet<String>,
}

#[test]
fn user_config_no_default() {
    snapshot!(UserConfig2, r#"{ "users": ["root", "bob"] }"#);
}

#[test]
fn user_config_no_default_not_allowed_empty() {
    snapshot!(UserConfig2, r#"{ }"#);
}

#[derive(Config, Debug)]
#[config(crate = crate)]
struct UserConfig3 {
    #[config(merge = "extend")]
    users: Option<BTreeSet<String>>,
}

#[test]
fn user_config_option_no_default() {
    snapshot!(UserConfig3, r#"{ "users": ["root", "bob"] }"#);
}

#[test]
fn user_config_option_no_default_not_allowed_empty() {
    snapshot!(UserConfig3, r#"{ }"#);
}

#[derive(Config, Debug)]
#[config(crate = crate)]
struct UserConfig4 {
    #[config(merge = "extend", default)]
    users: Option<BTreeSet<String>>,
}

#[test]
fn user_config_option() {
    snapshot!(UserConfig4, r#"{ "users": ["root", "bob"] }"#);
}

#[test]
fn user_config_option_not_allowed_empty() {
    snapshot!(UserConfig4, r#"{ }"#);
}
