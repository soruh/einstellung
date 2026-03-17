macro_rules! assert_macro_test {
    (
        $mode:ident, $name:ident:
        $( { $($tokens:tt)* } ),+ $(,)?
    ) => {
        paste::paste! {
            // --- Snapshot Test ---
            #[test]
            fn $name() {
                use quote::quote;
                let mut formatted_snapshots = Vec::new();

                $(
                    let input = quote! { $($tokens)* };
                    let output = crate::derive_config::derive(input.clone());

                    let combined_snapshot = quote! {
                        /// --- input ---
                        #input
                        /// --- output ---
                        #output
                    };
                    let syntax_tree: syn::File = syn::parse2(combined_snapshot.clone())
                        .expect("invalid combined syntax");
                    formatted_snapshots.push(prettyplease::unparse(&syntax_tree));
                )+

                let formatted = formatted_snapshots.join("\n// ---------------------------------\n");
                insta::assert_snapshot!(formatted);
            }

            // --- Trybuild Test ---
            #[test]
            fn [<$name _compile>]() {
                use quote::quote;
                let mut combined_input = quote! {};
                $(
                    combined_input.extend(quote! { $($tokens)* });
                )+

                let trybuild_tokens = quote! {
                    #[allow(unused_imports)]
                    use einstellung_derive::Config;
                    #combined_input
                    fn main() {}
                };

                let syntax_tree: syn::File = syn::parse2(trybuild_tokens).expect("Generated invalid code");
                let formatted_code = prettyplease::unparse(&syntax_tree);

                let manifest_dir = env!("CARGO_MANIFEST_DIR");
                let dir_path = std::path::Path::new(manifest_dir).join("src").join("trybuild_tests");
                std::fs::create_dir_all(&dir_path).expect("failed to create trybuild directory");

                let file_path = dir_path.join(format!("{}.rs", stringify!($name)));
                std::fs::write(&file_path, formatted_code).expect("failed to write trybuild file");

                let t = trybuild::TestCases::new();
                match stringify!($mode) {
                    "PASS" => t.pass(&file_path),
                    "FAIL" => t.compile_fail(&file_path),
                    other => panic!("invalid mode: {}", other),
                }
            }
        }
    };
}

assert_macro_test!(PASS, test_basic_primitives: {
    #[derive(Config)]
    struct ServerConfig {
        host: String,
        port: u16,
        is_active: bool,
    }
});

assert_macro_test!(FAIL, test_invalid: {
    #[derive(Config)]
    struct ServerConfig(u16);
});

assert_macro_test!(FAIL, test_invalid_merge: {
    #[derive(Config)]
    struct ServerConfig {
        host: String,
        port: u16,
        #[config(merge = "foo")]
        is_active: bool,
    }
});

assert_macro_test!(FAIL, test_invalid_merge2: {
    #[derive(Config)]
    struct ServerConfig {
        host: String,
        port: u16,
        #[config(merge = "subconfig")]
        is_active: bool,
    }
});

assert_macro_test!(PASS, test_optional_fields_no_double_option: {
    #[derive(Config)]
    struct ClientConfig {
        name: String,
        timeout_ms: Option<u32>,
        proxy: Option<String>,
    }
});

assert_macro_test!(PASS, test_default_values: {
    #[derive(Config)]
    struct NetworkConfig {
        #[config(default = "\"localhost\".to_string()")]
        host: String,
        #[config(default = "8080")]
        port: u16,
        #[config(default = "std::time::Duration::from_secs(30)")]
        timeout: std::time::Duration,
    }
});

assert_macro_test!(PASS, test_subconfig_resolution:
    {
        #[derive(Config)]
        struct AppConfig {
            app_name: String,
            #[config(subconfig)]
            database: DatabaseConfig,
            #[config(subconfig)]
            redis: RedisConfig,
        }
    },
    {
        #[derive(Config)]
        struct DatabaseConfig {
            url: String,
            pool_size: u32,
        }
    },
    {
        #[derive(Config)]
        struct RedisConfig {
            cluster_mode: bool,
        }
    }
);

assert_macro_test!(PASS, test_optional_subconfig:
    {
        #[derive(Config)]
        struct TelemetryConfig {
            enabled: bool,
            #[config(subconfig)]
            datadog: Option<DatadogConfig>,
        }
    },
    {
        #[derive(Config)]
        struct DatadogConfig {
            api_key: String,
        }
    }
);

assert_macro_test!(PASS, test_merge_strategies: {
    #[derive(Config)]
    struct LoggerConfig {
        level: String,
        #[config(merge = "append")]
        log_files: Vec<String>,
        #[config(merge = "replace")]
        output_format: String,
    }
});

assert_macro_test!(PASS, test_validation_functions: {
    #[derive(Config)]
    struct TlsConfig {
        #[config(validate = "crate::validators::validate_cert_path")]
        cert_path: String,
        #[config(validate = "crate::validators::validate_port")]
        port: u16,
    }
});

assert_macro_test!(PASS, test_serde_attribute_forwarding: {
    #[derive(Config)]
    struct ApiConfig {
        #[serde(rename = "API_KEY")]
        key: String,
        #[serde(alias = "max_retries", default)]
        retries: u8,
        #[serde(skip_serializing_if = "Option::is_none")]
        endpoint: Option<String>,
    }
});

assert_macro_test!(PASS, test_kitchen_sink:
    {
        #[derive(Config)]
        struct FullSystemConfig {
            #[serde(rename = "sys_name")]
            #[config(default = "\"production\".to_string()")]
            name: String,
            #[config(validate = "validate_system_port")]
            port: u16,
            #[config(subconfig)]
            database: DatabaseConfig,
            #[config(merge = "append")]
            #[serde(alias = "files")]
            log_files: Option<Vec<String>>,
            #[config(subconfig)]
            optional_cache: Option<CacheConfig>,
        }
    },
    {
        #[derive(Config)]
        struct DatabaseConfig {
            url: String,
            #[config(default = "5432")]
            port: u16,
        }
    },
    {
        #[derive(Config)]
        struct CacheConfig {
            #[config(default = "1024")]
            size_mb: u32,
        }
    }
);
