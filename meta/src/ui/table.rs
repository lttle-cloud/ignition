use proc_macro::TokenStream;
use quote::{ToTokens, quote};
use syn::{parse_macro_input, spanned::Spanned};

use crate::types::TableFieldArgs;

pub fn table_macro(_args: TokenStream, input: TokenStream) -> TokenStream {
    let table_struct = parse_macro_input!(input as syn::ItemStruct);

    let mut row_fields = vec![];
    let mut headers = vec![];
    let mut rows = vec![];

    for field in &table_struct.fields {
        let Some(ident) = &field.ident else {
            continue;
        };

        let ty_text = field.ty.to_token_stream().to_string();

        if ty_text != "String" && ty_text != "Option < String >" {
            return syn::Error::new(
                field.span(),
                format!(
                    "table field must be a String or Option<String>. found: {}",
                    ty_text
                ),
            )
            .to_compile_error()
            .into();
        }

        let field_args = field.attrs.iter().find_map(|attr| {
            if let syn::Meta::List(meta) = &attr.meta {
                if meta.path.is_ident("field") {
                    let args = meta.parse_args::<TableFieldArgs>();
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

        let text = field_args.name.clone().to_uppercase();
        let cell_style = field_args
            .cell_style
            .unwrap_or(crate::types::TableCellStyle::Default);
        let cell_style = syn::Ident::new(&format!("{:?}", cell_style), field.span());

        let max_width = match field_args.max_width {
            Some(max_width) => quote! {
                Some(#max_width)
            },
            None => quote! {
                None
            },
        };

        let min_with = match field_args.min_width {
            Some(min_width) => quote! {
                Some(#min_width)
            },
            None => quote! {
                None
            },
        };

        headers.push(quote! {
            crate::ui::table::TableHeader {
                text: #text.to_string(),
                cell_style: crate::ui::table::TableCellStyle::#cell_style,
                max_width: #max_width,
                min_width: #min_with,
            }
        });

        rows.push(quote! {
            row.#ident.clone().into()
        });

        let mut row_field = field.clone();
        row_field.attrs = vec![];
        row_field.vis = syn::Visibility::Public(Default::default());

        row_fields.push(row_field);
    }

    let table_struct_name = table_struct.ident.clone();
    let row_struct_name = syn::Ident::new(
        &format!("{}Row", table_struct.ident.to_string()),
        table_struct.span(),
    );

    quote! {
        #[derive(Debug, Clone)]
        pub struct #row_struct_name {
            #(#row_fields),*
        }

        #[derive(Debug, Clone)]
        pub struct #table_struct_name {
            rows: Vec<#row_struct_name>,
        }

        impl #table_struct_name {
            pub fn new() -> Self {
                Self {
                    rows: vec![],
                }
            }

            pub fn add_row(&mut self, row: #row_struct_name) {
                self.rows.push(row);
            }

            pub fn print(self) {
                let table = crate::ui::table::Table::from(self);
                table.print();
            }
        }

        impl From<#table_struct_name> for crate::ui::table::Table {
            fn from(table: #table_struct_name) -> Self {
                Self {
                    headers: vec![#(#headers),*],
                    rows: table.rows.into_iter().map(|row| vec![#(#rows),*]).collect(),
                }
            }
        }
    }
    .into()
}
