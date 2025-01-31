use darling::ast::NestedMeta;
use darling::{Error, FromMeta};
use quote::quote;

#[derive(Debug, FromMeta)]
struct CodecArgs {
    #[darling(default)]
    encoding: Option<bool>,

    #[darling(default)]
    decoding: Option<bool>,
}

#[proc_macro_attribute]
pub fn codec(
    args: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let attr_args = match NestedMeta::parse_meta_list(args.into()) {
        Ok(v) => v,
        Err(e) => {
            return proc_macro::TokenStream::from(Error::from(e).write_errors());
        }
    };

    let item = syn::parse_macro_input!(item as syn::Item);

    let args = match CodecArgs::from_list(&attr_args) {
        Ok(v) => v,
        Err(e) => {
            return proc_macro::TokenStream::from(e.write_errors());
        }
    };

    let encoding = args.encoding.unwrap_or(true);
    let decoding = args.decoding.unwrap_or(true);

    if !encoding && !decoding {
        return proc_macro::TokenStream::from(
            Error::custom("either encoding or decoding must be enabled").write_errors(),
        );
    }

    let derive_args = if encoding && decoding {
        quote! {
            #[derive(util::_serde::Serialize, util::_serde::Deserialize)]
        }
    } else if encoding {
        quote! {
            #[derive(util::_serde::Serialize)]
        }
    } else {
        quote! {
            #[derive(util::_serde::Deserialize)]
        }
    };

    quote! {
        #derive_args
        #[serde(crate = "util::_serde")]
        #item
    }
    .into()
}
