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

fn validate_nonempty(value: &str) -> Result<(), &'static str> {
    if value.is_empty() {
        Err("value must not be empty")
    } else {
        Ok(())
    }
}

#[derive(Config, Debug)]
#[config(crate = crate)]
struct CoercedValidatorConfig {
    #[config(validate = validate_nonempty)]
    value: String,
}

#[derive(Config, Debug)]
#[config(crate = crate)]
struct BuiltinValidatorConfig {
    #[config(validate = crate::validators::non_blank)]
    name: String,
    #[config(validate = crate::validators::non_empty_slice)]
    tags: Vec<String>,
    #[config(validate = |port: &u16| {
        if (1..=49151).contains(port) { Ok(()) } else { Err("port must be in the registered range") }
    })]
    port: u16,
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
struct OptionalCustomMergeConfig {
    #[config(merge(function = "merge_nonempty"))]
    value: Option<String>,
}

#[derive(Config, Debug)]
#[config(crate = crate)]
struct FreezableCustomMergeConfig {
    #[config(freezable, merge(function = "merge_nonempty"))]
    value: String,
}

#[derive(Config, Debug)]
#[config(crate = crate)]
struct NestedCustomMergeConfig {
    #[config(subconfig)]
    nested: CustomMergeConfig,
}

#[derive(Config, Debug)]
#[config(crate = crate, deny_unknown_fields)]
struct StrictConfig {
    value: String,
}

#[derive(Config, Debug)]
#[config(crate = crate)]
struct SecretConfig {
    api_key: crate::Secret<String>,
}

#[derive(Config, Debug)]
#[config(crate = crate)]
struct ModeConfig {
    name: String,
    #[config(subconfig)]
    remote: Option<RemoteConfig>,
}

#[derive(Config, Debug)]
#[config(crate = crate)]
struct RemoteConfig {
    api_url: String,
}

#[derive(Debug)]
struct RemoteMode {
    name: String,
    remote: RemoteConfig,
}

impl crate::ConfigView<ModeConfig> for RemoteMode {
    fn from_config(config: ModeConfig) -> Result<Self, ConfigError> {
        let remote = crate::require_for_view::<Self, _>(config.remote, "remote")?;
        Ok(Self {
            name: config.name,
            remote,
        })
    }
}

#[test]
fn secret_values_deserialize_but_debug_is_redacted() {
    let config = SecretConfig::load_complete(&JsonFileProvider::from_contents(
        r#"{ "api_key": "super-secret-value" }"#,
    ))
    .unwrap();

    assert_eq!(config.api_key.expose_secret(), "super-secret-value");
    let debug = format!("{config:?}");
    assert!(debug.contains("[REDACTED]"));
    assert!(!debug.contains("super-secret-value"));
}

#[test]
fn secret_values_require_explicit_unwrapping() {
    let secret = crate::Secret::new(String::from("token"));
    assert_eq!(secret.expose_secret(), "token");
    assert_eq!(secret.into_inner(), "token");
}

#[test]
fn require_for_view_reports_target_and_logical_path() {
    let error = crate::require_for_view::<RemoteMode, String>(None, "remote.api_url").unwrap_err();

    match error {
        ConfigError::MissingForView { view, field } => {
            assert!(view.ends_with("RemoteMode"));
            assert_eq!(field, "remote.api_url");
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn optional_config_can_build_without_mode_specific_fields() {
    let config =
        ModeConfig::load_complete(&JsonFileProvider::from_contents(r#"{ "name": "lint" }"#))
            .unwrap();
    assert!(config.remote.is_none());
}

#[test]
fn config_view_can_require_optional_mode_specific_fields() {
    let mode = ModeConfig::builder()
        .provider(&JsonFileProvider::from_contents(
            r#"{ "name": "run", "remote": { "api_url": "https://example.test" } }"#,
        ))
        .build_view::<RemoteMode>()
        .unwrap();

    assert_eq!(mode.name, "run");
    assert_eq!(mode.remote.api_url, "https://example.test");
}

#[test]
fn config_view_reports_missing_mode_requirement() {
    let error = ModeConfig::builder()
        .provider(&JsonFileProvider::from_contents(r#"{ "name": "run" }"#))
        .build_view::<RemoteMode>()
        .unwrap_err();

    match error.root_cause() {
        ConfigError::MissingForView { view, field } => {
            assert!(view.ends_with("RemoteMode"));
            assert_eq!(field, "remote");
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn tracked_config_preserves_provenance_through_view_conversion() {
    let tracked = ModeConfig::builder()
        .provider(&JsonFileProvider::from_contents(
            r#"{ "name": "run", "remote": { "api_url": "https://example.test" } }"#,
        ))
        .build_tracked_view::<RemoteMode>()
        .unwrap();

    assert_eq!(
        tracked
            .explain("remote.api_url")
            .unwrap()
            .last()
            .unwrap()
            .label(),
        "inline json"
    );
    assert_eq!(tracked.config().remote.api_url, "https://example.test");
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
fn typed_provider_trait_objects_support_runtime_composition() {
    struct RuntimeJsonProvider(&'static str);

    impl crate::ConfigProvider for RuntimeJsonProvider {
        fn load_partial<T: serde::de::DeserializeOwned>(&self) -> Result<T, ConfigError> {
            Ok(serde_json::from_str(self.0)?)
        }

        fn source(&self) -> crate::ConfigSource {
            crate::ConfigSource::new("runtime json provider")
        }
    }

    let providers: Vec<Box<dyn crate::ConfigProviderFor<AppConfig>>> = vec![
        Box::new(JsonFileProvider::from_owned_contents(
            r#"{ "app_name": "base" }"#.to_owned(),
        )),
        Box::new(RuntimeJsonProvider(
            r#"{ "app_name": "runtime", "network": { "listen": { "address": "192.168.0.1" } } }"#,
        )),
    ];

    let tracked = AppConfig::builder()
        .typed_providers(providers.iter().map(|provider| provider.as_ref()))
        .build_tracked()
        .unwrap();

    assert_eq!(tracked.config().app_name, "runtime");
    assert_eq!(
        tracked
            .explain("app_name")
            .unwrap()
            .iter()
            .map(crate::ConfigSource::label)
            .collect::<Vec<_>>(),
        vec!["inline json", "runtime json provider"]
    );
}

#[test]
fn custom_errors_can_attach_logical_paths() {
    let error = ConfigError::provider(
        "secret store",
        std::io::Error::new(std::io::ErrorKind::InvalidData, "lookup failed"),
    )
    .with_logical_path("model.remote.api_key")
    .with_source(crate::ConfigSource::new("vault"));

    assert_eq!(
        error.logical_path().as_deref(),
        Some("model.remote.api_key")
    );
    assert_eq!(error.config_source().unwrap().label(), "vault");
    assert!(matches!(error.root_cause(), ConfigError::Provider { .. }));
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

#[derive(Config, Debug)]
#[config(crate = crate)]
struct NestedFreezableConfig {
    #[config(subconfig)]
    nested: ConfigFreezable1,
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
fn nested_freeze_collision_includes_outer_field_context() {
    let mut first = NestedFreezableConfig::load_partial(&JsonFileProvider::from_contents(
        &format!("{{ \"nested\": {{ \"private_key\": {KEY:?} }} }}"),
    ))
    .unwrap();
    first.nested = first.nested.map(Freezable::freeze);

    let mut second = NestedFreezableConfig::load_partial(&JsonFileProvider::from_contents(
        r#"{ "nested": { "private_key": "overwritten key" } }"#,
    ))
    .unwrap();
    second.nested = second.nested.map(Freezable::freeze);

    let error = match first.merge(second) {
        Ok(_) => panic!("nested frozen merge unexpectedly succeeded"),
        Err(error) => error,
    };
    assert_eq!(error.logical_path().as_deref(), Some("nested.private_key"));
    assert_eq!(
        error.field_path().unwrap().to_string(),
        "NestedFreezableConfig::nested::private_key"
    );
    assert!(matches!(
        error.root_cause(),
        ConfigError::FreezeCollision(_)
    ));
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
fn config_builder_merges_provider_iterators_in_order() {
    let providers = [
        JsonFileProvider::from_contents(
            r#"{ "app_name": "base", "network": { "listen": { "address": "192.168.0.1" } } }"#,
        ),
        JsonFileProvider::from_contents(
            r#"{ "app_name": "override", "network": { "listen": { "port": 8443 } } }"#,
        ),
    ];

    let config = AppConfig::builder().providers(&providers).build().unwrap();

    assert_eq!(config.app_name, "override");
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

    assert!(matches!(error.root_cause(), ConfigError::Json(_)));
}

#[test]
fn failed_builder_does_not_load_later_providers() {
    use std::cell::Cell;

    struct CountingProvider<'a> {
        loads: &'a Cell<usize>,
    }

    impl crate::ConfigProvider for CountingProvider<'_> {
        fn load_partial<T: serde::de::DeserializeOwned>(&self) -> Result<T, ConfigError> {
            self.loads.set(self.loads.get() + 1);
            Ok(serde_json::from_str(
                r#"{ "app_name": "late", "network": { "listen": { "address": "192.168.0.1" } } }"#,
            )?)
        }
    }

    let invalid = JsonFileProvider::from_contents("{");
    let loads = Cell::new(0);
    let later = CountingProvider { loads: &loads };

    let _ = AppConfig::builder()
        .provider(&invalid)
        .provider(&later)
        .build()
        .unwrap_err();
    assert_eq!(loads.get(), 0);

    let invalid: &dyn crate::ConfigProviderFor<AppConfig> = &invalid;
    let later: &dyn crate::ConfigProviderFor<AppConfig> = &later;
    let _ = AppConfig::builder()
        .typed_provider(invalid)
        .typed_provider(later)
        .build()
        .unwrap_err();
    assert_eq!(loads.get(), 0);
}

#[test]
fn custom_merge_works_for_optional_complete_fields() {
    let base = OptionalCustomMergeConfig::load_partial(&JsonFileProvider::from_contents(
        r#"{ "value": "base" }"#,
    ))
    .unwrap();
    let next = OptionalCustomMergeConfig::load_partial(&JsonFileProvider::from_contents(
        r#"{ "value": "next" }"#,
    ))
    .unwrap();

    let merged = base.merge(next).unwrap().build().unwrap();

    assert_eq!(merged.value.as_deref(), Some("next"));
}

#[test]
fn custom_merge_works_for_freezable_fields() {
    let base = FreezableCustomMergeConfig::load_partial(&JsonFileProvider::from_contents(
        r#"{ "value": "base" }"#,
    ))
    .unwrap();
    let next = FreezableCustomMergeConfig::load_partial(&JsonFileProvider::from_contents(
        r#"{ "value": "next" }"#,
    ))
    .unwrap();

    let merged = base.merge(next).unwrap().build().unwrap();
    assert_eq!(merged.value, "next");
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

#[test]
fn deny_unknown_fields_rejects_typos() {
    let error = StrictConfig::load_partial(&JsonFileProvider::from_contents(
        r#"{ "value": "ok", "vlaue": "typo" }"#,
    ))
    .err()
    .expect("unknown field should be rejected");

    match error.root_cause() {
        ConfigError::Json(error) => {
            assert!(error.to_string().contains("data error"));
            assert!(
                error
                    .parser_error()
                    .to_string()
                    .contains("unknown field `vlaue`")
            );
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn builtin_and_closure_validators_work() {
    let valid = BuiltinValidatorConfig::load_complete(&JsonFileProvider::from_contents(
        r#"{ "name": "worker", "tags": ["api"], "port": 443 }"#,
    ))
    .unwrap();
    assert_eq!(valid.port, 443);

    let error = BuiltinValidatorConfig::load_complete(&JsonFileProvider::from_contents(
        r#"{ "name": "   ", "tags": ["api"], "port": 443 }"#,
    ))
    .unwrap_err();
    assert_eq!(error.field_path().unwrap().logical_path(), "name");

    let error = BuiltinValidatorConfig::load_complete(&JsonFileProvider::from_contents(
        r#"{ "name": "worker", "tags": [], "port": 443 }"#,
    ))
    .unwrap_err();
    assert_eq!(error.field_path().unwrap().logical_path(), "tags");

    let error = BuiltinValidatorConfig::load_complete(&JsonFileProvider::from_contents(
        r#"{ "name": "worker", "tags": ["api"], "port": 60000 }"#,
    ))
    .unwrap_err();
    assert_eq!(error.field_path().unwrap().logical_path(), "port");
}

#[test]
fn nested_merge_errors_include_outer_field_context() {
    let base = JsonFileProvider::from_contents(r#"{ "nested": { "value": "base" } }"#);
    let override_layer = JsonFileProvider::from_contents(r#"{ "nested": { "value": "" } }"#);

    let error = NestedCustomMergeConfig::builder()
        .provider(&base)
        .provider(&override_layer)
        .build()
        .unwrap_err();

    assert_eq!(error.field_path().unwrap().logical_path(), "nested.value");
    assert_eq!(
        error.field_path().unwrap().to_string(),
        "NestedCustomMergeConfig::nested::value"
    );
    assert!(error.to_string().starts_with(
        "configuration source inline json: Custom Merge failed for field 'NestedCustomMergeConfig::nested::value'"
    ));
}

#[test]
fn validator_uses_normal_reference_coercions() {
    let partial = CoercedValidatorConfig::load_partial(&JsonFileProvider::from_contents(
        r#"{ "value": "" }"#,
    ))
    .unwrap();

    let error = partial.build().unwrap_err();
    match error {
        ConfigError::Validation { field, reason } => {
            assert_eq!(field.to_string(), "CoercedValidatorConfig::value");
            assert_eq!(reason.to_string(), "value must not be empty");
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn field_paths_have_logical_dotted_form() {
    let path = crate::FieldPath::new("ListenConfig", "address")
        .context("NetworkConfig", "listen")
        .context("AppConfig", "network");

    assert_eq!(path.logical_path(), "network.listen.address");
    assert_eq!(path.to_string(), "AppConfig::network::listen::address");
}

#[test]
fn tracked_builder_explains_nested_sources_and_defaults() {
    let base = AppConfig::load_partial(&JsonFileProvider::from_contents(
        r#"{ "app_name": "base", "network": { "listen": { "address": "192.168.0.1" } } }"#,
    ))
    .unwrap();
    let override_layer = AppConfig::load_partial(&JsonFileProvider::from_contents(
        r#"{ "app_name": "override", "network": { "listen": {} } }"#,
    ))
    .unwrap();

    let tracked = AppConfig::builder()
        .layer_named("base config", base)
        .layer_named("local override", override_layer)
        .build_tracked()
        .unwrap();

    assert_eq!(tracked.config().app_name, "override");
    assert_eq!(
        tracked
            .explain("app_name")
            .unwrap()
            .iter()
            .map(crate::ConfigSource::label)
            .collect::<Vec<_>>(),
        vec!["base config", "local override"]
    );
    assert_eq!(
        tracked
            .explain("network.listen.address")
            .unwrap()
            .last()
            .unwrap()
            .label(),
        "base config"
    );
    assert_eq!(
        tracked
            .provenance()
            .latest_supplier("app_name")
            .unwrap()
            .label(),
        "local override"
    );
    assert_eq!(
        tracked
            .explain("network.listen.port")
            .unwrap()
            .last()
            .unwrap()
            .label(),
        "field default"
    );
}

#[test]
fn direct_load_attaches_source_to_validation_errors() {
    let error = AppConfig::load_complete(&JsonFileProvider::from_contents(
        r#"{ "app_name": "bad", "network": { "listen": { "address": "127.0.0.1" } } }"#,
    ))
    .unwrap_err();

    assert_eq!(
        error.field_path().unwrap().logical_path(),
        "network.listen.address"
    );
    assert!(
        error
            .to_string()
            .starts_with("configuration source inline json:")
    );
    assert!(matches!(error.root_cause(), ConfigError::Validation { .. }));
}

#[test]
fn tracked_partial_retains_sources_without_applying_defaults() {
    let layer = AppConfig::load_partial(&JsonFileProvider::from_contents(
        r#"{ "app_name": "base", "network": { "listen": { "address": "192.168.0.1" } } }"#,
    ))
    .unwrap();

    let tracked = AppConfig::builder()
        .layer_named("base config", layer)
        .build_tracked_partial()
        .unwrap();

    assert_eq!(
        tracked.explain("app_name").unwrap().last().unwrap().label(),
        "base config"
    );
    assert!(tracked.explain("network.listen.port").is_none());
}

#[test]
fn build_partial_errors_retain_prior_provenance() {
    let base = AppConfig::load_partial(&JsonFileProvider::from_contents(
        r#"{ "app_name": "base", "network": { "listen": { "address": "192.168.0.1" } } }"#,
    ))
    .unwrap();

    let error = match AppConfig::builder()
        .layer_named("base config", base)
        .provider(&JsonFileProvider::from_contents("{"))
        .build_partial()
    {
        Ok(_) => panic!("invalid provider unexpectedly produced a partial config"),
        Err(error) => error,
    };

    assert_eq!(error.config_source().unwrap().label(), "inline json");
    assert_eq!(
        error
            .provenance()
            .unwrap()
            .latest_supplier("app_name")
            .unwrap()
            .label(),
        "base config"
    );
}

#[test]
fn builder_errors_retain_provenance_for_failed_values() {
    let error = AppConfig::builder()
        .provider(&JsonFileProvider::from_contents(
            r#"{ "app_name": "bad", "network": { "listen": { "address": "127.0.0.1" } } }"#,
        ))
        .build()
        .unwrap_err();

    assert_eq!(
        error.logical_path().as_deref(),
        Some("network.listen.address")
    );
    assert_eq!(
        error
            .provenance()
            .unwrap()
            .latest_supplier("network.listen.address")
            .unwrap()
            .label(),
        "inline json"
    );
    assert!(matches!(error.root_cause(), ConfigError::Validation { .. }));
}

#[test]
fn view_errors_retain_composed_provenance() {
    let error = ModeConfig::builder()
        .provider(&JsonFileProvider::from_contents(r#"{ "name": "run" }"#))
        .build_view::<RemoteMode>()
        .unwrap_err();

    assert_eq!(error.logical_path().as_deref(), Some("remote"));
    assert_eq!(
        error
            .provenance()
            .unwrap()
            .latest_supplier("name")
            .unwrap()
            .label(),
        "inline json"
    );
    assert!(matches!(
        error.root_cause(),
        ConfigError::MissingForView { .. }
    ));
}

#[test]
fn field_source_accessors_combine_provenance_and_failed_layer() {
    let base = NestedCustomMergeConfig::load_partial(&JsonFileProvider::from_contents(
        r#"{ "nested": { "value": "base" } }"#,
    ))
    .unwrap();
    let override_layer = NestedCustomMergeConfig::load_partial(&JsonFileProvider::from_contents(
        r#"{ "nested": { "value": "" } }"#,
    ))
    .unwrap();

    let error = NestedCustomMergeConfig::builder()
        .layer_named("base config", base)
        .layer_named("CLI override", override_layer)
        .build()
        .unwrap_err();

    assert_eq!(
        error
            .field_provenance()
            .unwrap()
            .iter()
            .map(crate::ConfigSource::label)
            .collect::<Vec<_>>(),
        vec!["base config"]
    );
    assert_eq!(
        error
            .field_sources()
            .into_iter()
            .map(crate::ConfigSource::label)
            .collect::<Vec<_>>(),
        vec!["base config", "CLI override"]
    );
    assert_eq!(error.latest_field_source().unwrap().label(), "CLI override");
}

#[test]
fn diagnostic_accessors_expose_paths_and_sources_without_values() {
    let error = AppConfig::load_complete(&JsonFileProvider::from_contents(
        r#"{ "app_name": "bad", "network": { "listen": { "address": "127.0.0.1" } } }"#,
    ))
    .unwrap_err();

    assert_eq!(
        error.logical_path().as_deref(),
        Some("network.listen.address")
    );
    assert_eq!(error.config_source().unwrap().label(), "inline json");

    let missing = ConfigError::missing_for_view::<RemoteMode>("remote.api_url");
    assert_eq!(missing.logical_path().as_deref(), Some("remote.api_url"));
    assert!(missing.config_source().is_none());
}

#[test]
fn provenance_can_be_enumerated_without_configuration_values() {
    let tracked = AppConfig::builder()
        .provider(&JsonFileProvider::from_contents(
            r#"{ "app_name": "api", "network": { "listen": { "address": "192.168.0.1" } } }"#,
        ))
        .build_tracked()
        .unwrap();

    let paths = tracked
        .provenance()
        .iter()
        .map(|(path, sources)| (path, sources.last().unwrap().label()))
        .collect::<Vec<_>>();

    assert_eq!(
        paths,
        vec![
            ("app_name", "inline json"),
            ("network.listen.address", "inline json"),
            ("network.listen.port", "field default"),
        ]
    );
    assert!(!tracked.provenance().is_empty());
}

#[test]
fn structured_errors_expose_safe_source_locations_through_context() {
    let error = AppConfig::load_complete(&JsonFileProvider::from_contents(
        r#"{ "app_name": 7, "network": { "listen": { "address": "127.0.0.1" } } }"#,
    ))
    .unwrap_err();

    let location = error
        .source_location()
        .expect("JSON data error should include a source location");
    assert_eq!(location.line(), 1);
    assert!(location.column() > 0);
    assert_eq!(error.config_source().unwrap().label(), "inline json");
}

#[test]
fn providers_describe_sources_without_inline_contents() {
    let provider = JsonFileProvider::from_contents(r#"{ "api_key": "do-not-leak" }"#);
    assert_eq!(
        crate::ConfigProvider::source(&provider).label(),
        "inline json"
    );
}
