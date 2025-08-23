use crate::types::{ResourceArgs, StatusInfo, VersionInfo};
use proc_macro2::Span;
use syn::{Fields, FieldsNamed, spanned::Spanned};

pub fn generate_version_struct(
    struct_item: &syn::ItemStruct,
    version_info: &VersionInfo,
    args: &ResourceArgs,
) -> syn::ItemStruct {
    let mut item = struct_item.clone();

    let Fields::Named(FieldsNamed {
        brace_token,
        mut named,
    }) = item.fields
    else {
        panic!("struct must have named fields");
    };

    named.insert(
        0,
        syn::parse_quote!(#[serde(deserialize_with = "super::de_trim_non_empty_string")] name: String),
    );
    if args.namespaced {
        named.insert(
            0,
            syn::parse_quote!(#[serde(deserialize_with = "super::de_opt_trim_non_empty_string")] namespace: Option<String>),
        );
    }
    named.insert(0, syn::parse_quote!(tags: Option<Vec<String>>));

    item.attrs = syn::parse_quote!(#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]);

    item.vis = syn::Visibility::Public(syn::token::Pub {
        span: Span::call_site(),
    });
    item.fields = Fields::Named(FieldsNamed {
        brace_token,
        named: named.clone(),
    });
    item.fields.iter_mut().for_each(|field| {
        field.vis = syn::Visibility::Public(syn::token::Pub {
            span: Span::call_site(),
        });
    });
    item.ident = version_info.generated_ident.clone();

    item
}

pub fn generate_status_struct(
    struct_item: &syn::ItemStruct,
    status_info: &StatusInfo,
) -> syn::ItemStruct {
    let mut item = struct_item.clone();
    item.attrs = syn::parse_quote!(#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]);
    item.fields.iter_mut().for_each(|field| {
        field.vis = syn::Visibility::Public(syn::token::Pub { span: field.span() });
    });
    item.ident = status_info.generated_ident.clone();
    item.vis = syn::Visibility::Public(syn::token::Pub {
        span: item.ident.span(),
    });
    item
}

pub fn generate_additional_schema_item(item: &syn::Item, _args: &ResourceArgs) -> syn::Item {
    let add_attrs = syn::parse_quote!(#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]);
    match item {
        syn::Item::Struct(struct_item) => {
            let mut struct_item = struct_item.clone();
            struct_item.attrs = add_attrs;
            struct_item.vis = syn::Visibility::Public(syn::token::Pub {
                span: struct_item.ident.span(),
            });
            struct_item.fields.iter_mut().for_each(|field| {
                field.vis = syn::Visibility::Public(syn::token::Pub { span: field.span() });
            });
            syn::Item::Struct(struct_item)
        }
        syn::Item::Enum(enum_item) => {
            let mut enum_item = enum_item.clone();
            enum_item.attrs = add_attrs;
            enum_item.vis = syn::Visibility::Public(syn::token::Pub {
                span: enum_item.ident.span(),
            });
            syn::Item::Enum(enum_item)
        }
        _ => panic!("invalid item"),
    }
}
