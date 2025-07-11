use proc_macro::TokenStream;
use syn::parse_macro_input;

mod analysis;
mod generation;
mod types;
mod ui;

use analysis::{analyze_resource, validate_versions};
use generation::{
    generate_build_info_impl, generate_conversion_methods, generate_provide_key_impl,
    generate_provide_metadata_impl, generate_provide_metadata_impls, generate_schema_enum_variants,
    generate_status_provide_key_impl, generate_status_struct, generate_type_aliases,
    generate_version_enum_variants, generate_version_struct,
};
use types::ResourceArgs;

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
pub fn table(args: TokenStream, input: TokenStream) -> TokenStream {
    ui::table::table_macro(args, input)
}

#[proc_macro_attribute]
pub fn summary(args: TokenStream, input: TokenStream) -> TokenStream {
    ui::summary::summary_macro(args, input)
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
