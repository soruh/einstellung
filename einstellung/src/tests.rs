#![allow(unused)]

use crate::{Config, ConfigError, Freezable, JsonFileProvider, PartialConfig};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    default,
    fmt::Debug,
    net::IpAddr,
};

fn not_loopback(address: &IpAddr) -> Result<(), crate::BoxError> {
    if address.is_loopback() {
        return Err("Address must not be a multicast address".into());
    }
    Ok(())
}

fn merge_nonempty(
    current: Option<String>,
    next: Option<String>,
) -> Result<Option<String>, &'static str> {
    match next.as_deref() {
        Some("") => Err("value must not be empty"),
        Some(_) => Ok(next),
        None => Ok(current),
    }
}

#[derive(Config, Debug)]
#[config(crate = crate)]
struct CustomMergeConfig {
    #[config(merge(function = "merge_nonempty"))]
    value: String,
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

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn config_error_is_send_sync() {
    assert_send_sync::<ConfigError>();
}

#[test]
fn provider_error_preserves_source() {
    use std::error::Error;

    let err = ConfigError::provider(
        "test",
        std::io::Error::new(std::io::ErrorKind::InvalidData, "bad provider input"),
    );

    assert_eq!(err.to_string(), "test provider error: bad provider input");
    assert_eq!(
        err.source().map(ToString::to_string),
        Some("bad provider input".to_owned())
    );
}

#[track_caller]
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
    snapshot!(
        false,
        r#"{ "app_name": "foo", "network": { "listen": { "port": 443 } } }"#
    );
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
struct UserConfig1 {
    #[config(merge = "extend", default)]
    users: BTreeSet<String>,
}

#[test]
fn user_config() {
    snapshot!(UserConfig1, true, r#"{ "users": ["root", "bob"] }"#);
}

#[test]
fn user_config_allowed_empty() {
    snapshot!(UserConfig1, true, r#"{ }"#);
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
    #[config(merge = "extend")]
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
    #[config(default = || "Freezable Config 1".to_string())]
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
    #[config(default = || "Freezable Config 2".to_string())]
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

#[test]
fn config_error_propagates_through_anyhow() {
    fn fail() -> anyhow::Result<()> {
        Err(ConfigError::MissingField(crate::FieldPath::new(
            "TestConfig",
            "api_key",
        )))?;
        Ok(())
    }

    let err = fail().unwrap_err();
    assert!(err.downcast_ref::<ConfigError>().is_some());
}

#[test]
fn config_builder_merges_providers_in_order() {
    let base = JsonFileProvider::from_contents(
        r#"{ "app_name": "base", "network": { "listen": { "address": "192.168.0.1" } } }"#,
    );
    let override_layer = JsonFileProvider::from_contents(
        r#"{ "app_name": "override", "network": { "listen": { "port": 8443 } } }"#,
    );

    let config = AppConfig::builder()
        .provider(&base)
        .provider(&override_layer)
        .build()
        .unwrap();

    assert_eq!(config.app_name, "override");
    assert_eq!(
        config.network.listen.address,
        "192.168.0.1".parse::<IpAddr>().unwrap()
    );
    assert_eq!(config.network.listen.port, 8443);
}

#[test]
fn config_builder_accepts_loaded_layers() {
    let provider = JsonFileProvider::from_contents(
        r#"{ "app_name": "layer", "network": { "listen": { "address": "192.168.0.1" } } }"#,
    );
    let layer = AppConfig::load_partial(&provider).unwrap();

    let config = AppConfig::builder().layer(layer).build().unwrap();

    assert_eq!(config.app_name, "layer");
    assert_eq!(config.network.listen.port, 443);
}

#[test]
fn config_builder_retains_first_provider_error() {
    let invalid = JsonFileProvider::from_contents("{");
    let valid = JsonFileProvider::from_contents(
        r#"{ "app_name": "valid", "network": { "listen": { "address": "192.168.0.1" } } }"#,
    );

    let error = AppConfig::builder()
        .provider(&invalid)
        .provider(&valid)
        .build()
        .unwrap_err();

    assert!(matches!(error, ConfigError::Json(_)));
}

#[test]
fn custom_merge_accepts_convertible_error_types() {
    let base =
        CustomMergeConfig::load_partial(&JsonFileProvider::from_contents(r#"{ "value": "base" }"#))
            .unwrap();
    let invalid =
        CustomMergeConfig::load_partial(&JsonFileProvider::from_contents(r#"{ "value": "" }"#))
            .unwrap();

    let error = match base.merge(invalid) {
        Ok(_) => panic!("custom merge unexpectedly succeeded"),
        Err(error) => error,
    };

    match error {
        ConfigError::CustomMerge { field, reason } => {
            assert_eq!(field.to_string(), "CustomMergeConfig::value");
            assert_eq!(reason.to_string(), "value must not be empty");
        }
        other => panic!("unexpected error: {other}"),
    }
}
