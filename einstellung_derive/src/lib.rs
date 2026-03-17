use proc_macro::TokenStream;

#[cfg(test)]
mod test;

mod derive_config;

#[proc_macro_derive(Config, attributes(config))]
pub fn derive_config(input: TokenStream) -> TokenStream {
    derive_config::derive(input.into()).into()
}
