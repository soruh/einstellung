//! Emit partial declarations and trait implementations from the validated model.

use crate::derive_config::{parser::DefaultStrategy, transformer::UnwrapStrategy};

use super::transformer::{
    FreezeStrategy, MergeStrategy, PartialType, TransformedField, TransformedStruct,
};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote, quote_spanned};
use syn::{parse_quote_spanned, spanned::Spanned};

impl ToTokens for TransformedStruct {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        generate_partial_struct(self).to_tokens(tokens);
        generate_partial_impl(self).to_tokens(tokens);
        generate_freezable_impl(self).to_tokens(tokens);
        generate_config_impl(self).to_tokens(tokens);
    }
}

/// Convert a `syn::Path` to a literal for attributes such as `#[serde(crate = "...")]`.
fn path_to_litstr(path: &syn::Path) -> syn::LitStr {
    let mut s = String::new();
    let mut iter = path.segments.iter();

    if path.leading_colon.is_some() {
        s += "::";
    }

    if let Some(first) = iter.next() {
        s.push_str(&first.ident.to_string());
        for seg in iter {
            s.push_str("::");
            s.push_str(&seg.ident.to_string());
        }
    }

    syn::LitStr::new(&s, path.span())
}

/// Renders the Rust type for a partial field based on the `PartialType` metadata.
fn render_partial_value_type(pt: &PartialType, einstellung: &syn::Path) -> TokenStream {
    let core = &pt.core_type;

    let mut tokens = if pt.access_partial {
        quote!(<#core as #einstellung::Config>::Partial)
    } else {
        quote!(#core)
    };

    if pt.wrap_option {
        tokens = quote!(::core::option::Option<#tokens>);
    }

    tokens
}

/// Render the partial field type, including its freeze wrapper when required.
fn render_partial_type(pt: &PartialType, einstellung: &syn::Path) -> TokenStream {
    let tokens = render_partial_value_type(pt, einstellung);

    if pt.wrap_freeze {
        quote!(#einstellung::Freeze<#tokens>)
    } else {
        tokens
    }
}

/// Generate the associated Partial as described by the `TransformedStruct`.
fn generate_partial_struct(model: &TransformedStruct) -> TokenStream {
    let partial_ident = &model.partial_ident;
    let vis = &model.vis;
    let einstellung = &model.einstellung;
    let attrs = &model.attrs;
    let partial_doc = format!(
        "Partial configuration for the `{}` type.",
        model.complete_ident
    );
    let partial_doc = (!attrs.iter().any(|attr| attr.path().is_ident("doc")))
        .then(|| quote!(#[doc = #partial_doc]));
    let deny_unknown_fields = model
        .deny_unknown_fields
        .then(|| quote!(#[serde(deny_unknown_fields)]));

    let fields = model.fields.iter().map(|f| {
        let ident = &f.ident;
        let f_attrs = &f.attrs;
        let f_vis = &f.vis;
        let ty = render_partial_type(&f.partial_type, einstellung);
        let field_doc = format!("Partial value for the `{ident}` configuration field.");
        let field_doc = (!f_attrs.iter().any(|attr| attr.path().is_ident("doc")))
            .then(|| quote!(#[doc = #field_doc]));
        let deserialize_flattened = f.flattened_subconfig.then(|| {
            let path: syn::Path = syn::parse_quote!(#einstellung::deserialize_flattened);
            let path = path_to_litstr(&path);
            quote!(#[serde(deserialize_with = #path)])
        });

        quote! {
            #field_doc
            #(#f_attrs)*
            #deserialize_flattened
            #f_vis #ident: #ty
        }
    });

    let serde_path: syn::Path = parse_quote_spanned!(einstellung.span() => #einstellung::serde);
    let serde_lit = path_to_litstr(&serde_path);

    quote! {
        #partial_doc
        #[derive(::core::default::Default, #einstellung::serde::Deserialize)]
        #(#attrs)*
        #[serde(crate = #serde_lit)]
        #deny_unknown_fields
        #vis struct #partial_ident {
            #(#fields,)*
        }
    }
}

/// Generate the code to merge the field `left` with the field `right`.
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
            let ident_str = &f.logical_name;
            let partial_type = render_partial_value_type(&f.partial_type, einstellung);

            quote_spanned!(func_path.span() => {
                let _: #einstellung::MergeFunction<#partial_type, _> = #func_path;
                #func_path(#left, #right).map_err(|reason| #einstellung::ConfigError::CustomMerge {
                    field: #einstellung::FieldPath::new(#complete_str, #ident_str),
                    reason: #einstellung::into_box_error(reason),
                })?
            })
        }
        MergeStrategy::MergeSubconfig => {
            let ident_str = &f.logical_name;
            if f.flattened_subconfig {
                quote! {
                    match (#left, #right) {
                        (Some(a), Some(b)) => Some(#einstellung::PartialConfig::merge(a, b)?),
                        (a, b) => a.or(b)
                    }
                }
            } else {
                quote! {
                    match (#left, #right) {
                        (Some(a), Some(b)) => Some(#einstellung::merge_with_context(
                            a,
                            b,
                            #complete_str,
                            #ident_str,
                        )?),
                        (a, b) => a.or(b)
                    }
                }
            }
        }
    }
}

/// Generate the code to build a single field in a `PartialConfig::build` impl.
fn generate_build_for_field(
    einstellung: &syn::Path,
    complete_type_name: &str,
    f: &TransformedField,
) -> TokenStream {
    let ident = &f.ident;
    let ident_str = &f.logical_name;
    let complete_type = &f.complete_type;

    let unfreeze = if f.freeze == FreezeStrategy::Wrapped {
        quote! { #einstellung::Freeze::into_inner(self.#ident) }
    } else {
        quote! { self.#ident }
    };

    let build_input = if f.flattened_subconfig
        && matches!(f.build.unwrap, UnwrapStrategy::DontUnwrap)
    {
        quote! {
            #unfreeze.and_then(|value| {
                (!#einstellung::PartialConfig::provided_fields(&value).is_empty()).then_some(value)
            })
        }
    } else if f.flattened_subconfig {
        quote!(::core::option::Option::Some(#unfreeze.unwrap_or_default()))
    } else {
        unfreeze
    };

    let built = if f.build.build {
        if f.flattened_subconfig {
            quote! { #build_input.map(|x| #einstellung::PartialConfig::build(x)).transpose()? }
        } else {
            quote! { #build_input.map(|x| #einstellung::build_with_context(x, #complete_type_name, #ident_str)).transpose()? }
        }
    } else {
        quote! { #build_input }
    };

    let resolve = match &f.build.unwrap {
        UnwrapStrategy::DontUnwrap => quote! { #built },
        UnwrapStrategy::Unwrap => {
            quote! { #built.ok_or(#einstellung::ConfigError::MissingField(#einstellung::FieldPath::new(#complete_type_name, #ident_str)))? }
        }
        UnwrapStrategy::UnwrapWithDefault(default) => match default {
            DefaultStrategy::Value(val) => quote! { #built.unwrap_or_else(|| #val) },
            DefaultStrategy::Call(func) => quote! { #built.unwrap_or_else(#func) },
            DefaultStrategy::DefaultTrait => {
                quote! { #built.unwrap_or_else(::core::default::Default::default) }
            }
        },
    };

    let validated = if let Some(validate_func) = &f.validate_func {
        quote_spanned!(validate_func.span() => {
            let #ident: #complete_type = #resolve;
            let validator = #validate_func;
            if let Err(e) = validator(&#ident) {
                return Err(#einstellung::ConfigError::Validation {
                    field: #einstellung::FieldPath::new(#complete_type_name, #ident_str),
                    reason: #einstellung::into_box_error(e),
                });
            }
            #ident
        })
    } else {
        resolve
    };

    quote_spanned!(ident.span() => #ident: #validated)
}

/// Generate the code to merge a single field in a `PartialConfig::merge` impl.
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
            let ident_str = &f.logical_name;
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
            let ident_str = &f.logical_name;
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

/// Borrow the optional field value through any freeze wrapper.
fn partial_option_ref(f: &TransformedField, einstellung: &syn::Path) -> TokenStream {
    let ident = &f.ident;
    if f.freeze == FreezeStrategy::Wrapped {
        quote! {
            match &self.#ident {
                #einstellung::Freeze::Free(value) | #einstellung::Freeze::Frozen(value) => value
            }
        }
    } else {
        quote! { &self.#ident }
    }
}

/// Generate provenance paths for values explicitly supplied by this partial.
fn generate_provided_field(f: &TransformedField, einstellung: &syn::Path) -> TokenStream {
    let ident_str = &f.logical_name;
    let field = partial_option_ref(f, einstellung);

    if f.build.build {
        if f.flattened_subconfig {
            quote! {
                if let ::core::option::Option::Some(value) = (#field).as_ref() {
                    fields.extend(#einstellung::PartialConfig::provided_fields(value));
                }
            }
        } else {
            quote! {
                if let ::core::option::Option::Some(value) = (#field).as_ref() {
                    fields.push(::std::string::String::from(#ident_str));
                    for nested in #einstellung::PartialConfig::provided_fields(value) {
                        fields.push(::std::format!("{}.{}", #ident_str, nested));
                    }
                }
            }
        }
    } else {
        quote! {
            if (#field).is_some() {
                fields.push(::std::string::String::from(#ident_str));
            }
        }
    }
}

/// Generate paths for defaults that final construction will actually apply.
fn generate_defaulted_field(f: &TransformedField, einstellung: &syn::Path) -> TokenStream {
    let ident_str = &f.logical_name;
    let field = partial_option_ref(f, einstellung);
    let has_default = matches!(f.build.unwrap, UnwrapStrategy::UnwrapWithDefault(_));

    if f.build.build {
        if f.flattened_subconfig {
            let optional = matches!(f.build.unwrap, UnwrapStrategy::DontUnwrap);
            if optional {
                return quote! {
                    if let ::core::option::Option::Some(value) = (#field).as_ref() {
                        if !#einstellung::PartialConfig::provided_fields(value).is_empty() {
                            fields.extend(#einstellung::PartialConfig::defaulted_fields(value));
                        }
                    }
                };
            }
            let core = &f.partial_type.core_type;
            quote! {
                if let ::core::option::Option::Some(value) = (#field).as_ref() {
                    fields.extend(#einstellung::PartialConfig::defaulted_fields(value));
                } else {
                    let value = <<#core as #einstellung::Config>::Partial as ::core::default::Default>::default();
                    fields.extend(#einstellung::PartialConfig::defaulted_fields(&value));
                }
            }
        } else {
            let missing = has_default.then(|| {
                quote! {
                    else {
                        fields.push(::std::string::String::from(#ident_str));
                    }
                }
            });
            quote! {
                if let ::core::option::Option::Some(value) = (#field).as_ref() {
                    for nested in #einstellung::PartialConfig::defaulted_fields(value) {
                        fields.push(::std::format!("{}.{}", #ident_str, nested));
                    }
                } #missing
            }
        }
    } else if has_default {
        quote! {
            if (#field).is_none() {
                fields.push(::std::string::String::from(#ident_str));
            }
        }
    } else {
        quote! {}
    }
}

/// Generate the impl of `PartialConfig` for the associated partial struct.
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

    let provided_fields = fields
        .iter()
        .map(|f| generate_provided_field(f, einstellung))
        .collect::<Vec<_>>();
    let defaulted_fields = fields
        .iter()
        .map(|f| generate_defaulted_field(f, einstellung))
        .collect::<Vec<_>>();
    let provided_fields_decl = if provided_fields.iter().any(|tokens| !tokens.is_empty()) {
        quote!(let mut fields = ::std::vec::Vec::new();)
    } else {
        quote!(let fields = ::std::vec::Vec::new();)
    };
    let defaulted_fields_decl = if defaulted_fields.iter().any(|tokens| !tokens.is_empty()) {
        quote!(let mut fields = ::std::vec::Vec::new();)
    } else {
        quote!(let fields = ::std::vec::Vec::new();)
    };

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
            fn provided_fields(&self) -> ::std::vec::Vec<::std::string::String> {
                #provided_fields_decl
                #(#provided_fields)*
                fields
            }
            fn defaulted_fields(&self) -> ::std::vec::Vec<::std::string::String> {
                #defaulted_fields_decl
                #(#defaulted_fields)*
                fields
            }
        }
    }
}

/// Generate the impl of `Freezable` for the associated partial struct if required.
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

    let is_field_frozen = fields
        .iter()
        .filter_map(|f| {
            let ident = &f.ident;
            (f.freeze != FreezeStrategy::NotFreezable).then(
                || quote_spanned!(ident.span() => #einstellung::Freezable::is_frozen(&self.#ident)),
            )
        })
        .collect::<Vec<_>>();
    let is_frozen = if is_field_frozen.is_empty() {
        quote!(false)
    } else {
        quote!(#(#is_field_frozen)||*)
    };

    quote_spanned! {partial_ident.span() =>
        #[automatically_derived]
        impl #einstellung::Freezable for #partial_ident {
            fn freeze(self) -> Self { Self { #(#freeze_fields,)* } }
            fn is_frozen(&self) -> bool { #is_frozen }
        }
    }
}

/// Generate the actual impl of `Config` for the input type.
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
