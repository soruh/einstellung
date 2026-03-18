macro_rules! assert_macro_test {
    // Entry point
    ( $mode:ident, $name:ident: $($tail:tt)* ) => {
        assert_macro_test!(@munch $mode, $name, [], [], $($tail)*);
    };

    // Muncher: helper { ... }
    ( @munch $mode:ident, $name:ident, [ $($snaps:expr,)* ], [ $($compiles:expr,)* ], helper { $($body:tt)* } $($tail:tt)* ) => {
        assert_macro_test!(
            @munch $mode, $name,
            [
                $($snaps,)* {
                    let inner = ::quote::quote! { $($body)* };
                    // Parse just the valid Rust code
                    let tree: syn::File = syn::parse2(inner).expect("Helper parse fail");
                    // Format as a string, then prepend our comment header
                    format!("/// --- helper ---\n{}", prettyplease::unparse(&tree))
                },
            ],
            [ $($compiles,)* ::quote::quote! { $($body)* }, ],
            $($tail)*
        );
    };

    // Muncher: standard block { ... }
    ( @munch $mode:ident, $name:ident, [ $($snaps:expr,)* ], [ $($compiles:expr,)* ], { $($body:tt)* } $($tail:tt)* ) => {
        assert_macro_test!(
            @munch $mode, $name,
            [
                $($snaps,)* {
                    let input = ::quote::quote! { $($body)* };
                    let output = crate::derive_config::derive(input.clone());

                    // Parse input and output separately (both are valid Rust code individually)
                    let in_tree: syn::File = syn::parse2(input).expect("Input parse fail");
                    let out_tree: syn::File = syn::parse2(output).expect("Output parse fail");

                    // Stitch them together using standard string formatting
                    format!(
                        "/// --- input ---\n{}\n/// --- output ---\n{}",
                        prettyplease::unparse(&in_tree),
                        prettyplease::unparse(&out_tree)
                    )
                },
            ],
            [ $($compiles,)* ::quote::quote! { $($body)* }, ],
            $($tail)*
        );
    };

    // Muncher: comma skipper (allows optional commas between blocks)
    ( @munch $mode:ident, $name:ident, $snaps:tt, $compiles:tt, , $($tail:tt)* ) => {
        assert_macro_test!(@munch $mode, $name, $snaps, $compiles, $($tail)*);
    };

    // Termination
    ( @munch $mode:ident, $name:ident, [ $($snaps:expr,)* ], [ $($compiles:expr,)* ], ) => {
        paste::paste! {
            #[test]
            fn [<output_ $name>]() {
                let mut formatted_snapshots = Vec::new();
                $(
                    // $snaps is already a formatted String now, just push it!
                    formatted_snapshots.push($snaps);
                )*
                let formatted = formatted_snapshots.join("\n// ---------------------------------\n\n");
                insta::assert_snapshot!(formatted);
            }

            #[test]
            fn [<compile_ $name>]() {
                let mut combined_input = proc_macro2::TokenStream::new();
                $(
                    combined_input.extend($compiles);
                )*

                let trybuild_tokens = ::quote::quote! {
                    #[allow(unused_imports)] use einstellung::serde;
                    use einstellung_derive::Config;
                    #combined_input
                    fn main() {}
                };

                let syntax_tree: syn::File = syn::parse2(trybuild_tokens).expect("Compile parse fail");
                let formatted_code = prettyplease::unparse(&syntax_tree);

                let manifest_dir = env!("CARGO_MANIFEST_DIR");
                let dir_path = std::path::Path::new(manifest_dir).join("src").join("trybuild_tests");
                std::fs::create_dir_all(&dir_path).ok();

                let file_path = dir_path.join(format!("{}.rs", stringify!($name)));
                std::fs::write(&file_path, formatted_code).expect("Write fail");

                let t = trybuild::TestCases::new();
                match stringify!($mode) {
                    "PASS" => t.pass(&file_path),
                    "FAIL" => t.compile_fail(&file_path),
                    _ => panic!("Invalid mode"),
                }
            }
        }
    };
}

assert_macro_test!(PASS, basic_primitives: {
    #[derive(Config)]
    struct ServerConfig {
        host: String,
        port: u16,
        is_active: bool,
    }
});

assert_macro_test!(FAIL, invalid: {
    #[derive(Config)]
    struct ServerConfig(u16);
});

assert_macro_test!(FAIL, invalid_merge_foo: {
    #[derive(Config)]
    struct ServerConfig {
        host: String,
        port: u16,
        #[config(merge = "foo")]
        is_active: bool,
    }
});

assert_macro_test!(FAIL, invalid_merge_subconfig: {
    #[derive(Config)]
    struct ServerConfig {
        host: String,
        port: u16,
        #[config(merge = "subconfig")]
        is_active: bool,
    }
});

assert_macro_test!(PASS, optional_fields_no_double_option: {
    #[derive(Config)]
    struct ClientConfig {
        name: String,
        timeout_ms: Option<u32>,
        proxy: Option<String>,
    }
});

assert_macro_test!(PASS, default_values: {
    #[derive(Config)]
    struct NetworkConfig {
        #[config(default = || "localhost".to_string())]
        host: String,
        #[config(default = 8080)]
        port: u16,
        #[config(default = || std::time::Duration::from_secs(30))]
        timeout: std::time::Duration,
    }
});

assert_macro_test!(PASS, subconfig_resolution:
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

assert_macro_test!(PASS, optional_subconfig:
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

assert_macro_test!(PASS, merge_strategies: {
    #[derive(Config)]
    struct LoggerConfig {
        level: String,
        #[config(merge = "extend")]
        log_files: Vec<String>,
        #[config(merge = "replace")]
        output_format: String,
    }
});

assert_macro_test!(PASS, serde_attribute_forwarding: {
    #[derive(Config)]
    struct ApiConfig {
        #[config(serde(rename = "API_KEY"))]
        key: String,
        #[config(serde(alias = "max_retries", default))]
        retries: u8,
        #[config(serde(skip_serializing_if = "Option::is_none"))]
        endpoint: Option<String>,
    }
});

assert_macro_test!(PASS, validation_functions:
    helper {
        pub mod validators {
            pub fn validate_cert_path(_: &String) -> Result<(), Box<dyn std::error::Error>> { Ok(()) }
            pub fn validate_port(_: &u16) -> Result<(), Box<dyn std::error::Error>> { Ok(()) }
        }
    },
    {
        #[derive(Config)]
        struct TlsConfig {
            #[config(validate = validators::validate_cert_path)]
            cert_path: String,
            #[config(validate = validators::validate_port)]
            port: u16,
        }
    }
);

assert_macro_test!(PASS, kitchen_sink:
    helper {
        fn validate_system_port(_: &u16) -> Result<(), Box<dyn std::error::Error>> { Ok(()) }
    },
    {
        #[derive(Config)]
        struct FullSystemConfig {
            #[config(serde(rename = "sys_name"))]
            #[config(default = || "production".to_string())]
            name: String,
            #[config(validate = "validate_system_port")]
            port: u16,
            #[config(subconfig)]
            database: DatabaseConfig,

            #[config(merge = "extend")]
            #[config(serde(alias = "files"))]
            log_files: Vec<String>,

            #[config(merge = "extend")]
            users: std::collections::HashSet<String>,

            #[config(subconfig)]
            optional_cache: Option<CacheConfig>,
        }
    },
    {
        #[derive(Config)]
        struct DatabaseConfig {
            url: String,
            #[config(default = 5432)]
            port: u16,
        }
    },
    {
        #[derive(Config)]
        struct CacheConfig {
            #[config(default = 1024)]
            size_mb: u32,
        }
    }
);
