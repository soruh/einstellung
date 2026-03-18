use darling::{FromDeriveInput, FromField, ast, util::SpannedValue};
use proc_macro2::Span;
use syn::{Ident, PathArguments, parse_quote};

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
pub enum DefaultStrategy {
    #[default]
    Required,
    Standard,
    Value(syn::Expr),
    Call(syn::Expr),
}

/// Custom parser for the 'default' attribute field
fn parse_default_expr(meta: &syn::Meta) -> darling::Result<DefaultStrategy> {
    match meta {
        syn::Meta::Path(_) => Ok(DefaultStrategy::Standard),
        syn::Meta::NameValue(nv) => {
            let expr = &nv.value;

            match expr {
                // Match closures: `default = || ...`
                // OR explicit calls: `default = my_function()`
                syn::Expr::Closure(_) => Ok(DefaultStrategy::Call(expr.clone())),

                syn::Expr::Call(call) if call.args.is_empty() => {
                    Ok(DefaultStrategy::Call((*call.func).clone()))
                }
                syn::Expr::Call(_) => Ok(DefaultStrategy::Call(parse_quote!(|| #expr))),
                // Everything else (MyEnum::Variant, "strings", numbers) falls through to Value
                _ => Ok(DefaultStrategy::Value(expr.clone())),
            }
        }
        _ => Err(darling::Error::unsupported_format(
            "expected default or default = ...",
        )),
    }
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

    #[darling(default, with = "parse_default_expr")]
    pub default: DefaultStrategy,

    #[darling(default)]
    pub subconfig: bool,

    #[darling(default)]
    pub merge: Option<SpannedValue<MergeStrategy>>,

    #[darling(default)]
    pub validate: Option<syn::Expr>,
}

#[derive(Debug, darling::FromMeta)]
pub enum MergeStrategy {
    Replace,
    Extend,
    Function(SpannedValue<String>),
}

pub fn parse(input: syn::DeriveInput) -> Result<ConfigStructReceiver, darling::Error> {
    ConfigStructReceiver::from_derive_input(&input)
}
