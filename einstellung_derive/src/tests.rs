macro_rules! assert_expansion {
    ( $( { $($tokens:tt)* } ),+ $(,)? ) => {{
        let formatted = [
            $( ::quote::quote! { $($tokens)* } ),+
        ]
        .into_iter() // Ensure we are iterating
        .map(|input| {
            let output = crate::derive_config::expand(input.clone());
            
            // Combine them into one stream
            let combined = ::quote::quote! { 
                #input 
                #output 
            };
            
            let syntax_tree: ::syn::File = ::syn::parse2(combined)
                .expect("Combined input and output is not valid Rust syntax");
            
            ::prettyplease::unparse(&syntax_tree)
        })
        .collect::<Vec<_>>()
        .join("\n// ---------------------------------\n");

        ::insta::assert_snapshot!(formatted);
    }};
}

#[test]
fn basic_struct() {
    assert_expansion!({
        struct BasicConfig {
            host: String,
            port: u16,
        }
    });
}
#[test]
fn default_values() {
    assert_expansion!({
        struct DefaultConfig {
            #[config(default = "localhost")]
            host: String,

            #[config(default = 8080)]
            port: u16,
        }
    });
}
#[test]
fn required_fields() {
    assert_expansion!({
        struct RequiredConfig {
            #[config(required)]
            host: String,

            #[config(required)]
            port: u16,
        }
    });
}
#[test]
fn nested_config() {
    assert_expansion!(
        {
            struct DatabaseConfig {
                url: String,
            }
        },
        {
            struct AppConfig {
                database: DatabaseConfig,
                #[config(default = 8080)]
                port: u16,
            }
        },
    );
}
#[test]
fn optional_fields() {
    assert_expansion!({
        struct OptionalConfig {
            host: Option<String>,
            port: Option<u16>,
        }
    });
}
#[test]
fn collection_fields() {
    assert_expansion!({
        struct CollectionConfig {
            servers: Vec<String>,
            retries: Vec<u32>,
        }
    });
}
#[test]
fn rename_fields() {
    assert_expansion!({
        struct RenameConfig {
            #[config(rename = "server_host")]
            host: String,

            #[config(rename = "server_port")]
            port: u16,
        }
    });
}
#[test]
fn validation_method() {
    assert_expansion!({
        #[config(validate = "Self::validate")]
        struct ValidateConfig {
            port: u16
        }
    });
}
#[test]
fn complex_interaction1() {
    assert_expansion!(
        {
            struct DatabaseConfig {
                #[config(required)]
                url: String,
            }
        },
        {
            struct AppConfig {
                #[config(rename = "db_config")]
                database: DatabaseConfig,

                #[config(default = 3000)]
                port: u16,
            }
        },
    );
}
#[test]
fn complex_interaction2() {
    assert_expansion!({
        struct NetworkConfig {
            #[config(default = "127.0.0.1")]
            host: Option<String>,

            #[config(default = 8080)]
            ports: Vec<u16>,
        }
    });
}
#[test]
fn nested_layers() {
    assert_expansion!(
        {
            struct DBConfig {
                url: String,
            }
        },
        {
            struct BackendConfig {
                database: DBConfig,
            }
        },
        {
            struct AppConfig {
                backend: BackendConfig,
                port: u16,
            }
        },
    );
}
#[test]
fn empty_struct() {
    assert_expansion!({
        struct EmptyConfig {}
    });
}
#[test]
fn mix_option_vec_default_required() {
    assert_expansion!({
        struct MixedConfig {
            #[config(required)]
            url: String,

            #[config(default = 5)]
            retries: u32,

            cache_servers: Option<Vec<String>>,
        }
    });
}
#[test]
fn full_feature_struct() {
    assert_expansion!(
        {
            #[config(validate = "Self::validate")]
            struct FullConfig {
                #[config(required)]
                name: String,

                #[config(default = 8080)]
                port: u16,

                #[config(rename = "db_config")]
                database: DatabaseConfig,

                log_files: Option<Vec<String>>,
            }
        },
        {
            struct DatabaseConfig {
                #[config(required)]
                url: String,

                #[config(default = "postgres")]
                driver: String,
            }

        },
    );
}
#[test]
fn multiple_structs() {
    assert_expansion!(
        { struct ConfigA { a: String }},
        { struct ConfigB { b: u32 }},
        { struct ConfigC { c: Option<Vec<String>> }},
    );
}
#[test]
fn literal_types() {
    assert_expansion!({
        struct LiteralConfig {
            #[config(default = 42)]
            max_connections: u32,

            #[config(default = "info")]
            log_level: String,

            #[config(default = true)]
            enabled: bool,
        }
    });
}
