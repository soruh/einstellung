use crate::derive_config::transformer::{FallbackStrategy, FieldKind, TransformedStruct};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote, quote_spanned};
use std::fmt::Write;
use syn::{parse_quote_spanned, spanned::Spanned};

impl ToTokens for TransformedStruct {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        generate_partial_struct(self).to_tokens(tokens);
        generate_partial_impl(self).to_tokens(tokens);
        generate_config_impl(self).to_tokens(tokens);
    }
}

fn path_to_litstr(path: &syn::Path) -> syn::LitStr {
    let mut s = String::new();
    let mut iter = path.segments.iter();

    if path.leading_colon.is_some() {
        s += "::";
    }

    if let Some(first) = iter.next() {
        write!(&mut s, "{}", first.ident).unwrap();
        for seg in iter {
            write!(&mut s, "::{}", seg.ident).unwrap();
        }
    }

    syn::LitStr::new(&s, path.span())
}

fn generate_partial_struct(model: &TransformedStruct) -> TokenStream {
    let partial_ident = &model.partial_ident;
    let vis = &model.vis;
    let einstellung = &model.einstellung;

    let fields = model.fields.iter().map(|f| {
        let ident = &f.ident;
        let attrs = &f.serde_attrs;
        let f_vis = &f.vis;
        let partial_type = &f.partial_type;

        let needs_serde_default = if let FieldKind::Extend { complete_is_optional, .. } = &f.kind {
            !complete_is_optional
        } else {
            false
        };

        let default_attr = if needs_serde_default {
            quote! { #[serde(default)] }
        } else {
            quote! {}
        };

        quote_spanned! { partial_type.span() =>
            #default_attr
            #(#attrs)*
            #f_vis #ident: #partial_type
        }
    });

    let serde: syn::Path = parse_quote_spanned!(einstellung.span() => #einstellung::serde);
    let serde_lit = path_to_litstr(&serde);

    quote! {
        #[derive(Default, Debug, #einstellung::serde::Deserialize)]
        #[serde(crate = #serde_lit)]
        #vis struct #partial_ident {
            #(#fields,)*
        }
    }
}

fn generate_partial_impl(model: &TransformedStruct) -> TokenStream {
    let partial_ident = &model.partial_ident;
    let complete_ident = &model.complete_ident;
    let einstellung = &model.einstellung;
    let complete_str = complete_ident.to_string();

    let merge_fields = model.fields.iter().map(|f| {
        let ident = &f.ident;
        let partial_type = &f.partial_type;

        match &f.kind {
            FieldKind::Replace { .. } => quote! { #ident: next.#ident.or(self.#ident) },
            FieldKind::Extend {
                partial_is_optional,
                ..
            } => {
                if *partial_is_optional {
                    quote! {
                        #ident: match (self.#ident, next.#ident) {
                            (Some(mut a), Some(b)) => {
                                ::core::iter::Extend::extend(&mut a, b);
                                Some(a)
                            },
                            (a, b) => a.or(b)
                        }
                    }
                } else {
                    quote! {
                        #ident: {
                            let mut #ident = self.#ident;
                            #ident.extend(next.#ident);
                            #ident
                        }
                    }
                }
            }
            FieldKind::CustomMerge { func_path, .. } => {

                let span = func_path.span();
                let func_path: &syn::Path = func_path;

                quote_spanned! { span =>
                    #ident: {
                        let _: #einstellung::MergeFunction<#partial_type> = #func_path;
                        #func_path(self.#ident, next.#ident)
                    }
                }
            },
            FieldKind::Subconfig { .. } => quote! {
                #ident: match (self.#ident, next.#ident) {
                    (Some(a), Some(b)) => Some(#einstellung::PartialConfig::merge(a, b)),
                    (a, b) => a.or(b)
                }
            },
        }
    });

    let build_fields = model.fields.iter().map(|f| {
        let ident = &f.ident;
        let ident_str = ident.to_string();
        let complete_type = &f.complete_type;

        let resolve = match &f.kind {
            FieldKind::Subconfig { complete_is_optional, .. } => {
                if *complete_is_optional {
                    quote! { self.#ident.map(|x| #einstellung::build_with_context(x, #complete_str, #ident_str)).transpose()? }
                } else {
                    quote! { #einstellung::build_with_context(self.#ident.unwrap_or_default(), #complete_str, #ident_str)? }
                }
            }
            FieldKind::Extend { partial_is_optional, complete_is_optional, .. } => {
                if *partial_is_optional && !*complete_is_optional {
                    quote! { self.#ident.ok_or(#einstellung::ConfigError::MissingField(#einstellung::FieldPath::new(#complete_str, #ident_str)))? }
                } else {
                    quote! { self.#ident }
                }
            }
            FieldKind::Replace { fallback, .. } | FieldKind::CustomMerge { fallback, .. } => match fallback {
                FallbackStrategy::Require => quote! { self.#ident.ok_or(#einstellung::ConfigError::MissingField(#einstellung::FieldPath::new(#complete_str, #ident_str)))? },
                FallbackStrategy::Keep => quote! { self.#ident },
                FallbackStrategy::Value(value) => quote! { self.#ident.unwrap_or(#value) },
                FallbackStrategy::Call(value) => quote! { self.#ident.unwrap_or_else(#value) },
                FallbackStrategy::Standard => quote! { self.#ident.unwrap_or_else(::core::Default::default) },
            }
        };


        if let Some(validate_func) = &f.validate_func {
            quote_spanned! { complete_type.span() => 

                let #ident: #complete_type = #resolve;
                let _: #einstellung::ValidationFunction<#complete_type> = #validate_func;
                if let Err(e) = (#validate_func)(&#ident) {
                    return Err(#einstellung::ConfigError::Validation {
                        field: #einstellung::FieldPath::new(#complete_str, #ident_str),
                        reason: e.into(),
                    });
                }
            }
        } else {
            quote_spanned! { complete_type.span() => let #ident = #resolve; }
        }
    });

    let field_names = model.fields.iter().map(|f| &f.ident);

    quote_spanned! { partial_ident.span() =>
        impl #einstellung::PartialConfig for #partial_ident {
            type Complete = #complete_ident;

            fn merge(self, next: Self) -> Self {
                Self { #(#merge_fields,)* }
            }

            fn build(self) -> Result<Self::Complete, #einstellung::ConfigError> {
                #(#build_fields)*
                Ok(#complete_ident { #(#field_names,)* })
            }
        }
    }
}

fn generate_config_impl(model: &TransformedStruct) -> TokenStream {
    let complete_ident = &model.complete_ident;
    let partial_ident = &model.partial_ident;
    let einstellung = &model.einstellung;

    quote_spanned! { complete_ident.span() =>
        impl #einstellung::Config for #complete_ident {
            type Partial = #partial_ident;
        }
    }
}
