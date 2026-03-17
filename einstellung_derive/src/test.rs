macro_rules! assert_macro_test {
    (
        $mode:ident:
        $( { $($tokens:tt)* } ),+ $(,)?
    ) => {{
        use std::io::Write;
        use quote::quote;
        use syn;
        use prettyplease;
        use trybuild;

        let mut formatted_snapshots = Vec::new();
        let mut combined_input = quote! {};

        $(
            let input = quote! { $($tokens)* };
            let output = crate::derive_config::derive(input.clone());

            // --- snapshot ---
            let combined_snapshot = quote! {
                /// --- input ---
                #input

                /// --- output ---
                #output
            };
            let syntax_tree: syn::File = syn::parse2(combined_snapshot.clone())
                .unwrap_or_else(|err| panic!("invalid combined syntax: {err}\n{combined_snapshot}"));
            formatted_snapshots.push(prettyplease::unparse(&syntax_tree));

            // --- accumulate input for trybuild ---
            combined_input.extend(input);
        )+

        // --- snapshot assertion ---
        let formatted = formatted_snapshots.join("\n// ---------------------------------\n");
        insta::assert_snapshot!(formatted);

        let trybuild_input = quote! {
            use einstellung_derive::Config;

            #combined_input
            fn main() {}
        };

        // --- write to tempfile ---
        let mut temp_file = tempfile::NamedTempFile::new()
            .expect("failed to create tempfile");
        writeln!(temp_file, "{}", trybuild_input)
            .expect("failed to write to tempfile");

        let path = temp_file.path().to_str().unwrap();

        // --- run trybuild ---
        let t = trybuild::TestCases::new();
        match stringify!($mode) {
            "PASS" => t.pass(path),
            "FAIL" => t.compile_fail(path),
            other => panic!("invalid mode: {}", other),
        }

        // keep tempfile alive until trybuild reads it
        temp_file.keep().ok();
    }};
}

#[test]
fn test_basic_primitives() {
    assert_macro_test!(PASS: {
        #[derive(Config)]
        struct ServerConfig {
            host: String,
            port: u16,
            is_active: bool,
        }
    });
}

#[test]
fn test_invalid() {
    assert_macro_test!(PASS: {
        #[derive(Config)]
        struct ServerConfig(u16);
    });
}

#[test]
fn test_invalid_merge() {
    assert_macro_test!(PASS: {
        #[derive(Config)]
        struct ServerConfig {
            host: String,
            port: u16,
            #[config(merge = "foo")]
            is_active: bool,
        }
    });
}

#[test]
fn test_invalid_merge2() {
    assert_macro_test!(PASS: {
        #[derive(Config)]
        struct ServerConfig {
            host: String,
            port: u16,
            #[config(merge = "subconfig")]
            is_active: bool,
        }
    });
}

#[test]
fn test_optional_fields_no_double_option() {
    // Ensures Option<T> is handled correctly and doesn't become Option<Option<T>>
    assert_macro_test!(PASS: {
        #[derive(Config)]
        struct ClientConfig {
            name: String,
            timeout_ms: Option<u32>,
            proxy: Option<String>,
        }
    });
}

#[test]
fn test_default_values() {
    // Tests both primitive defaults and function call defaults
    assert_macro_test!(PASS: {
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
}

#[test]
fn test_subconfig_resolution() {
    // Tests the `subconfig` flag and multiple structs in the expansion
    assert_macro_test!(PASS:
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
}

#[test]
fn test_optional_subconfig() {
    // Tests a subconfig that is wrapped in an Option
    assert_macro_test!(PASS:
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
}

#[test]
fn test_merge_strategies() {
    // Tests the "append" and default "replace" merge behaviors
    assert_macro_test!(PASS: {
        #[derive(Config)]
        struct LoggerConfig {
            level: String,

            #[config(merge = "append")]
            log_files: Vec<String>,

            #[config(merge = "replace")]
            output_format: String,
        }
    });
}

#[test]
fn test_validation_functions() {
    // Tests the injection of validation functions in the build() step
    assert_macro_test!(PASS: {
        #[derive(Config)]
        struct TlsConfig {
            #[config(validate = "crate::validators::validate_cert_path")]
            cert_path: String,

            #[config(validate = "crate::validators::validate_port")]
            port: u16,
        }
    });
}

#[test]
fn test_serde_attribute_forwarding() {
    // Tests that darling's forward_attrs correctly passes #[serde(...)]
    // down to the generated Partial struct.
    assert_macro_test!(PASS: {
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
}

#[test]
fn test_kitchen_sink() {
    // A massive struct testing the interaction of all attributes combined
    assert_macro_test!(PASS:
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
}
