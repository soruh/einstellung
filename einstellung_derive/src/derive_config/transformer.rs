use super::parser::{ConfigFieldReceiver, ConfigStructReceiver};
use crate::derive_config::parser::{DefaultStrategy, MergeStrategyReceiver};
use syn::{
    GenericArgument, Meta, PathArguments, Token, Type, parse::Parser, punctuated::Punctuated,
    spanned::Spanned,
};

#[derive(Debug)]
pub struct TransformedStruct {
    pub complete_ident: syn::Ident,
    pub partial_ident: syn::Ident,
    pub any_freezable: bool,
    pub deny_unknown_fields: bool,
    pub vis: syn::Visibility,
    pub fields: Vec<TransformedField>,
    pub attrs: Vec<syn::Attribute>,
    pub einstellung: syn::Path,
}

#[derive(Debug)]
pub struct TransformedField {
    pub ident: syn::Ident,
    pub logical_name: String,
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
pub enum UnwrapStrategy {
    DontUnwrap,
    Unwrap,
    UnwrapWithDefault(DefaultStrategy),
}

#[derive(Debug)]
pub struct BuildStategy {
    pub build: bool,
    pub unwrap: UnwrapStrategy,
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

#[derive(Clone, Copy, Debug, Default)]
enum SerdeRenameRule {
    #[default]
    None,
    LowerCase,
    UpperCase,
    PascalCase,
    CamelCase,
    SnakeCase,
    ScreamingSnakeCase,
    KebabCase,
    ScreamingKebabCase,
}

impl SerdeRenameRule {
    fn parse(value: &str, span: proc_macro2::Span) -> syn::Result<Self> {
        let rule = match value {
            "lowercase" => Self::LowerCase,
            "UPPERCASE" => Self::UpperCase,
            "PascalCase" => Self::PascalCase,
            "camelCase" => Self::CamelCase,
            "snake_case" => Self::SnakeCase,
            "SCREAMING_SNAKE_CASE" => Self::ScreamingSnakeCase,
            "kebab-case" => Self::KebabCase,
            "SCREAMING-KEBAB-CASE" => Self::ScreamingKebabCase,
            _ => {
                return Err(syn::Error::new(
                    span,
                    format!("unknown serde rename rule {value:?}"),
                ));
            }
        };
        Ok(rule)
    }

    fn apply_to_field(self, field: &str) -> String {
        match self {
            Self::None | Self::LowerCase | Self::SnakeCase => field.to_owned(),
            Self::UpperCase => field.to_ascii_uppercase(),
            Self::PascalCase => {
                let mut pascal = String::new();
                let mut capitalize = true;
                for ch in field.chars() {
                    if ch == '_' {
                        capitalize = true;
                    } else if capitalize {
                        pascal.push(ch.to_ascii_uppercase());
                        capitalize = false;
                    } else {
                        pascal.push(ch);
                    }
                }
                pascal
            }
            Self::CamelCase => {
                let pascal = Self::PascalCase.apply_to_field(field);
                let mut chars = pascal.chars();
                match chars.next() {
                    Some(first) => first.to_ascii_lowercase().to_string() + chars.as_str(),
                    None => pascal,
                }
            }
            Self::ScreamingSnakeCase => field.to_ascii_uppercase(),
            Self::KebabCase => field.replace('_', "-"),
            Self::ScreamingKebabCase => field.to_ascii_uppercase().replace('_', "-"),
        }
    }
}

fn forwarded_serde_metas(
    direct: &[Meta],
    partial: &[super::parser::PartialReceiver],
) -> syn::Result<Vec<Meta>> {
    let mut metas = Vec::new();
    let parser = Punctuated::<Meta, Token![,]>::parse_terminated;
    for meta in direct {
        if let Meta::List(list) = meta
            && list.path.is_ident("serde")
        {
            metas.extend(parser.parse2(list.tokens.clone())?);
        } else {
            metas.push(meta.clone());
        }
    }
    for receiver in partial {
        for nested in &receiver.0 {
            let darling::ast::NestedMeta::Meta(Meta::List(list)) = nested else {
                continue;
            };
            if !list.path.is_ident("serde") {
                continue;
            }
            metas.extend(parser.parse2(list.tokens.clone())?);
        }
    }
    Ok(metas)
}

fn serde_deserialize_setting(metas: &[Meta], name: &str) -> syn::Result<Option<syn::LitStr>> {
    for meta in metas {
        match meta {
            Meta::NameValue(value) if value.path.is_ident(name) => {
                let syn::Expr::Lit(expr) = &value.value else {
                    continue;
                };
                let syn::Lit::Str(value) = &expr.lit else {
                    continue;
                };
                return Ok(Some(value.clone()));
            }
            Meta::List(list) if list.path.is_ident(name) => {
                let parser = Punctuated::<Meta, Token![,]>::parse_terminated;
                let nested = parser.parse2(list.tokens.clone())?;
                for nested in nested {
                    let Meta::NameValue(value) = nested else {
                        continue;
                    };
                    if !value.path.is_ident("deserialize") {
                        continue;
                    }
                    let syn::Expr::Lit(expr) = value.value else {
                        continue;
                    };
                    let syn::Lit::Str(value) = expr.lit else {
                        continue;
                    };
                    return Ok(Some(value));
                }
            }
            _ => {}
        }
    }
    Ok(None)
}

fn serde_rename_rule(
    direct: &[Meta],
    partial: &[super::parser::PartialReceiver],
) -> syn::Result<SerdeRenameRule> {
    let metas = forwarded_serde_metas(direct, partial)?;
    let Some(value) = serde_deserialize_setting(&metas, "rename_all")? else {
        return Ok(SerdeRenameRule::None);
    };
    SerdeRenameRule::parse(&value.value(), value.span())
}

fn serde_field_name(
    field: &ConfigFieldReceiver,
    ident: &syn::Ident,
    rename_rule: SerdeRenameRule,
) -> syn::Result<String> {
    let metas = forwarded_serde_metas(&field.serde, &field.partial)?;
    if let Some(rename) = serde_deserialize_setting(&metas, "rename")? {
        Ok(rename.value())
    } else {
        let rust_name = ident.to_string();
        let rust_name = rust_name.strip_prefix("r#").unwrap_or(&rust_name);
        Ok(rename_rule.apply_to_field(rust_name))
    }
}

/// Helper to extract inner type of an `Option`.
/// For `Option<T>` return `Some(T)`
/// For anything else return `None`
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

/// Transform the parsed struct into a type describing the output types and impls
pub fn transform_struct(mut receiver: ConfigStructReceiver) -> syn::Result<TransformedStruct> {
    let rename_rule = serde_rename_rule(&receiver.serde, &receiver.partial)?;
    let attrs = receiver.take_partial_attrs();

    let complete_ident = receiver.ident.clone();
    let partial_ident = syn::Ident::new(&format!("{complete_ident}Partial"), complete_ident.span());
    let vis = receiver.vis;
    let einstellung = receiver.einstellung;

    let struct_data = receiver.data.take_struct().ok_or_else(|| {
        syn::Error::new(
            complete_ident.span(),
            "Config can only be derived for structs with named fields",
        )
    })?;

    let any_freezable = receiver.freezable || struct_data.iter().any(|field| field.freezable);
    let deny_unknown_fields = receiver.deny_unknown_fields;

    let mut fields = Vec::with_capacity(struct_data.len());
    let mut errors: Option<syn::Error> = None;

    for field in struct_data {
        match transform_field(field, receiver.freezable, rename_rule) {
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
            deny_unknown_fields,
            attrs,
            vis,
            fields,
            einstellung,
        })
    }
}

/// Transform a parsed field into its partial form
fn transform_field(
    mut field: ConfigFieldReceiver,
    all_freezeable: bool,
    rename_rule: SerdeRenameRule,
) -> syn::Result<TransformedField> {
    let ident = field
        .ident
        .clone()
        .ok_or_else(|| syn::Error::new(field.ty.span(), "Config fields must be named"))?;
    let logical_name = serde_field_name(&field, &ident, rename_rule)?;
    let attrs = field.take_partial_attrs();
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

    let merge = if field.subconfig {
        MergeStrategy::MergeSubconfig
    } else {
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

    let unwrap = if complete_is_optional {
        if let Some(default) = field.default {
            return Err(syn::Error::new(
                default.span(),
                "#[config(default = ...)] is meaningless on an `Option` type",
            ));
        }

        UnwrapStrategy::DontUnwrap
    } else if let Some(default) = field.default {
        UnwrapStrategy::UnwrapWithDefault(default.into_inner())
    } else {
        UnwrapStrategy::Unwrap
    };

    let build = BuildStategy {
        build: field.subconfig,
        unwrap,
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
        logical_name,
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
