use proc_macro2::{Ident, TokenStream};
use quote::quote;
use syn::{DeriveInput, parse_macro_input};

pub fn expand(input: TokenStream) -> TokenStream {
    let input: DeriveInput = syn::parse2(input.clone()).expect("Failed to parse input as DeriveInput");
    
    match expand_config(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

fn expand_config(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {

    let struct_ident = &input.ident;

    // let fields = parse_struct_fields(&input)?;
    // let metadata = generate_metadata(struct_ident, &fields);
    // let loader = generate_loader(struct_ident, &fields);

    Ok(quote! {
        impl einstellung::Config for #struct_ident {

            fn metadata() -> &'static einstellung::ConfigMetadata {
                // #metadata
            }

            fn load(
                source: &dyn einstellung::ConfigSource
            ) -> Result<Self, einstellung::ConfigError> {
                // #loader
            }
        }
    })
}

// fn parse_struct_fields(input: &DeriveInput) -> syn::Result<Vec<Field>> {

//     let data = match &input.data {
//         syn::Data::Struct(s) => s,
//         _ => {
//             return Err(syn::Error::new_spanned(
//                 input,
//                 "Config can only be derived for structs"
//             ))
//         }
//     };

//     let fields = match &data.fields {
//         syn::Fields::Named(fields) => &fields.named,
//         _ => {
//             return Err(syn::Error::new_spanned(
//                 data,
//                 "Config requires named fields"
//             ))
//         }
//     };

//     fields.iter().map(Field::parse).collect()
// }

// pub struct Field {
//     pub ident: syn::Ident,
//     pub ty: syn::Type,
//     pub attrs: FieldAttr,
// }

// impl Field {

//     pub fn parse(field: &syn::Field) -> syn::Result<Self> {

//         let ident = field.ident.clone().unwrap();
//         let ty = field.ty.clone();

//         let attrs = FieldAttr::parse(&field.attrs)?;

//         Ok(Self { ident, ty, attrs })
//     }

// }

// #[derive(Default)]
// pub struct FieldAttr {
//     pub required: bool,
//     pub default: Option<syn::Lit>,
//     pub rename: Option<String>,
// }

// impl FieldAttr {

//     pub fn parse(attrs: &[syn::Attribute]) -> syn::Result<Self> {

//         let mut result = FieldAttr::default();

//         for attr in attrs {

//             if !attr.path().is_ident("config") {
//                 continue;
//             }

//             attr.parse_nested_meta(|meta| {

//                 if meta.path.is_ident("required") {
//                     result.required = true;
//                     return Ok(());
//                 }

//                 if meta.path.is_ident("default") {

//                     let value: syn::Lit = meta.value()?.parse()?;
//                     result.default = Some(value);
//                     return Ok(());
//                 }

//                 Err(meta.error("unknown config attribute"))
//             })?;
//         }

//         Ok(result)
//     }
// }

// fn generate_metadata(
//     struct_ident: &syn::Ident,
//     fields: &[Field]
// ) -> TokenStream {

//     let field_meta = fields.iter().map(|f| {

//         let name = f.ident.to_string();
//         let required = f.attrs.required;

//         quote! {
//             einstellung::FieldMetadata {
//                 name: #name,
//                 required: #required,
//             }
//         }
//     });

//     quote! {

//         {
//             static META: einstellung::ConfigMetadata =
//                 einstellung::ConfigMetadata {

//                     name: stringify!(#struct_ident),

//                     fields: &[
//                         #( #field_meta ),*
//                     ]
//                 };

//             &META
//         }

//     }
// }

// fn generate_loader(
//     struct_ident: &Ident,
//     fields: &[Field]
// ) -> TokenStream {

//     let defaults = fields.iter().filter_map(|f| {

//         let name = &f.ident;

//         let default = f.attrs.default.as_ref()?;

//         Some(quote! {
//             if cfg.#name == Default::default() {
//                 cfg.#name = #default.into();
//             }
//         })
//     });

//     quote! {

//         let mut cfg: #struct_ident = source.load()?;

//         #( #defaults )*

//         cfg.validate()?;

//         Ok(cfg)

//     }
// }
