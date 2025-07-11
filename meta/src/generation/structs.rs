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

    named.insert(0, syn::parse_quote!(name: String));
    if args.namespaced {
        named.insert(0, syn::parse_quote!(namespace: Option<String>));
    }

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
