use crate::derive_config::{parser::DefaultStrategy, transformer::UnwrapStrategy};

use super::transformer::{
    FreezeStrategy, MergeStrategy, PartialType, TransformedField, TransformedStruct,
};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote, quote_spanned};
use std::fmt::Write;
use syn::{parse_quote_spanned, spanned::Spanned};

impl ToTokens for TransformedStruct {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        generate_partial_struct(self).to_tokens(tokens);
        generate_partial_impl(self).to_tokens(tokens);
        generate_freezable_impl(self).to_tokens(tokens);
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

    // Wrap in Option as partial fields are technically always optional during merging
    if pt.wrap_option {
        tokens = quote!(::core::option::Option<#tokens>);
    }

    // Apply Freeze wrapping if the strategy requires it
    if pt.wrap_freeze {
        tokens = quote!(#einstellung::Freeze<#tokens>);
    }

    tokens
}

/// Generate the associated Partial as described by the `TransformedStruct`
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
        #(#attrs)*
        #[serde(crate = #serde_lit)]
        #vis struct #partial_ident {
            #(#fields,)*
        }
    }
}

/// Generate the code to merge the field `left` with the field `right`
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

/// Generate the code to build a single field in a `PartialConfig::build` impl
fn generate_build_for_field(
    einstellung: &syn::Path,
    complete_type_name: &str,
    f: &TransformedField,
) -> TokenStream {
    let ident = &f.ident;
    let ident_str = ident.to_string();
    let complete_type = &f.complete_type;

    let unfreeze = if f.freeze == FreezeStrategy::Wrapped {
        quote! { #einstellung::Freeze::into_inner(self.#ident) }
    } else {
        quote! { self.#ident }
    };

    let built = if f.build.build {
        quote! { #unfreeze.map(|x| #einstellung::build_with_context(x, #complete_type_name, #ident_str)).transpose()? }
    } else {
        quote! { #unfreeze }
    };

    let resolve = match &f.build.unwrap {
        UnwrapStrategy::DontUnwrap => quote! { #built },
        UnwrapStrategy::Unwrap => {
            quote! { #built.ok_or(#einstellung::ConfigError::MissingField(#einstellung::FieldPath::new(#complete_type_name, #ident_str)))? }
        }
        UnwrapStrategy::UnwrapWithDefault(default) => match default {
            DefaultStrategy::Value(val) => quote! { #built.unwrap_or(#val) },
            DefaultStrategy::Call(func) => quote! { #built.unwrap_or_else(#func) },
            DefaultStrategy::DefaultTrait => {
                quote! { #built.unwrap_or_else(::core::default::Default::default) }
            }
        },
    };

    let validated = if let Some(validate_func) = &f.validate_func {
        quote_spanned!(validate_func.span() => {
            let #ident: #complete_type = #resolve;
            let _: #einstellung::ValidationFunction<#complete_type, _> = #validate_func;
            if let Err(e) = (#validate_func)(&#ident) {
                return Err(#einstellung::ConfigError::Validation {
                    field: #einstellung::FieldPath::new(#complete_type_name, #ident_str),
                    #[allow(clippy::useless_conversion)]
                    reason: e.into(),
                });
            }
            #ident
        })
    } else {
        resolve
    };

    quote_spanned!(ident.span() => #ident: #validated)
}

/// Generate the code to merge a single field in a `PartialConfig::merge` impl
fn generate_merge_for_field(
    einstellung: &syn::Path,
    complete_type_name: &str,
    f: &TransformedField,
) -> TokenStream {
    let ident = &f.ident;

    let merged = match f.freeze {
        FreezeStrategy::NotFreezable => generate_field_merge(
            f,
            einstellung,
            complete_type_name,
            quote!(self.#ident),
            quote!(next.#ident),
        ),
        FreezeStrategy::Wrapped => {
            let ident_str = ident.to_string();
            let merge = generate_field_merge(
                f,
                einstellung,
                complete_type_name,
                quote!(left),
                quote!(right),
            );

            quote! {
                match #einstellung::FreezeCombination::of_freeze(self.#ident, next.#ident) {
                    #einstellung::FreezeCombination::BothFree(left, right) => #einstellung::Freeze::Free(#merge),
                    #einstellung::FreezeCombination::OneFrozen(x) => #einstellung::Freeze::Frozen(x),
                    #einstellung::FreezeCombination::BothFrozen => return ::core::result::Result::Err(#einstellung::ConfigError::FreezeCollision(#einstellung::FieldPath::new(#complete_type_name, #ident_str))),
                }
            }
        }
        FreezeStrategy::IntrinsicallyFreezable => {
            let ident_str = ident.to_string();
            let merge = generate_field_merge(
                f,
                einstellung,
                complete_type_name,
                quote!(left),
                quote!(right),
            );

            quote! {
                match #einstellung::FreezeCombination::of(self.#ident, next.#ident) {
                    #einstellung::FreezeCombination::BothFree(left, right) => #merge,
                    #einstellung::FreezeCombination::OneFrozen(x) => x,
                    #einstellung::FreezeCombination::BothFrozen => return ::core::result::Result::Err(#einstellung::ConfigError::FreezeCollision(#einstellung::FieldPath::new(#complete_type_name, #ident_str))),
                }
            }
        }
    };

    quote_spanned!(ident.span() => #ident: #merged)
}

/// Generate the impl of `PartialConfig` for the associated partial struct
fn generate_partial_impl(model: &TransformedStruct) -> TokenStream {
    let TransformedStruct {
        partial_ident,
        complete_ident,
        einstellung,
        fields,
        ..
    } = model;

    let complete_type_name = complete_ident.to_string();

    let merge_fields = fields
        .iter()
        .map(|f| generate_merge_for_field(einstellung, &complete_type_name, f));

    let build_fields = fields
        .iter()
        .map(|f| generate_build_for_field(einstellung, &complete_type_name, f));

    quote_spanned! { partial_ident.span() =>
        #[automatically_derived]
        impl #einstellung::PartialConfig for #partial_ident {
            type Complete = #complete_ident;

            fn merge(self, next: Self) -> ::core::result::Result<Self, #einstellung::ConfigError> {
                ::core::result::Result::Ok(Self { #(#merge_fields,)* })
            }
            fn build(self) -> ::core::result::Result<Self::Complete, #einstellung::ConfigError> {
                ::core::result::Result::Ok(#complete_ident { #(#build_fields),* })
            }
        }
    }
}

/// Generate the impl of `Freezable` for the associated partial struct if required
fn generate_freezable_impl(model: &TransformedStruct) -> TokenStream {
    let TransformedStruct {
        partial_ident,
        einstellung,
        any_freezable,
        fields,
        ..
    } = model;

    // don't impl `Freezable` for types without any freezable fields
    if !any_freezable {
        return quote! {};
    }

    let freeze_fields = fields.iter().map(|f| {
        let ident = &f.ident;
        let resolve = match f.freeze {
            FreezeStrategy::NotFreezable => quote!(self.#ident),
            _ => quote!(#einstellung::Freezable::freeze(self.#ident)),
        };
        quote_spanned!(ident.span() => #ident: #resolve)
    });

    let is_field_frozen = fields.iter().filter_map(|f| {
        let ident = &f.ident;
        (f.freeze != FreezeStrategy::NotFreezable).then(
            || quote_spanned!(ident.span() => #einstellung::Freezable::is_frozen(&self.#ident)),
        )
    });

    quote_spanned! {partial_ident.span() =>
        #[automatically_derived]
        impl #einstellung::Freezable for #partial_ident {
            fn freeze(self) -> Self { Self { #(#freeze_fields,)* } }
            fn is_frozen(&self) -> bool { #(#is_field_frozen)||* }
        }
    }
}

/// Generate the actual impl of `Config` for the input type
fn generate_config_impl(model: &TransformedStruct) -> TokenStream {
    let TransformedStruct {
        partial_ident,
        einstellung,
        complete_ident,
        ..
    } = model;

    quote_spanned! {partial_ident.span() =>
        #[automatically_derived]
        impl #einstellung::Config for #complete_ident {
            type Partial = #partial_ident;
        }
    }
}
