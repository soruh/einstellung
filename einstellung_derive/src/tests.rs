use proc_macro2::TokenStream;

macro_rules! assert_expansion {
    ( $( { $($tokens:tt)* } ),+ $(,)? ) => {{

        let formatted = [
            $( quote::quote! { $($tokens)* } ),+
        ]
        .map(|input| {
            let output = crate::derive_config::derive(input.clone());

            let combined = quote::quote! {
                /// --- input ---
                #input

                /// --- output ---
                #output
            };

            let syntax_tree: syn::File = match syn::parse2(combined) {
                Ok(res) => res,
                Err(err) => {
                    let combined = quote::quote! {
                        #input
                        #output
                    };

                    let fmt = format_tokenstream_fallback(combined).unwrap_or_else(|err| panic!("invalid output: {err}"));

                    panic!("Combined input and output is not valid Rust syntax: {err}\n{fmt}")
                }
            };


            prettyplease::unparse(&syntax_tree)
        })
        .join("\n// ---------------------------------\n");

        insta::assert_snapshot!(formatted);
    }};
}

pub fn format_tokenstream_fallback(ts: TokenStream) -> Result<String, Box<dyn std::error::Error>> {
    use std::io::Write;

    let code = ts.to_string();

    let mut rustfmt = std::process::Command::new("rustfmt")
        .args(["--emit", "stdout"])
        .args(["--color", "always"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;

    {
        let stdin = rustfmt.stdin.as_mut().ok_or("Failed to open stdin")?;
        stdin.write_all(code.as_bytes())?;
    }

    let output = rustfmt.wait_with_output()?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(err.into());
    }

    Ok(String::from_utf8(output.stdout)?)
}

#[test]
fn test_basic_primitives() {
    assert_expansion!({
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
    assert_expansion!({
        #[derive(Config)]
        struct ServerConfig(u16);
    });
}

#[test]
fn test_invalid_merge() {
    assert_expansion!({
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
    assert_expansion!({
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
    assert_expansion!({
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
    assert_expansion!({
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
    assert_expansion!(
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
    assert_expansion!(
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
    assert_expansion!({
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
    assert_expansion!({
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
    assert_expansion!({
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
    assert_expansion!(
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
