use quote::{ToTokens, quote};

pub fn derive(input: proc_macro2::TokenStream) -> proc_macro2::TokenStream {
    let input: syn::DeriveInput = match syn::parse2(input) {
        Ok(val) => val,
        Err(e) => return e.to_compile_error(),
    };

    // 1. Parse using darling
    let parsed = match parser::parse(input) {
        Ok(p) => p,
        Err(e) => return e.write_errors(),
    };

    // 2. Transform into our intermediate model
    let model = match transformer::transform(parsed) {
        Ok(m) => m,
        Err(e) => return e.to_compile_error(),
    };

    // 3. Generate the output TokenStream
    model.to_token_stream()
}

pub mod parser {
    use darling::{FromDeriveInput, FromField, ast, util::SpannedValue};

    #[derive(FromDeriveInput)]
    #[darling(attributes(config), supports(struct_named))]
    pub struct ConfigStructReceiver {
        pub ident: syn::Ident,
        pub vis: syn::Visibility,
        pub data: ast::Data<darling::util::Ignored, ConfigFieldReceiver>,
    }

    #[derive(FromField)]
    #[darling(attributes(config))]
    pub struct ConfigFieldReceiver {
        pub ident: Option<syn::Ident>,
        pub vis: syn::Visibility,
        pub ty: syn::Type,

        /// Captures #[config(serde(...))] nested attributes
        #[darling(default, multiple)]
        pub serde: Vec<syn::Meta>,

        #[darling(default)]
        pub default: Option<syn::Expr>,
        #[darling(default)]
        pub subconfig: bool,
        #[darling(default)]
        pub merge: Option<SpannedValue<MergeStrategy>>,
        #[darling(default)]
        pub validate: Option<syn::Path>,
    }

    #[derive(Debug, darling::FromMeta)]
    pub enum MergeStrategy {
        Replace,
        Append,
        Function(String),
        #[darling(skip)]
        Subconfig,
    }

    pub fn parse(input: syn::DeriveInput) -> Result<ConfigStructReceiver, darling::Error> {
        ConfigStructReceiver::from_derive_input(&input)
    }
}

pub mod transformer {
    use super::parser::{ConfigFieldReceiver, ConfigStructReceiver};
    use crate::derive_config::parser::MergeStrategy;
    use syn::{GenericArgument, PathArguments, Type};

    pub struct TransformedStruct {
        pub complete_ident: syn::Ident,
        pub partial_ident: syn::Ident,
        pub vis: syn::Visibility,
        pub fields: Vec<TransformedField>,
    }

    pub struct TransformedField {
        pub ident: syn::Ident,
        pub vis: syn::Visibility,
        pub partial_type: syn::Type,
        pub is_optional: bool,
        pub is_subconfig: bool,
        pub default_expr: Option<syn::Expr>,
        pub merge_strategy: MergeStrategy,
        pub validate_func: Option<syn::Path>,
        pub serde_attrs: Vec<syn::Attribute>,
    }

    pub fn transform(receiver: ConfigStructReceiver) -> syn::Result<TransformedStruct> {
        let complete_ident = receiver.ident.clone();
        let partial_ident =
            syn::Ident::new(&format!("{complete_ident}Partial"), complete_ident.span());

        let vis = receiver.vis;

        let struct_data = receiver
            .data
            .take_struct()
            .expect("Only named structs are supported");

        let mut fields = Vec::new();
        let mut errors: Option<syn::Error> = None;

        for field in struct_data {
            match transform_field(field) {
                Ok(f) => fields.push(f),
                Err(e) => {
                    if let Some(ref mut errs) = errors {
                        errs.combine(e);
                    } else {
                        errors = Some(e);
                    }
                }
            }
        }

        if let Some(err) = errors {
            return Err(err);
        }

        Ok(TransformedStruct {
            complete_ident,
            partial_ident,
            vis,
            fields,
        })
    }

    fn transform_field(field: ConfigFieldReceiver) -> syn::Result<TransformedField> {
        let ident = field.ident.clone().ok_or_else(|| {
            syn::Error::new(proc_macro2::Span::call_site(), "Named fields are required")
        })?;

        // Reconstruct #[serde(...)] attributes from nested #[config(serde(...))]
        let serde_attrs = field
            .serde
            .into_iter()
            .map(|meta| {
                syn::parse_quote! { #[serde(#meta)] }
            })
            .collect();

        let inner_type_if_optional = extract_type_from_option(&field.ty);
        let is_optional = inner_type_if_optional.is_some();
        let core_type = inner_type_if_optional.cloned().unwrap_or(field.ty);

        let partial_type = if field.subconfig {
            syn::parse_quote!(Option<<#core_type as ::einstellung::Config>::Partial>)
        } else {
            syn::parse_quote!(Option<#core_type>)
        };

        let merge_strategy = if let Some(strategy) = field.merge {
            if field.subconfig {
                return Err(syn::Error::new(
                    strategy.span(),
                    "It is invalid to specify a merge strategy on a subconfig",
                ));
            }
            strategy.into_inner()
        } else if field.subconfig {
            MergeStrategy::Subconfig
        } else {
            MergeStrategy::Replace
        };

        Ok(TransformedField {
            ident,
            vis: field.vis,
            partial_type,
            is_optional,
            is_subconfig: field.subconfig,
            default_expr: field.default,
            merge_strategy,
            validate_func: field.validate,
            serde_attrs,
        })
    }

    fn extract_type_from_option(ty: &Type) -> Option<&Type> {
        if let Type::Path(type_path) = ty
            && type_path.qself.is_none()
            && let Some(segment) = type_path.path.segments.last()
            && segment.ident == "Option"
            && let PathArguments::AngleBracketed(args) = &segment.arguments
            && let Some(GenericArgument::Type(inner_ty)) = args.args.first()
        {
            return Some(inner_ty);
        }
        None
    }
}

pub mod generator {
    use super::transformer::TransformedStruct;
    use crate::derive_config::parser::MergeStrategy;
    use proc_macro2::TokenStream;
    use quote::{ToTokens, quote};

    impl ToTokens for TransformedStruct {
        fn to_tokens(&self, tokens: &mut TokenStream) {
            generate_partial_struct(self).to_tokens(tokens);
            generate_partial_impl(self).to_tokens(tokens);
            generate_config_impl(self).to_tokens(tokens);
        }
    }

    fn generate_partial_struct(model: &TransformedStruct) -> TokenStream {
        let partial_ident = &model.partial_ident;
        let vis = &model.vis;

        let fields = model.fields.iter().map(|f| {
            let ident = &f.ident;
            let ty = &f.partial_type;
            let attrs = &f.serde_attrs;
            let f_vis = &f.vis;
            quote! {
                #(#attrs)*
                #f_vis #ident: #ty
            }
        });

        quote! {
            #[derive(Default, Debug, ::einstellung::serde::Deserialize)]
            #[serde(crate = "::einstellung::serde")]
            #vis struct #partial_ident {
                #(#fields,)*
            }
        }
    }

    fn generate_partial_impl(model: &TransformedStruct) -> TokenStream {
        let partial_ident = &model.partial_ident;
        let complete_ident = &model.complete_ident;

        let merge_fields = model.fields.iter().map(|f| {
            let ident = &f.ident;
            match f.merge_strategy {
                MergeStrategy::Replace => quote! {
                    #ident: next.#ident.or(self.#ident)
                },
                MergeStrategy::Append => quote! {
                    #ident: match (self.#ident, next.#ident) {
                        (Some(mut a), Some(b)) => {
                            a.extend(b);
                            Some(a)
                        },
                        (a, b) => a.or(b)
                    }
                },
                MergeStrategy::Function(ref func_name) => {
                    let func_path: syn::Path =
                        syn::parse_str(func_name).expect("Invalid merge function path");
                    quote! {
                        #ident: #func_path(self.#ident, next.#ident)
                    }
                }
                MergeStrategy::Subconfig => quote! {
                    #ident: match (self.#ident, next.#ident) {
                        (Some(a), Some(b)) => ::einstellung::PartialConfig::merge(a, b),
                        (a, b) => a.or(b)
                    }
                },
            }
        });

        let build_fields = model.fields.iter().map(|f| {
            let ident = &f.ident;
            let ident_str = ident.to_string();

            let resolve = if f.is_subconfig {
                if f.is_optional {
                    quote! { self.#ident.map(::einstellung::PartialConfig::build).transpose()? }
                } else {
                    quote! { self.#ident.unwrap_or_default().build()? }
                }
            } else if let Some(syn::Expr::Lit(default_literal)) = &f.default_expr {
                quote! { self.#ident.unwrap_or(#default_literal) }
            } else if let Some(default_expr) = &f.default_expr {
                quote! { self.#ident.unwrap_or_else(|| #default_expr) }
            } else if f.is_optional {
                quote! { self.#ident }
            } else {
                quote! { self.#ident.ok_or(::einstellung::ConfigError::MissingField(#ident_str))? }
            };

            if let Some(validate_func) = &f.validate_func {
                quote! {
                    let #ident = #resolve;
                    if let Err(e) = #validate_func(&#ident) {
                        return Err(::einstellung::ConfigError::Validation {
                            field: #ident_str,
                            reason: e.to_string(),
                        });
                    }
                }
            } else {
                quote! { let #ident = #resolve; }
            }
        });

        let field_names = model.fields.iter().map(|f| &f.ident);

        quote! {
            impl ::einstellung::PartialConfig for #partial_ident {
                type Complete = #complete_ident;

                fn merge(self, next: Self) -> Self {
                    Self { #(#merge_fields,)* }
                }

                fn build(self) -> Result<Self::Complete, ::einstellung::ConfigError> {
                    #(#build_fields)*
                    Ok(#complete_ident { #(#field_names,)* })
                }
            }
        }
    }

    fn generate_config_impl(model: &TransformedStruct) -> TokenStream {
        let complete_ident = &model.complete_ident;
        let partial_ident = &model.partial_ident;

        quote! {
            impl ::einstellung::Config for #complete_ident {
                type Partial = #partial_ident;
            }
        }
    }
}
