use darling::{FromDeriveInput, FromField, ast, util::SpannedValue};
use proc_macro2::Span;
use syn::{Ident, PathArguments, parse_quote_spanned, spanned::Spanned};

pub fn parse(input: syn::DeriveInput) -> Result<ConfigStructReceiver, darling::Error> {
    ConfigStructReceiver::from_derive_input(&input)
}

#[derive(FromDeriveInput)]
#[darling(attributes(config), supports(struct_named))]
pub struct ConfigStructReceiver {
    pub ident: syn::Ident,
    pub vis: syn::Visibility,
    pub data: ast::Data<darling::util::Ignored, ConfigFieldReceiver>,

    #[darling(default, multiple)]
    pub serde: Vec<syn::Meta>,

    #[darling(default, multiple)]
    pub partial: Vec<PartialReceiver>,

    #[darling(default)]
    pub freezable: bool,

    #[darling(rename = "crate")]
    #[darling(default = default_crate_path)]
    pub einstellung: syn::Path,
}

#[derive(FromField)]
#[darling(attributes(config))]
pub struct ConfigFieldReceiver {
    pub ident: Option<syn::Ident>,
    pub vis: syn::Visibility,
    pub ty: syn::Type,

    #[darling(default)]
    pub freezable: bool,

    #[darling(default, multiple)]
    pub serde: Vec<syn::Meta>,

    #[darling(default, multiple)]
    pub partial: Vec<PartialReceiver>,

    #[darling(default, with = "parse_default_expr")]
    pub default: Option<SpannedValue<DefaultStrategy>>,

    #[darling(default)]
    pub subconfig: bool,

    #[darling(default)]
    pub merge: Option<SpannedValue<MergeStrategyReceiver>>,

    #[darling(default)]
    pub validate: Option<syn::Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefaultStrategy {
    DefaultTrait,
    Value(syn::Expr),
    Call(syn::Expr),
}

#[derive(Debug, darling::FromMeta)]
pub enum MergeStrategyReceiver {
    Replace,
    Extend,
    Function(SpannedValue<String>),
}

impl ConfigStructReceiver {
    /// Helper to merge all attributes intended for the partial type
    /// This removes the attributes from the receiver
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
    /// This removes the attributes from the receiver
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

/// Helper to parse expressions passed as `default = `
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

/// Helper to receive `#[config(partial(...))]` attributes
#[derive(Debug)]
pub struct PartialReceiver(pub Vec<darling::ast::NestedMeta>);
impl darling::FromMeta for PartialReceiver {
    fn from_list(items: &[darling::ast::NestedMeta]) -> darling::Result<Self> {
        Ok(PartialReceiver(items.to_vec()))
    }
}

/// Generates a `syn::Path` pointing to the extern crate `einstellung`
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
