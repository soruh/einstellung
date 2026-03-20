use quote::ToTokens;

pub mod generator;
pub mod parser;
pub mod transformer;

pub fn derive(input: proc_macro2::TokenStream) -> proc_macro2::TokenStream {
    let input: syn::DeriveInput = match syn::parse2(input) {
        Ok(val) => val,
        Err(e) => return e.to_compile_error(),
    };

    let parsed = match parser::parse(input) {
        Ok(p) => p,
        Err(e) => return e.write_errors(),
    };

    let model = match transformer::transform_struct(parsed) {
        Ok(m) => m,
        Err(e) => return e.to_compile_error(),
    };

    model.to_token_stream()
}
