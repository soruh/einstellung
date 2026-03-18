use super::parser::{ConfigFieldReceiver, ConfigStructReceiver};
use crate::derive_config::parser::MergeStrategy;
use syn::{GenericArgument, Path, PathArguments, Type};

pub struct TransformedStruct {
    pub complete_ident: syn::Ident,
    pub partial_ident: syn::Ident,
    pub vis: syn::Visibility,
    pub fields: Vec<TransformedField>,
    pub einstellung: syn::Path,
}

pub enum ResolvedMerge {
    Replace,
    Append,
    Function(Path),
    Subconfig,
}

pub struct TransformedField {
    pub ident: syn::Ident,
    pub vis: syn::Visibility,
    pub partial_type: syn::Type,
    pub is_optional: bool,
    pub is_subconfig: bool,
    pub default_expr: Option<syn::Expr>,
    pub merge_strategy: ResolvedMerge,
    pub validate_func: Option<syn::Path>,
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

    // Reconstruct attributes by wrapping the Meta directly
    let serde_attrs = field
        .serde
        .into_iter()
        .map(|meta| syn::parse_quote! { #[#meta] })
        .collect();

    let inner_type_if_optional = extract_type_from_option(&field.ty);
    let is_optional = inner_type_if_optional.is_some();
    let core_type = inner_type_if_optional.cloned().unwrap_or(field.ty);

    let partial_type = if field.subconfig {
        syn::parse_quote!(Option<<#core_type as #einstellung::Config>::Partial>)
    } else {
        syn::parse_quote!(Option<#core_type>)
    };

    // Resolve Merge Strategy and handle parsing errors
    let merge_strategy = if let Some(strategy) = field.merge {
        let span = strategy.span();
        if field.subconfig {
            return Err(syn::Error::new(
                span,
                "Merge strategy is invalid on a subconfig",
            ));
        }

        match strategy.into_inner() {
            MergeStrategy::Replace => ResolvedMerge::Replace,
            MergeStrategy::Extend => ResolvedMerge::Append,
            MergeStrategy::Function(s) => {
                let path = syn::parse_str::<Path>(&s).map_err(|_| {
                    syn::Error::new(span, format!("Invalid function path: '{}'", s))
                })?;
                ResolvedMerge::Function(path)
            }
        }
    } else if field.subconfig {
        ResolvedMerge::Subconfig
    } else {
        ResolvedMerge::Replace
    };

    Ok(TransformedField {
        ident,
        vis: field.vis,
        partial_type,
        is_optional,
        is_subconfig: field.subconfig,
        default_expr: field.default,
        merge_strategy,
        validate_func: field.validate,
        serde_attrs,
    })
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
