use darling::{FromDeriveInput, FromField, ast, util::SpannedValue};
use proc_macro2::Span;
use syn::{Ident, PathArguments};

#[derive(FromDeriveInput)]
#[darling(attributes(config), supports(struct_named))]
pub struct ConfigStructReceiver {
    pub ident: syn::Ident,
    pub vis: syn::Visibility,
    pub data: ast::Data<darling::util::Ignored, ConfigFieldReceiver>,

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

#[derive(FromField)]
#[darling(attributes(config))]
pub struct ConfigFieldReceiver {
    pub ident: Option<syn::Ident>,
    pub vis: syn::Visibility,
    pub ty: syn::Type,

    #[darling(default, multiple)]
    pub serde: Vec<syn::Meta>,

    #[darling(default)]
    pub default: Option<syn::Expr>,
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
    Function(String),
}

pub fn parse(input: syn::DeriveInput) -> Result<ConfigStructReceiver, darling::Error> {
    ConfigStructReceiver::from_derive_input(&input)
}
