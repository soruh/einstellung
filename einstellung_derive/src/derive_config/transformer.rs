use super::parser::{ConfigFieldReceiver, ConfigStructReceiver};
use crate::derive_config::parser::{DefaultStrategyReceiver, MergeStrategyReceiver};
use syn::{GenericArgument, PathArguments, Type};

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
    Custom(syn::Path),
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
        match transform_field(field, receiver.freezable) {
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

    if field.subconfig
        && let Some(strategy) = field.merge
    {
        return Err(syn::Error::new(
            strategy.span(),
            "Merge strategy is invalid on a subconfig",
        ));
    }

    // Determine the merge strategy
    let merge = if field.subconfig {
        MergeStrategy::MergeSubconfig
    } else {
        // Extract the inner strategy from SpannedValue, defaulting to Replace
        let merge_strategy = match field.merge {
            Some(m) => m.into_inner(),
            None => MergeStrategyReceiver::Replace,
        };

        match merge_strategy {
            MergeStrategyReceiver::Extend => MergeStrategy::Extend,
            MergeStrategyReceiver::Replace => MergeStrategy::Replace,
            MergeStrategyReceiver::Function(s) => match syn::parse_str(&s) {
                Ok(path) => MergeStrategy::Custom(path),
                Err(err) => {
                    return Err(syn::Error::new(
                        s.span(),
                        format!("Invalid merge function path: {err}"),
                    ));
                }
            },
        }
    };

    // Determine the build strategy
    #[rustfmt::skip]
    let build = if field.subconfig {
        if complete_is_optional {
            BuildStategy::SubconfigOptional
        } else {
            BuildStategy::SubconfigRequired
        }
    } else {
        match (complete_is_optional, field.default) {
            (_, DefaultStrategyReceiver::Value(e)) => BuildStategy::Default(DefaultInitializer::Value(e)),
            (_, DefaultStrategyReceiver::Call(e)) => BuildStategy::Default(DefaultInitializer::Call(e)),

            (true, DefaultStrategyReceiver::DefaultTrait) => BuildStategy::Optional,
            (true, DefaultStrategyReceiver::None) => BuildStategy::Optional,
            
            (false, DefaultStrategyReceiver::DefaultTrait) => BuildStategy::Default(DefaultInitializer::DefaultTrait),
            (false, DefaultStrategyReceiver::None) => BuildStategy::Required,
        }
    };

    let freezable = field.freezable || all_freezeable;
    let freeze = if !freezable {
        FreezeStrategy::NotFreezable
    } else if field.subconfig {
        FreezeStrategy::IntrinsicallyFreezable
    } else {
        FreezeStrategy::Wrapped
    };

    let partial_type = PartialType {
        core_type,
        access_partial: field.subconfig,
        wrap_option: true,
        wrap_freeze: freeze == FreezeStrategy::Wrapped,
    };

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
