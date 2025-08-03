use proc_macro::TokenStream;
use quote::{ToTokens, quote};
use syn::{parse_macro_input, spanned::Spanned};

use crate::types::SummaryFieldArgs;

pub fn summary_macro(_args: TokenStream, input: TokenStream) -> TokenStream {
    let summary_struct = parse_macro_input!(input as syn::ItemStruct);

    let mut summary_fields = vec![];
    let mut summary_rows = vec![];

    for field in &summary_struct.fields {
        let Some(ident) = &field.ident else {
            continue;
        };

        let ty_text = field.ty.to_token_stream().to_string();

        if ty_text != "String" && ty_text != "Option < String >" && ty_text != "Vec < String >" {
            return syn::Error::new(
                field.span(),
                format!(
                    "summary field must be a String, Option<String> or Vec<String>. found: {}",
                    ty_text
                ),
            )
            .to_compile_error()
            .into();
        }
        let is_vec = ty_text == "Vec < String >";
        let is_option = ty_text == "Option < String >";

        let field_args = field.attrs.iter().find_map(|attr| {
            if let syn::Meta::List(meta) = &attr.meta {
                if meta.path.is_ident("field") {
                    let args = meta.parse_args::<SummaryFieldArgs>();
                    if let Ok(args) = args {
                        return Some(args);
                    }
                }
            }

            None
        });

        let Some(field_args) = field_args else {
            panic!("field must have a field attribute");
        };

        let name = field_args.name.clone();
        let cell_style = field_args
            .cell_style
            .unwrap_or(crate::types::SummaryCellStyle::Default);
        let cell_style = syn::Ident::new(&format!("{:?}", cell_style), field.span());

        let value_set = if is_vec {
            quote! {
                summary.#ident.clone()
            }
        } else if is_option {
            quote! {
                summary.#ident.as_ref().map(|v| vec![v.clone()]).unwrap_or_default()
            }
        } else {
            quote! {
                vec![summary.#ident.clone()]
            }
        };

        summary_rows.push(quote! {
            crate::ui::summary::SummaryRow {
                name: #name.to_string(),
                cell_style: crate::ui::summary::SummaryCellStyle::#cell_style,
                value: #value_set,
            }
        });

        let mut summary_field = field.clone();
        summary_field.attrs = vec![];
        summary_field.vis = syn::Visibility::Public(Default::default());

        summary_fields.push(summary_field);
    }

    let summary_struct_name = summary_struct.ident.clone();

    quote! {
        #[derive(Debug, Clone)]
        pub struct #summary_struct_name {
            #(#summary_fields),*
        }

        impl From<#summary_struct_name> for crate::ui::summary::Summary {
            fn from(summary: #summary_struct_name) -> Self {
                Self {
                    rows: vec![#(#summary_rows),*],
                }
            }
        }

        impl #summary_struct_name {
            pub fn print(self) {
                let summary = crate::ui::summary::Summary::from(self);
                summary.print();
            }
        }
    }
    .into()
}
