use super::parser::{ConfigFieldReceiver, ConfigStructReceiver};
use crate::derive_config::parser::{DefaultStrategy, MergeStrategy};
use darling::util::SpannedValue;
use syn::{GenericArgument, PathArguments, Type};

#[derive(Debug)]
pub struct TransformedStruct {
    pub complete_ident: syn::Ident,
    pub partial_ident: syn::Ident,
    pub vis: syn::Visibility,
    pub fields: Vec<TransformedField>,
    pub einstellung: syn::Path,
}

#[derive(Debug)]
pub enum FallbackStrategy {
    Require,
    Keep, // Used when the complete type is an Option
    Value(syn::Expr),
    Call(syn::Expr),
    Standard, // Default::default()
}

#[derive(Debug)]
pub enum FieldKind {
    Subconfig {
        complete_is_optional: bool,
    },
    Extend {
        partial_is_optional: bool,
        complete_is_optional: bool,
    },
    Replace {
        fallback: FallbackStrategy,
    },
    CustomMerge {
        func_path: SpannedValue<syn::Path>,
        fallback: FallbackStrategy,
    },
}

#[derive(Debug)]
pub struct TransformedField {
    pub ident: syn::Ident,
    pub vis: syn::Visibility,
    pub complete_type: syn::Type,
    pub partial_type: syn::Type,
    pub kind: FieldKind,
    pub validate_func: Option<syn::Expr>,
    pub serde_attrs: Vec<syn::Attribute>,
}

pub fn transform(receiver: ConfigStructReceiver) -> syn::Result<TransformedStruct> {
    let complete_ident = receiver.ident.clone();
    let partial_ident = syn::Ident::new(&format!("{complete_ident}Partial"), complete_ident.span());
    let vis = receiver.vis;
    let einstellung = receiver.einstellung;

    let struct_data = receiver
        .data
        .take_struct()
        .expect("Only named structs supported");

    let mut fields = Vec::new();
    let mut errors: Option<syn::Error> = None;

    for field in struct_data {
        match transform_field(field, &einstellung) {
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
        Err(err)
    } else {
        Ok(TransformedStruct {
            complete_ident,
            partial_ident,
            vis,
            fields,
            einstellung,
        })
    }
}

fn transform_field(
    field: ConfigFieldReceiver,
    einstellung: &syn::Path,
) -> syn::Result<TransformedField> {
    let ident = field.ident.clone().ok_or_else(|| {
        syn::Error::new(proc_macro2::Span::call_site(), "Named fields are required")
    })?;

    let serde_attrs = field
        .serde
        .into_iter()
        .map(|meta| syn::parse_quote! { #[#meta] })
        .collect();

    let complete_type = field.ty;
    let inner_type_if_optional = extract_type_from_option(&complete_type);
    let complete_is_optional = inner_type_if_optional.is_some();
    let core_type = inner_type_if_optional
        .cloned()
        .unwrap_or_else(|| complete_type.clone());

    let (partial_type, kind) = if field.subconfig {
        if let Some(strategy) = field.merge {
            return Err(syn::Error::new(
                strategy.span(),
                "Merge strategy is invalid on a subconfig",
            ));
        }

        let partial_type = syn::parse_quote!(Option<<#core_type as #einstellung::Config>::Partial>);

        (
            partial_type,
            FieldKind::Subconfig {
                complete_is_optional,
            },
        )
    } else {
        let span = field
            .merge
            .as_ref()
            .map(|s| s.span())
            .unwrap_or_else(|| ident.span());
        let merge_strategy = match field.merge {
            Some(m) => m.into_inner(),
            None => MergeStrategy::Replace,
        };

        match merge_strategy {
            MergeStrategy::Extend => {
                let partial_is_optional =
                    complete_is_optional || field.default == DefaultStrategy::Required;

                let partial_type = if partial_is_optional {
                    syn::parse_quote!(Option<#core_type>)
                } else {
                    syn::parse_quote!(#core_type)
                };

                (
                    partial_type,
                    FieldKind::Extend {
                        partial_is_optional,
                        complete_is_optional,
                    },
                )
            }
            MergeStrategy::Replace => (
                syn::parse_quote!(Option<#core_type>),
                FieldKind::Replace {
                    fallback: determine_fallback(&field.default, complete_is_optional),
                },
            ),
            MergeStrategy::Function(s) => {
                let func_path = syn::parse_str::<syn::Path>(&s).map_err(|_| {
                    syn::Error::new(span, format!("Invalid function path: '{}'", &*s))
                })?;

                let func_path = SpannedValue::new(func_path, s.span());

                let partial_type = syn::parse_quote!(Option<#core_type>);

                (
                    partial_type,
                    FieldKind::CustomMerge {
                        func_path,
                        fallback: determine_fallback(&field.default, complete_is_optional),
                    },
                )
            }
        }
    };

    Ok(TransformedField {
        ident,
        vis: field.vis,
        kind,
        partial_type,
        complete_type,
        validate_func: field.validate,
        serde_attrs,
    })
}

fn determine_fallback(default: &DefaultStrategy, is_optional: bool) -> FallbackStrategy {
    match default {
        DefaultStrategy::Required => {
            if is_optional {
                FallbackStrategy::Keep
            } else {
                FallbackStrategy::Require
            }
        }
        DefaultStrategy::Standard => FallbackStrategy::Standard,
        DefaultStrategy::Value(e) => FallbackStrategy::Value(e.clone()),
        DefaultStrategy::Call(e) => FallbackStrategy::Call(e.clone()),
    }
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
