#![allow(unused)]

use crate::{Config, ConfigError, Freezable, JsonFileProvider, PartialConfig};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    default,
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

fn print_res<T: Debug>(res: Result<T, ConfigError>, expect_success: bool) -> String {
    let s = match &res {
        Ok(res) => format!("pass:\n{res:#?}"),
        Err(err) => format!("error:\n{err}\n{err:#?}"),
    };

    assert!(
        res.is_ok() == expect_success,
        "expected {} but got: {s}",
        if expect_success { "sucess" } else { "failure" }
    );

    s
}

macro_rules! snapshot {
    ($success: literal, $s: expr) => {
        snapshot!(AppConfig, $success, $s);
    };
    ($t: ty, $success: literal, $s: expr) => {{
        let res = <$t>::load_complete(&JsonFileProvider::new($s));
        insta::assert_snapshot!(print_res(res, $success));
    }};
}

#[test]
fn missing_field() {
    snapshot!(
        false,
        r#"{ "network": { "listen": { "address": "192.168.0.1" } } }"#
    );
}

#[test]
fn missing_nested_field() {
    snapshot!(false, r#"{ "app_name": "foo" }"#);
}

#[test]
fn validation_fail() {
    snapshot!(
        false,
        r#"{ "app_name": "foo", "network": { "listen": { "address": "127.0.0.1" } } }"#
    );
}

#[test]
fn success() {
    snapshot!(
        true,
        r#"{ "app_name": "foo", "network": { "listen": { "address": "192.168.0.1" } } }"#
    );
}

#[derive(Config, Debug)]
#[config(crate = crate)]
struct UserConfig {
    #[config(merge = "extend", default)]
    users: BTreeSet<String>,
}

#[test]
fn user_config() {
    snapshot!(UserConfig, true, r#"{ "users": ["root", "bob"] }"#);
}

#[test]
fn user_config_allowed_empty() {
    snapshot!(UserConfig, true, r#"{ }"#);
}

#[derive(Config, Debug)]
#[config(crate = crate)]
struct UserConfig2 {
    #[config(merge = "extend")]
    users: BTreeSet<String>,
}

#[test]
fn user_config_no_default() {
    snapshot!(UserConfig2, true, r#"{ "users": ["root", "bob"] }"#);
}

#[test]
fn user_config_no_default_not_allowed_empty() {
    snapshot!(UserConfig2, false, r#"{ }"#);
}

#[derive(Config, Debug)]
#[config(crate = crate)]
struct UserConfig3 {
    #[config(merge = "extend")]
    users: Option<BTreeSet<String>>,
}

#[test]
fn user_config_option_no_default() {
    snapshot!(UserConfig3, true, r#"{ "users": ["root", "bob"] }"#);
}

#[test]
fn user_config_option_no_default_allowed_empty() {
    snapshot!(UserConfig3, true, r#"{ }"#);
}

#[derive(Config, Debug)]
#[config(crate = crate)]
struct UserConfig4 {
    #[config(merge = "extend", default)]
    users: Option<BTreeSet<String>>,
}

#[test]
fn user_config_option() {
    snapshot!(UserConfig4, true, r#"{ "users": ["root", "bob"] }"#);
}

#[test]
fn user_config_option_allowed_empty() {
    snapshot!(UserConfig4, true, r#"{ }"#);
}

#[derive(Debug, Default, ::serde::Deserialize)]
enum ConfigMode {
    ModeA,
    ModeB,
    #[default]
    ModeC,
}

#[derive(Config, Debug)]
#[config(crate = crate)]
struct ConfigWithEnum {
    #[config(default)]
    mode: ConfigMode,
}

#[test]
fn config_enum_correct() {
    snapshot!(ConfigWithEnum, true, r#"{ "mode": "ModeA" }"#);
}

#[test]
fn config_enum_incorrect() {
    snapshot!(ConfigWithEnum, false, r#"{ "mode": "ModeZ" }"#);
}

#[test]
fn config_enum_missing() {
    snapshot!(ConfigWithEnum, true, r#"{ }"#);
}

#[derive(Config, Debug, PartialEq, Eq)]
#[config(crate = crate)]
#[config(partial(derive(Clone)))]
struct ConfigFreezable1 {
    #[config(default = "Freezable Config 1".to_string())]
    name: String,
    pass: u8,
    #[config(freezable)]
    private_key: String,
}

#[derive(Config, Debug, PartialEq, Eq)]
#[config(crate = crate)]
#[config(freezable)]
#[config(partial(derive(Clone)))]
struct ConfigFreezable2 {
    #[config(default = "Freezable Config 2".to_string())]
    name: String,
    pass: u8,
    private_key: String,
}

const KEY: &str = "uILfaXH0dj9qUGV71O/Wyg==";

#[test]
fn config_freeze_partial() {
    let frozen = ConfigFreezable1::load_partial(&JsonFileProvider::new(format!(
        "{{ \"private_key\": {KEY:?}, \"pass\": 1 }}"
    )))
    .unwrap()
    .freeze();

    let overwrite = ConfigFreezable1::load_partial(&JsonFileProvider::new(
        r#"{ "name": "overwritten name", "pass": 2, "private_key": "overwritten key" }"#,
    ))
    .unwrap();

    assert_eq!(
        frozen
            .clone()
            .merge(overwrite.clone())
            .unwrap()
            .build()
            .unwrap(),
        ConfigFreezable1 {
            name: "overwritten name".to_string(),
            pass: 2,
            private_key: KEY.to_string(),
        }
    );

    assert_eq!(
        overwrite.merge(frozen).unwrap().build().unwrap(),
        ConfigFreezable1 {
            name: "overwritten name".to_string(),
            pass: 1,
            private_key: KEY.to_string(),
        }
    );
}

#[test]
fn config_freeze_complete() {
    let frozen = ConfigFreezable2::load_partial(&JsonFileProvider::new(format!(
        "{{ \"private_key\": {KEY:?}, \"pass\": 1 }}"
    )))
    .unwrap()
    .freeze();

    let overwrite = ConfigFreezable2::load_partial(&JsonFileProvider::new(
        r#"{ "name": "overwritten name", "pass": 2, "private_key": "overwritten key" }"#,
    ))
    .unwrap();

    let expected = ConfigFreezable2 {
        name: "Freezable Config 2".to_string(),
        pass: 1,
        private_key: KEY.to_string(),
    };

    assert_eq!(
        frozen
            .clone()
            .merge(overwrite.clone())
            .unwrap()
            .build()
            .unwrap(),
        expected
    );
    assert_eq!(overwrite.merge(frozen).unwrap().build().unwrap(), expected);
}

#[test]
fn config_freeze_collision() {
    let frozen1 = ConfigFreezable2::load_partial(&JsonFileProvider::new(format!(
        "{{ \"private_key\": {KEY:?} }}"
    )))
    .unwrap()
    .freeze();

    let frozen2 = ConfigFreezable2::load_partial(&JsonFileProvider::new(
        r#"{ "name": "overwritten name", "private_key": "overwritten key" }"#,
    ))
    .unwrap()
    .freeze();

    let res = frozen1.merge(frozen2).and_then(|x| x.build());
    insta::assert_snapshot!(print_res(res, false));
}
