use darling::{FromDeriveInput, FromField, ast, util::SpannedValue};
use proc_macro2::Span;
use syn::{Ident, PathArguments, parse_quote_spanned, spanned::Spanned};

#[derive(Debug)]
pub struct PartialReceiver(pub Vec<darling::ast::NestedMeta>);

impl darling::FromMeta for PartialReceiver {
    fn from_list(items: &[darling::ast::NestedMeta]) -> darling::Result<Self> {
        Ok(PartialReceiver(items.to_vec()))
    }
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
    #[darling(default = || syn::Path {
        leading_colon: Some(Default::default()),
        segments: std::iter::once(syn::PathSegment {
            ident: Ident::new("einstellung", Span::call_site()),
            arguments: PathArguments::None,
        }).collect(),
    })]
    pub einstellung: syn::Path,
}

#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub enum DefaultStrategyReceiver {
    #[default]
    None,
    DefaultTrait,
    Value(syn::Expr),
    Call(syn::Expr),
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
    pub default: DefaultStrategyReceiver,

    #[darling(default)]
    pub subconfig: bool,

    #[darling(default)]
    pub merge: Option<SpannedValue<MergeStrategyReceiver>>,

    #[darling(default)]
    pub validate: Option<syn::Expr>,
}

impl ConfigStructReceiver {
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

#[derive(Debug, darling::FromMeta)]
pub enum MergeStrategyReceiver {
    Replace,
    Extend,
    Function(SpannedValue<String>),
}

/// Custom parser for the 'default' attribute field
fn parse_default_expr(meta: &syn::Meta) -> darling::Result<DefaultStrategyReceiver> {
    match meta {
        syn::Meta::Path(_) => Ok(DefaultStrategyReceiver::DefaultTrait),
        syn::Meta::NameValue(nv) => {
            let expr = &nv.value;

            use syn::Expr::*;
            Ok(match expr {
                Closure(_) => DefaultStrategyReceiver::Call(expr.clone()),
                Call(call) if call.args.is_empty() => {
                    DefaultStrategyReceiver::Call((*call.func).clone())
                }
                Call(_) => {
                    DefaultStrategyReceiver::Call(parse_quote_spanned!(expr.span() => || #expr))
                }
                _ => DefaultStrategyReceiver::Value(expr.clone()),
            })
        }
        _ => Err(darling::Error::unsupported_format(
            "expected default or default = ...",
        )),
    }
}

pub fn parse(input: syn::DeriveInput) -> Result<ConfigStructReceiver, darling::Error> {
    ConfigStructReceiver::from_derive_input(&input)
}
