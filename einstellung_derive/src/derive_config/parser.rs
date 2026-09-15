//! Receive Rust syntax and configuration attributes with Darling.

use darling::{FromDeriveInput, FromField, ast, util::SpannedValue};
use proc_macro2::Span;
use syn::{Ident, PathArguments, parse_quote_spanned, spanned::Spanned};

/// Parse the derive input and collect supported configuration attributes.
///
/// # Errors
///
/// Returns an error for unsupported syntax or malformed configuration attributes.
pub fn parse(input: syn::DeriveInput) -> Result<ConfigStructReceiver, darling::Error> {
    ConfigStructReceiver::from_derive_input(&input)
}

#[derive(FromDeriveInput)]
#[darling(attributes(config), supports(struct_named))]
/// Parsed struct syntax and configuration attributes before semantic validation.
pub struct ConfigStructReceiver {
    /// Original Rust identifier used when emitting the corresponding declaration.
    pub ident: syn::Ident,
    /// Visibility preserved on the generated declaration.
    pub vis: syn::Visibility,
    /// Generic parameters retained so unsupported generic derives can be rejected explicitly.
    pub generics: syn::Generics,
    /// Named fields received from the input struct.
    pub data: ast::Data<darling::util::Ignored, ConfigFieldReceiver>,

    #[darling(default, multiple)]
    /// Serde attributes forwarded to the partial declaration.
    pub serde: Vec<syn::Meta>,

    #[darling(default, multiple)]
    /// Additional attributes forwarded to the partial declaration.
    pub partial: Vec<PartialReceiver>,

    #[darling(default)]
    /// Whether this declaration opts its fields into freeze-aware merging.
    pub freezable: bool,

    #[darling(default)]
    /// Whether the generated partial rejects unknown input keys.
    pub deny_unknown_fields: bool,

    #[darling(rename = "crate")]
    #[darling(default = default_crate_path)]
    /// Path to the runtime crate, including an explicit crate-path override.
    pub einstellung: syn::Path,
}

#[derive(FromField)]
#[darling(attributes(config), forward_attrs(doc))]
/// Parsed field syntax and attributes before partial-type transformation.
pub struct ConfigFieldReceiver {
    /// Original field documentation preserved on the generated partial field.
    pub attrs: Vec<syn::Attribute>,
    /// Original Rust identifier used when emitting the corresponding declaration.
    pub ident: Option<syn::Ident>,
    /// Visibility preserved on the generated declaration.
    pub vis: syn::Visibility,
    /// Complete Rust field type before optional and freeze wrappers are introduced.
    pub ty: syn::Type,

    #[darling(default)]
    /// Whether this declaration opts its fields into freeze-aware merging.
    pub freezable: bool,

    #[darling(default, multiple)]
    /// Serde attributes forwarded to the partial declaration.
    pub serde: Vec<syn::Meta>,

    #[darling(default, multiple)]
    /// Additional attributes forwarded to the partial declaration.
    pub partial: Vec<PartialReceiver>,

    #[darling(default, with = "parse_default_expr")]
    /// Fallback expression and source span for a missing final field.
    pub default: Option<SpannedValue<DefaultStrategy>>,

    #[darling(default)]
    /// Whether the field recursively loads and builds another configuration.
    pub subconfig: bool,

    #[darling(default)]
    /// Selected merge strategy for combining existing and incoming values.
    pub merge: Option<SpannedValue<MergeStrategyReceiver>>,

    #[darling(default)]
    /// Expression invoked with a reference to the completed field value.
    pub validate: Option<syn::Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// How a missing final field obtains its fallback value.
pub enum DefaultStrategy {
    /// Construct the fallback with the type’s `Default` implementation.
    DefaultTrait,
    /// Evaluate this expression lazily when the field needs a fallback.
    Value(syn::Expr),
    /// Invoke this function or closure when the field needs a fallback.
    Call(syn::Expr),
}

#[derive(Debug, darling::FromMeta)]
/// Merge strategy as accepted by the attribute parser.
pub enum MergeStrategyReceiver {
    /// Keep the incoming value when present, otherwise retain the existing value.
    Replace,
    /// Append incoming collection contents using `Extend`.
    Extend,
    /// User-supplied merge function path and its diagnostic span.
    Function(SpannedValue<String>),
}

impl ConfigStructReceiver {
    /// Helper to merge all attributes intended for the partial type
    /// This removes the attributes from the receiver.
    pub fn take_partial_attrs(&mut self) -> Vec<syn::Attribute> {
        let partial_attrs = std::mem::take(&mut self.partial)
            .into_iter()
            .flat_map(|meta| meta.0);

        let serde_attrs = std::mem::take(&mut self.serde)
            .into_iter()
            .map(darling::ast::NestedMeta::Meta);

        partial_attrs
            .chain(serde_attrs)
            .map(|meta| syn::parse_quote!(#[#meta]))
            .collect()
    }
}

impl ConfigFieldReceiver {
    /// Helper to merge all attributes intended for the partial type
    /// This removes the attributes from the receiver.
    pub fn take_partial_attrs(&mut self) -> Vec<syn::Attribute> {
        let partial_attrs = std::mem::take(&mut self.partial)
            .into_iter()
            .flat_map(|meta| meta.0);

        let serde_attrs = std::mem::take(&mut self.serde)
            .into_iter()
            .map(darling::ast::NestedMeta::Meta);

        let forwarded = partial_attrs
            .chain(serde_attrs)
            .map(|meta| syn::parse_quote!(#[#meta]))
            .collect::<Vec<syn::Attribute>>();
        std::mem::take(&mut self.attrs)
            .into_iter()
            .chain(forwarded)
            .collect()
    }
}

/// Helper to parse expressions passed as `default = `.
///
/// # Errors
///
/// Returns an error if the default attribute uses unsupported syntax.
fn parse_default_expr(meta: &syn::Meta) -> darling::Result<Option<SpannedValue<DefaultStrategy>>> {
    let res = match meta {
        syn::Meta::Path(_) => DefaultStrategy::DefaultTrait,
        syn::Meta::NameValue(nv) => {
            let expr = &nv.value;

            use syn::Expr::*;
            match expr {
                Closure(_) => DefaultStrategy::Call(expr.clone()),
                Call(call) if call.args.is_empty() => DefaultStrategy::Call((*call.func).clone()),
                Call(_) => DefaultStrategy::Call(parse_quote_spanned!(expr.span() => || #expr)),
                _ => DefaultStrategy::Value(expr.clone()),
            }
        }
        _ => {
            return Err(
                darling::Error::unsupported_format("expected default or default = ...")
                    .with_span(&meta.span()),
            );
        }
    };

    Ok(Some(SpannedValue::new(res, meta.span())))
}

/// Helper to receive `#[config(partial(...))]` attributes.
#[derive(Debug)]
pub struct PartialReceiver(pub Vec<darling::ast::NestedMeta>);
impl darling::FromMeta for PartialReceiver {
    fn from_list(items: &[darling::ast::NestedMeta]) -> darling::Result<Self> {
        Ok(PartialReceiver(items.to_vec()))
    }
}

/// Generates a `syn::Path` pointing to the extern crate `einstellung`.
fn default_crate_path() -> syn::Path {
    syn::Path {
        leading_colon: Some(Default::default()),
        segments: std::iter::once(syn::PathSegment {
            ident: Ident::new("einstellung", Span::call_site()),
            arguments: PathArguments::None,
        })
        .collect(),
    }
}
