use super::transformer::{
    BuildStategy, DefaultInitializer, FreezeStrategy, MergeStrategy, PartialType, TransformedField,
    TransformedStruct,
};
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

/// Converts a syn::Path into a literal string for use in attributes like #[serde(crate = "...")]
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

/// Renders the Rust type for a partial field based on the PartialType metadata
fn render_partial_type(pt: &PartialType, einstellung: &syn::Path) -> TokenStream {
    let core = &pt.core_type;

    // Determine the base type: either the core type or its Partial counterpart
    let mut tokens = if pt.access_partial {
        quote!(<#core as #einstellung::Config>::Partial)
    } else {
        quote!(#core)
    };

    // Apply Freeze wrapping if the strategy requires it
    if pt.wrap_freeze {
        tokens = quote!(#einstellung::Freeze<#tokens>);
    }

    // Wrap in Option as partial fields are technically always optional during merging
    if pt.wrap_option {
        // todo
        // tokens = quote!(::core::option::Option<#tokens>);
        tokens = quote!(Option<#tokens>);
    }

    tokens
}

fn generate_partial_struct(model: &TransformedStruct) -> TokenStream {
    let partial_ident = &model.partial_ident;
    let vis = &model.vis;
    let einstellung = &model.einstellung;
    let attrs = &model.attrs;

    let fields = model.fields.iter().map(|f| {
        let ident = &f.ident;
        let f_attrs = &f.attrs;
        let f_vis = &f.vis;
        let ty = render_partial_type(&f.partial_type, einstellung);

        quote! {
            #(#f_attrs)*
            #f_vis #ident: #ty
        }
    });

    let serde_path: syn::Path = parse_quote_spanned!(einstellung.span() => #einstellung::serde);
    let serde_lit = path_to_litstr(&serde_path);

    quote! {
        #[derive(::core::default::Default, #einstellung::serde::Deserialize)]
        #(#[#attrs])*
        #[serde(crate = #serde_lit)]
        #vis struct #partial_ident {
            #(#fields,)*
        }
    }
}

fn generate_field_merge(
    f: &TransformedField,
    einstellung: &syn::Path,
    complete_str: &str,
    left: TokenStream,
    right: TokenStream,
) -> TokenStream {
    match &f.merge {
        MergeStrategy::Replace => quote! { #right.or(#left) },
        MergeStrategy::Extend => quote! {
            match (#left, #right) {
                (Some(mut a), Some(b)) => {
                    ::core::iter::Extend::extend(&mut a, b);
                    Some(a)
                },
                (a, b) => a.or(b)
            }
        },
        MergeStrategy::Custom(func_path) => {
            let ident_str = f.ident.to_string();
            let partial_type = render_partial_type(&f.partial_type, einstellung);

            quote_spanned!(func_path.span() => {
                let _: #einstellung::MergeFunction<#partial_type, _> = #func_path;
                #func_path(#left, #right).map_err(|reason| #einstellung::ConfigError::CustomMerge {
                    field: #einstellung::FieldPath::new(#complete_str, #ident_str),
                    reason,
                })
            })
        }
        MergeStrategy::MergeSubconfig => quote! {
            match (#left, #right) {
                (Some(a), Some(b)) => Some(#einstellung::PartialConfig::merge(a, b)?),
                (a, b) => a.or(b)
            }
        },
    }
}

fn generate_partial_impl(model: &TransformedStruct) -> TokenStream {
    let partial_ident = &model.partial_ident;
    let complete_ident = &model.complete_ident;
    let einstellung = &model.einstellung;
    let complete_str = complete_ident.to_string();

    let merge_fields = model.fields.iter().map(|f| {
        let ident = &f.ident;

        match f.freeze {
            FreezeStrategy::NotFreezable => {
                let merged = generate_field_merge(f, einstellung, &complete_str, quote!(self.#ident), quote!(next.#ident));
                quote! { #ident: #merged }
            }
            FreezeStrategy::Wrapped => {
                let ident_str = ident.to_string();
                let merge = generate_field_merge(f, einstellung, &complete_str, quote!(left), quote!(right));

                quote!{
                    #ident: match #einstellung::FreezeCombination::of_freeze(self.#ident, next.#ident) {
                        #einstellung::FreezeCombination::BothFree(left, right) => #einstellung::Freeze::Free(#merge),
                        #einstellung::FreezeCombination::OneFrozen(x) => #einstellung::Freeze::Frozen(x),
                        #einstellung::FreezeCombination::BothFrozen => return ::core::result::Result::Err(#einstellung::ConfigError::FreezeCollision(#einstellung::FieldPath::new(#complete_str, #ident_str))),
                    }
                }
            },
            FreezeStrategy::IntrinsicallyFreezable => {
                let ident_str = ident.to_string();
                let merge = generate_field_merge(f, einstellung, &complete_str, quote!(left), quote!(right));

                quote!{
                    #ident: match #einstellung::FreezeCombination::of(self.#ident, next.#ident) {
                        #einstellung::FreezeCombination::BothFree(left, right) => #merge,
                        #einstellung::FreezeCombination::OneFrozen(x) => x,
                        #einstellung::FreezeCombination::BothFrozen => return ::core::result::Result::Err(#einstellung::ConfigError::FreezeCollision(#einstellung::FieldPath::new(#complete_str, #ident_str))),
                    }
                }
            },
        }
    });

    let build_fields = model.fields.iter().map(|f| {
        let ident = &f.ident;
        let ident_str = ident.to_string();
        let complete_type = &f.complete_type;

        let unfreeze = if f.freeze == FreezeStrategy::Wrapped {
            quote! { #einstellung::Freeze::into_inner(self.#ident) }
        } else {
            quote! { self.#ident }
        };

        let resolve = match &f.build {
            BuildStategy::SubconfigOptional => quote! { #unfreeze.map(|x| #einstellung::build_with_context(x, #complete_str, #ident_str)).transpose()? },
            BuildStategy::SubconfigRequired => quote! { #einstellung::build_with_context(#unfreeze.unwrap_or_default(), #complete_str, #ident_str)? },
            BuildStategy::Required => quote! { #unfreeze.ok_or(#einstellung::ConfigError::MissingField(#einstellung::FieldPath::new(#complete_str, #ident_str)))? },
            BuildStategy::Optional => quote! { #unfreeze },
            BuildStategy::Default(init) => match init {
                DefaultInitializer::Value(val) => quote! { #unfreeze.unwrap_or(#val) },
                DefaultInitializer::Call(func) => quote! { #unfreeze.unwrap_or_else(#func) },
                DefaultInitializer::DefaultTrait => quote! { #unfreeze.unwrap_or_else(::core::default::Default::default) },
            },
        };

        if let Some(validate_func) = &f.validate_func {
            quote_spanned! { complete_type.span() =>
                let #ident: #complete_type = #resolve;
                let _: #einstellung::ValidationFunction<#complete_type, _> = #validate_func;
                if let Err(e) = (#validate_func)(&#ident) {
                    return Err(#einstellung::ConfigError::Validation {
                        field: #einstellung::FieldPath::new(#complete_str, #ident_str),
                        #[allow(clippy::useless_conversion)]
                        reason: e.into(),
                    });
                }
            }
        } else {
            quote_spanned! { complete_type.span() => let #ident = #resolve; }
        }
    });

    let freeze_fields = model.fields.iter().map(|f| {
        let ident = &f.ident;
        match f.freeze {
            FreezeStrategy::NotFreezable => quote! { #ident: self.#ident },
            _ => quote! { #ident: #einstellung::Freezable::freeze(self.#ident) },
        }
    });

    let is_field_frozen = model.fields.iter().filter_map(|f| {
        let ident = &f.ident;
        match f.freeze {
            FreezeStrategy::NotFreezable => None,
            _ => Some(quote! { #einstellung::Freezable::is_frozen(&self.#ident) }),
        }
    });

    let freezable_impl = if model.any_freezable {
        quote! {
            #[automatically_derived]
            impl #einstellung::Freezable for #partial_ident {
                fn freeze(self) -> Self { Self { #(#freeze_fields,)* } }
                fn is_frozen(&self) -> bool { #(#is_field_frozen)||* }
            }
        }
    } else {
        quote! {}
    };

    let field_names = model.fields.iter().map(|f| &f.ident);

    quote_spanned! { partial_ident.span() =>
        #[automatically_derived]
        impl #einstellung::PartialConfig for #partial_ident {
            type Complete = #complete_ident;

            fn merge(self, next: Self) -> ::core::result::Result<Self, #einstellung::ConfigError> {
                ::core::result::Result::Ok(Self { #(#merge_fields,)* })
            }

            fn build(self) -> ::core::result::Result<Self::Complete, #einstellung::ConfigError> {
                #(#build_fields)*
                ::core::result::Result::Ok(#complete_ident { #(#field_names,)* })
            }
        }
        #freezable_impl
    }
}

fn generate_config_impl(model: &TransformedStruct) -> TokenStream {
    let complete_ident = &model.complete_ident;
    let partial_ident = &model.partial_ident;
    let einstellung = &model.einstellung;

    quote_spanned! { complete_ident.span() =>
        #[automatically_derived]
        impl #einstellung::Config for #complete_ident {
            type Partial = #partial_ident;
        }
    }
}
