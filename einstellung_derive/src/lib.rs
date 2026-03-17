use proc_macro::TokenStream;

#[cfg(test)]
mod tests;

mod derive_config;

#[proc_macro_derive(Config, attributes(config))]
pub fn derive_config(input: TokenStream) -> TokenStream {
    derive_config::derive(input.into()).into()
}
