//! Validate parsed attributes and resolve field types, names, and merge policies.

use super::parser::{ConfigFieldReceiver, ConfigStructReceiver};
use crate::derive_config::parser::{DefaultStrategy, MergeStrategyReceiver};
use syn::{
    GenericArgument, Meta, PathArguments, Token, Type, parse::Parser, punctuated::Punctuated,
    spanned::Spanned,
};

#[derive(Debug)]
/// Validated model used to generate a partial type and its trait implementations.
pub struct TransformedStruct {
    /// Identifier of the user’s complete configuration type.
    pub complete_ident: syn::Ident,
    /// Identifier of the generated companion partial type.
    pub partial_ident: syn::Ident,
    /// Whether the partial needs a `Freezable` implementation.
    pub any_freezable: bool,
    /// Whether the generated partial rejects unknown input keys.
    pub deny_unknown_fields: bool,
    /// Visibility preserved on the generated declaration.
    pub vis: syn::Visibility,
    /// Validated fields in declaration order.
    pub fields: Vec<TransformedField>,
    /// Attributes forwarded to the generated partial declaration.
    pub attrs: Vec<syn::Attribute>,
    /// Path to the runtime crate, including an explicit crate-path override.
    pub einstellung: syn::Path,
}

#[derive(Debug)]
/// Validated type, merge, build, and provenance policy for one field.
pub struct TransformedField {
    /// Original Rust identifier used when emitting the corresponding declaration.
    pub ident: syn::Ident,
    /// Canonical input field name after Serde deserialization renaming.
    pub logical_name: String,
    /// Visibility preserved on the generated declaration.
    pub vis: syn::Visibility,
    /// Rust type used by the complete configuration field.
    pub complete_type: syn::Type,
    /// Instructions for constructing the partial field’s Rust type.
    pub partial_type: PartialType,
    /// Whether nested keys share the containing object’s input namespace.
    pub flattened_subconfig: bool,
    /// Policy for resolving the partial field into its complete value.
    pub build: BuildStategy,
    /// Selected merge strategy for combining existing and incoming values.
    pub merge: MergeStrategy,
    /// How the partial field participates in freeze-aware merging.
    pub freeze: FreezeStrategy,
    /// Optional validator expression called after resolving the complete value.
    pub validate_func: Option<syn::Expr>,
    /// Attributes forwarded to the generated partial declaration.
    pub attrs: Vec<syn::Attribute>,
}

#[derive(Debug, PartialEq, Eq)]
/// Representation used to enforce a field’s freeze policy.
pub enum FreezeStrategy {
    /// Merge this field without freeze state.
    NotFreezable,
    /// Store the partial value in the runtime `Freeze` wrapper.
    Wrapped,
    /// Delegate freeze state to a nested partial configuration.
    IntrinsicallyFreezable,
}

#[derive(Debug)]
/// How an optional partial value becomes a complete field.
pub enum UnwrapStrategy {
    /// Preserve optionality in the complete configuration.
    DontUnwrap,
    /// Require a supplied value and report a missing-field error otherwise.
    Unwrap,
    /// Use the configured fallback if the partial contains no value.
    UnwrapWithDefault(DefaultStrategy),
}

#[derive(Debug)]
/// Resolution and optionality policy for a completed field.
pub struct BuildStategy {
    /// Whether construction recursively builds a nested configuration.
    pub build: bool,
    /// Policy for absent values after nested configuration construction.
    pub unwrap: UnwrapStrategy,
}

#[derive(Debug)]
/// Validated operation used to combine two partial field values.
pub enum MergeStrategy {
    /// Recursively merge nested partial configurations.
    MergeSubconfig,
    /// Keep the incoming value when present, otherwise retain the existing value.
    Replace,
    /// Append incoming collection contents using `Extend`.
    Extend,
    /// Call the user-supplied merge function at this path.
    Custom(syn::Path),
}

#[derive(Debug)]
/// Describes how a complete field type is represented in a partial configuration.
pub struct PartialType {
    /// Field type after removing an outer `Option`, when present.
    pub core_type: syn::Type,
    /// Use the core type’s associated partial type for a nested configuration.
    pub access_partial: bool,
    /// Wrap the represented value in `Option` to record absence.
    pub wrap_option: bool,
    /// Wrap the optional value in `Freeze` to retain merge protection.
    pub wrap_freeze: bool,
}

#[derive(Clone, Copy, Debug, Default)]
/// Supported Serde field-renaming rules for canonical logical paths.
enum SerdeRenameRule {
    #[default]
    /// Preserve the original field name.
    None,
    /// Apply Serde’s lowercase field-name rule.
    LowerCase,
    /// Convert ASCII letters to uppercase.
    UpperCase,
    /// Capitalize underscore-separated words and remove separators.
    PascalCase,
    /// Apply Pascal casing with a lowercase initial character.
    CamelCase,
    /// Preserve the snake-case Rust field name.
    SnakeCase,
    /// Uppercase the snake-case field name.
    ScreamingSnakeCase,
    /// Replace underscores with hyphens.
    KebabCase,
    /// Uppercase the field name and replace underscores with hyphens.
    ScreamingKebabCase,
}

impl SerdeRenameRule {
    /// Parse a Serde rename rule, rejecting unsupported spellings at the supplied span.
    ///
    /// # Errors
    ///
    /// Returns a diagnostic if the rename rule is not one of Serde’s supported spellings.
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

    /// Apply the selected rule to a Rust field name with its raw prefix removed.
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

/// Collect Serde settings from both shorthand and explicit partial attributes.
///
/// # Errors
///
/// Returns a syntax error when a forwarded Serde attribute cannot be parsed.
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

/// Read a string setting, honoring a deserialize-specific nested override.
///
/// # Errors
///
/// Returns a syntax error when a nested deserialize-specific setting is malformed.
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

/// Resolve the container’s Serde deserialization rename rule.
///
/// # Errors
///
/// Returns an error for malformed attributes or an unsupported rename rule.
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

/// Check whether the forwarded Serde attributes contain a flag.
///
/// # Errors
///
/// Returns a syntax error if the forwarded Serde attributes cannot be parsed.
fn serde_has_flag(
    direct: &[Meta],
    partial: &[super::parser::PartialReceiver],
    name: &str,
) -> syn::Result<bool> {
    Ok(forwarded_serde_metas(direct, partial)?
        .iter()
        .any(|meta| matches!(meta, Meta::Path(path) if path.is_ident(name))))
}

/// Resolve a field’s canonical input name for diagnostics and provenance.
///
/// # Errors
///
/// Returns a syntax error if the field’s forwarded Serde attributes are malformed.
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
/// For anything else return `None`.
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

/// Transform the parsed struct into a type describing the output types and impls.
///
/// # Errors
///
/// Returns combined diagnostics for unsupported generics, invalid field policies,
/// or duplicate canonical input names.
pub fn transform_struct(mut receiver: ConfigStructReceiver) -> syn::Result<TransformedStruct> {
    if !receiver.generics.params.is_empty() {
        return Err(syn::Error::new(
            receiver.generics.span(),
            "generic Config structs are not supported",
        ));
    }

    let rename_rule = serde_rename_rule(&receiver.serde, &receiver.partial)?;
    let attrs = receiver.take_partial_attrs();

    let complete_ident = receiver.ident.clone();
    let complete_name = complete_ident.to_string();
    let complete_name = complete_name.strip_prefix("r#").unwrap_or(&complete_name);
    let partial_ident = syn::Ident::new(&format!("{complete_name}Partial"), complete_ident.span());
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
        match transform_field(
            field,
            receiver.freezable,
            receiver.deny_unknown_fields,
            rename_rule,
        ) {
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

    let mut logical_names = std::collections::BTreeMap::<&str, &syn::Ident>::new();
    for field in &fields {
        if field.flattened_subconfig {
            continue;
        }
        if let Some(previous) = logical_names.insert(&field.logical_name, &field.ident) {
            let error = syn::Error::new(
                field.ident.span(),
                format!(
                    "configuration key {:?} is also used by field `{previous}`",
                    field.logical_name
                ),
            );
            if let Some(ref mut errors) = errors {
                errors.combine(error);
            } else {
                errors = Some(error);
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

/// Transform a parsed field into its partial form.
///
/// # Errors
///
/// Returns a diagnostic for unsupported field types, incompatible attributes,
/// or an invalid custom merge function path.
fn transform_field(
    mut field: ConfigFieldReceiver,
    all_freezeable: bool,
    deny_unknown_fields: bool,
    rename_rule: SerdeRenameRule,
) -> syn::Result<TransformedField> {
    let ident = field
        .ident
        .clone()
        .ok_or_else(|| syn::Error::new(field.ty.span(), "Config fields must be named"))?;
    let logical_name = serde_field_name(&field, &ident, rename_rule)?;
    let flattened = serde_has_flag(&field.serde, &field.partial, "flatten")?;
    if flattened && !field.subconfig {
        return Err(syn::Error::new(
            ident.span(),
            "#[config(serde(flatten))] is only supported on #[config(subconfig)] fields",
        ));
    }
    if flattened && deny_unknown_fields {
        return Err(syn::Error::new(
            ident.span(),
            "#[config(deny_unknown_fields)] cannot be combined with #[config(serde(flatten))]",
        ));
    }
    let flattened_subconfig = field.subconfig && flattened;
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

    if flattened_subconfig && let Some(default) = &field.default {
        return Err(syn::Error::new(
            default.span(),
            "#[config(default)] is not supported on a flattened subconfig",
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
        flattened_subconfig,
        validate_func: field.validate,
        attrs,
        build,
        merge,
    })
}
