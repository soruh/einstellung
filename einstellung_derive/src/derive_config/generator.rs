use crate::derive_config::{
    parser::DefaultStrategy,
    transformer::{ResolvedMerge, TransformedStruct},
};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::{parse_quote_spanned, spanned::Spanned};

impl ToTokens for TransformedStruct {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        generate_partial_struct(self).to_tokens(tokens);
        generate_partial_impl(self).to_tokens(tokens);
        generate_config_impl(self).to_tokens(tokens);
    }
}

use std::fmt::Write;

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
        let ty = &f.partial_type;
        let attrs = &f.serde_attrs;
        let f_vis = &f.vis;

        let default = if f.merge_strategy == ResolvedMerge::Extend && !f.is_optional {
            quote! { #[serde(default)] }
        } else {
            quote! {}
        };

        quote! {
            #default
            #(#attrs)*
            #f_vis #ident: #ty
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
        match &f.merge_strategy {
            ResolvedMerge::Replace => quote! {
                #ident: next.#ident.or(self.#ident)
            },
            ResolvedMerge::Extend => {
                if f.is_optional {
                    quote! {
                        #ident: match (self.#ident, next.#ident) {
                        (Some(mut a), Some(b)) => {
                            a.extend(b);
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
            ResolvedMerge::Function(path) => quote! {
                #ident: #path(self.#ident, next.#ident)
            },
            ResolvedMerge::Subconfig => quote! {
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

        let resolve = if f.is_subconfig {
            if f.is_optional {
                quote! { self.#ident.map(|x| #einstellung::build_with_context(x, #complete_str, #ident_str)).transpose()? }
            } else {
                quote! { #einstellung::build_with_context(self.#ident.unwrap_or_default(), #complete_str, #ident_str)? }
            }
        } else if f.merge_strategy == ResolvedMerge::Extend {
            quote! { self.#ident }
        } else if let DefaultStrategy::Value(value) = &f.default_expr {
            quote! { self.#ident.unwrap_or(#value) }
        } else if let DefaultStrategy::Call(value) = &f.default_expr {
            quote! { self.#ident.unwrap_or_else(#value) }
        } else if let DefaultStrategy::Inherit = &f.default_expr {
            quote! { self.#ident.unwrap_or_else(::core::Default::default) }
        } else {
            if f.is_optional {
                quote! { self.#ident }
            } else {
                quote! { self.#ident.ok_or(#einstellung::ConfigError::MissingField(#einstellung::FieldPath::new(#complete_str, #ident_str)))? }
            }
        };

        if let Some(validate_func) = &f.validate_func {
            quote! {
                let #ident = #resolve;
                if let Err(e) = (#validate_func)(&#ident) {
                    return Err(#einstellung::ConfigError::Validation {
                        field: #einstellung::FieldPath::new(#complete_str, #ident_str),
                        reason: e.into(),
                    });
                }
            }
        } else {
            quote! { let #ident = #resolve; }
        }
    });

    let field_names = model.fields.iter().map(|f| &f.ident);

    quote! {
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

    quote! {
        impl #einstellung::Config for #complete_ident {
            type Partial = #partial_ident;
        }
    }
}
