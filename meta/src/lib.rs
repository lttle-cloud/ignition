use proc_macro::TokenStream;
use quote::{ToTokens, quote};
use syn::{parse_macro_input, spanned::Spanned};

mod analysis;
mod generation;
mod types;

use analysis::{analyze_resource, validate_versions};
use generation::{
    generate_build_info_impl, generate_conversion_methods, generate_provide_key_impl,
    generate_provide_metadata_impl, generate_provide_metadata_impls, generate_schema_enum_variants,
    generate_status_provide_key_impl, generate_status_struct, generate_type_aliases,
    generate_version_enum_variants, generate_version_struct,
};
use types::ResourceArgs;

use crate::types::{SummaryCellStyle, SummaryFieldArgs, TableCellStyle, TableFieldArgs};

#[proc_macro_attribute]
pub fn resource(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as ResourceArgs);
    let resource_mod = parse_macro_input!(input as syn::ItemMod);

    // Phase 1: Extract and analyze
    let analysis = analyze_resource(args, &resource_mod);

    // Phase 2: Validate
    if let Err(e) = validate_versions(&analysis) {
        return e.to_compile_error().into();
    }

    // Phase 3: Generate
    let output = generate_output(analysis, &resource_mod);

    output.into()
}

#[proc_macro_attribute]
pub fn table(_args: TokenStream, input: TokenStream) -> TokenStream {
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
        let cell_style = field_args.cell_style.unwrap_or(TableCellStyle::Default);
        let cell_style = syn::Ident::new(&format!("{:?}", cell_style), field.span());

        let max_width = match field_args.max_width {
            Some(max_width) => quote! {
                Some(#max_width)
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

#[proc_macro_attribute]
pub fn summary(_args: TokenStream, input: TokenStream) -> TokenStream {
    let summary_struct = parse_macro_input!(input as syn::ItemStruct);

    let mut summary_fields = vec![];
    let mut summary_rows = vec![];

    for field in &summary_struct.fields {
        let Some(ident) = &field.ident else {
            continue;
        };

        let ty_text = field.ty.to_token_stream().to_string();

        if ty_text != "String" && ty_text != "Option < String >" {
            return syn::Error::new(
                field.span(),
                format!(
                    "summary field must be a String or Option<String>. found: {}",
                    ty_text
                ),
            )
            .to_compile_error()
            .into();
        }

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
        let cell_style = field_args.cell_style.unwrap_or(SummaryCellStyle::Default);
        let cell_style = syn::Ident::new(&format!("{:?}", cell_style), field.span());

        summary_rows.push(quote! {
            crate::ui::summary::SummaryRow {
                name: #name.to_string(),
                cell_style: crate::ui::summary::SummaryCellStyle::#cell_style,
                value: summary.#ident.clone().into(),
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

fn generate_output(
    analysis: types::ResourceAnalysis,
    resource_mod: &syn::ItemMod,
) -> proc_macro2::TokenStream {
    let enum_name = syn::Ident::new(&analysis.args.name, proc_macro2::Span::call_site());
    let schema_enum_name = syn::Ident::new(
        &format!("{}Schema", enum_name),
        proc_macro2::Span::call_site(),
    );

    // Generate version structs
    let version_structs = analysis.versions.iter().map(|version_info| {
        let (_, items) = resource_mod
            .content
            .clone()
            .expect("resource module must have content");

        let struct_item = items
            .iter()
            .find_map(|item| {
                if let syn::Item::Struct(s) = item {
                    if s.ident == version_info.original_ident {
                        Some(s)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .unwrap();

        generate_version_struct(struct_item, version_info, &analysis.args)
    });

    // Generate status struct
    let status_struct = {
        let (_, items) = resource_mod
            .content
            .clone()
            .expect("resource module must have content");

        let struct_item = items
            .iter()
            .find_map(|item| {
                if let syn::Item::Struct(s) = item {
                    if s.ident == analysis.status.original_ident {
                        Some(s)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .unwrap();

        generate_status_struct(struct_item, &analysis.status)
    };

    let version_enum_variants = generate_version_enum_variants(&analysis.args, &analysis.versions);
    let schema_enum_variants = generate_schema_enum_variants(&analysis.args, &analysis.versions);
    let version_provide_metadata_impls =
        generate_provide_metadata_impls(&analysis.versions, &analysis.args);
    let provide_key_impl = generate_provide_key_impl(&analysis);
    let provide_metadata_impl = generate_provide_metadata_impl(&analysis);
    let provide_key_impl_status = generate_status_provide_key_impl(&analysis);
    let build_info_impl = generate_build_info_impl(&analysis);
    let type_aliases = generate_type_aliases(&analysis);
    let conversion_methods = generate_conversion_methods(&analysis);

    quote::quote! {
        #(#version_structs)*

        #(#version_provide_metadata_impls)*

        #[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
        pub enum #enum_name {
            #(#version_enum_variants),*
        }

        #[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
        pub enum #schema_enum_name {
            #(#schema_enum_variants),*
        }

        #provide_key_impl

        #provide_metadata_impl

        #status_struct

        #provide_key_impl_status

        #build_info_impl

        #type_aliases

        #conversion_methods
    }
}

#[proc_macro]
pub fn include_build_mod(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as syn::LitStr);

    let input_file = format!("/{}.rs", input.value());

    let output = quote::quote! {
        include!(concat!(env!("OUT_DIR"), #input_file));
    };

    output.into()
}
