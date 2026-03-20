use super::parser::{ConfigFieldReceiver, ConfigStructReceiver};
use crate::derive_config::parser::{DefaultStrategyReceiver, MergeStrategyReceiver};
use darling::util::SpannedValue;
use syn::{GenericArgument, PathArguments, Type, parse_quote};

#[derive(Debug)]
pub struct TransformedStruct {
    pub complete_ident: syn::Ident,
    pub partial_ident: syn::Ident,
    pub any_freezable: bool,
    pub vis: syn::Visibility,
    pub fields: Vec<TransformedField>,
    pub attrs: Vec<syn::Attribute>,
    pub einstellung: syn::Path,
}

#[derive(Debug)]
pub struct TransformedField {
    pub ident: syn::Ident,
    pub vis: syn::Visibility,
    pub complete_type: syn::Type,
    pub partial_type: PartialType,
    pub build: BuildStategy,
    pub merge: MergeStrategy,
    pub freeze: FreezeStrategy,
    pub validate_func: Option<syn::Expr>,
    pub attrs: Vec<syn::Attribute>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum FreezeStrategy {
    NotFreezable,
    Wrapped,
    IntrinsicallyFreezable,
}

#[derive(Debug)]
pub enum DefaultInitializer {
    Value(syn::Expr),
    Call(syn::Expr),
    DefaultTrait,
}

#[derive(Debug)]
pub enum BuildStategy {
    SubconfigRequired,
    SubconfigOptional,
    Required,
    Optional,
    Default(DefaultInitializer),
}

#[derive(Debug)]
pub enum MergeStrategy {
    MergeSubconfig,
    Replace,
    Extend,
    Custom(SpannedValue<String>),
}

#[derive(Debug)]
pub struct PartialType {
    pub core_type: syn::Type,
    pub access_partial: bool,
    pub wrap_option: bool,
    pub wrap_freeze: bool,
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

pub fn transform(mut receiver: ConfigStructReceiver) -> syn::Result<TransformedStruct> {
    let attrs = receiver.take_partial_attrs();

    let complete_ident = receiver.ident.clone();
    let partial_ident = syn::Ident::new(&format!("{complete_ident}Partial"), complete_ident.span());
    let vis = receiver.vis;
    let einstellung = receiver.einstellung;

    let struct_data = receiver
        .data
        .take_struct()
        .expect("Only named structs supported");

    let any_freezable = receiver.freezable || struct_data.iter().any(|field| field.freezable);

    let mut fields = Vec::with_capacity(struct_data.len());
    let mut errors: Option<syn::Error> = None;

    for field in struct_data {
        match transform_field(field, &einstellung, receiver.freezable) {
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
            any_freezable,
            attrs,
            vis,
            fields,
            einstellung,
        })
    }
}

fn transform_field(
    mut field: ConfigFieldReceiver,
    einstellung: &syn::Path,
    all_freezeable: bool,
) -> syn::Result<TransformedField> {
    let attrs = field.take_partial_attrs();

    let ident = field.ident.unwrap();
    let complete_type = field.ty;

    let inner_type_if_optional = extract_type_from_option(&complete_type);
    let complete_is_optional = inner_type_if_optional.is_some();
    let core_type = inner_type_if_optional
        .cloned()
        .unwrap_or_else(|| complete_type.clone());

    if field.subconfig {
        if let Some(strategy) = field.merge {
            return Err(syn::Error::new(
                strategy.span(),
                "Merge strategy is invalid on a subconfig",
            ));
        }
    }

    let mut partial_type = if field.subconfig {
        PartialType {
            core_type,
            access_partial: true,
            wrap_option: true,
            wrap_freeze: false,
        }
    } else {
        PartialType {
            core_type,
            access_partial: false,
            wrap_option: true,
            wrap_freeze: false,
        }
    };

    // {
    //     let span = field
    //         .merge
    //         .as_ref()
    //         .map(|s| s.span())
    //         .unwrap_or_else(|| ident.span());
    //     let merge_strategy = match field.merge {
    //         Some(m) => m.into_inner(),
    //         None => MergeStrategy::Replace,
    //     };

    //     let partial_type = syn::parse_quote!(Option<#core_type>);

    //     let kind = match merge_strategy {
    //         MergeStrategyReceiver::Extend => FieldKind::Extend {
    //             fallback: determine_fallback(&field.default, complete_is_optional),
    //         },
    //         MergeStrategyReceiver::Replace => FieldKind::Replace {
    //             fallback: determine_fallback(&field.default, complete_is_optional),
    //         },
    //         MergeStrategyReceiver::Function(s) => {
    //             let func_path = syn::parse_str::<syn::Path>(&s).map_err(|_| {
    //                 syn::Error::new(span, format!("Invalid function path: '{}'", &*s))
    //             })?;

    //             let func_path = SpannedValue::new(func_path, s.span());

    //             FieldKind::CustomMerge {
    //                 func_path,
    //                 fallback: determine_fallback(&field.default, complete_is_optional),
    //             }
    //         }
    //     };

    //     (partial_type, kind)
    // }

    let build = todo!();
    let merge = todo!();

    let freezable = field.freezable || all_freezeable;
    let freeze = if !freezable {
        FreezeStrategy::NotFreezable
    } else if field.subconfig {
        FreezeStrategy::IntrinsicallyFreezable
    } else {
        FreezeStrategy::Wrapped
    };

    if freeze == FreezeStrategy::Wrapped {
        partial_type.wrap_freeze = true;
    }

    Ok(TransformedField {
        ident,
        vis: field.vis,
        freeze,
        partial_type,
        complete_type,
        validate_func: field.validate,
        attrs,
        build,
        merge,
    })
}

fn determine_fallback(default: &DefaultStrategyReceiver, is_optional: bool) -> FallbackStrategy {
    match default {
        DefaultStrategyReceiver::Required => {
            if is_optional {
                FallbackStrategy::Keep
            } else {
                FallbackStrategy::Require
            }
        }
        DefaultStrategyReceiver::Standard => {
            if is_optional {
                FallbackStrategy::Keep
            } else {
                FallbackStrategy::Standard
            }
        }
        DefaultStrategyReceiver::Value(e) => FallbackStrategy::Value(e.clone()),
        DefaultStrategyReceiver::Call(e) => FallbackStrategy::Call(e.clone()),
    }
}
