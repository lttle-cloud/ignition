use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::{
    Field, Fields, FieldsNamed, Ident, MetaNameValue, Result, Token, Variant,
    parse::{Parse, ParseStream},
    parse_macro_input,
    punctuated::Punctuated,
    spanned::Spanned,
};

struct TagsList(pub Punctuated<Ident, Token![+]>);

impl Parse for TagsList {
    fn parse(input: ParseStream) -> Result<Self> {
        let list = input.parse_terminated(Ident::parse, Token![+])?;
        Ok(TagsList(list))
    }
}

impl TagsList {
    pub fn tags(&self) -> Vec<String> {
        self.0.iter().map(|ident| ident.to_string()).collect()
    }
}

#[derive(Debug)]
struct ResourceArgs {
    pub name: String,
    pub tag: String,
    pub namespaced: bool,
}

impl Parse for ResourceArgs {
    fn parse(input: ParseStream) -> Result<Self> {
        let list = input.parse_terminated(MetaNameValue::parse, Token![,])?;

        let mut name = None;
        let mut tag = None;
        let mut namespaced = true;

        for item in list {
            if item.path.is_ident("name") {
                let syn::Expr::Lit(syn::ExprLit {
                    attrs: _,
                    lit: syn::Lit::Str(lit),
                    ..
                }) = item.value
                else {
                    return Err(syn::Error::new(item.value.span(), "name must be a string"));
                };

                name = Some(lit.value());
            } else if item.path.is_ident("tag") {
                let syn::Expr::Lit(syn::ExprLit {
                    attrs: _,
                    lit: syn::Lit::Str(lit),
                    ..
                }) = item.value
                else {
                    return Err(syn::Error::new(item.value.span(), "tag must be a string"));
                };

                tag = Some(lit.value());
            } else if item.path.is_ident("namespaced") {
                let syn::Expr::Lit(syn::ExprLit {
                    attrs: _,
                    lit: syn::Lit::Bool(lit),
                    ..
                }) = item.value
                else {
                    return Err(syn::Error::new(
                        item.value.span(),
                        "namespaced must be a boolean",
                    ));
                };

                namespaced = lit.value();
            }
        }

        let name = name.ok_or(syn::Error::new(Span::call_site(), "name is required"))?;
        let tag = tag.ok_or(syn::Error::new(Span::call_site(), "tag is required"))?;

        Ok(ResourceArgs {
            name,
            tag,
            namespaced,
        })
    }
}

#[proc_macro_attribute]
pub fn resource(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as ResourceArgs);
    let resource_mod = parse_macro_input!(input as syn::ItemMod);

    let mut version_structs = Vec::new();
    let mut version_enum_variants = Vec::<Variant>::new();
    let mut version_provide_metadata_impls = Vec::new();
    let mut status_struct = None;

    for item in resource_mod.content.unwrap().1 {
        if let syn::Item::Struct(struct_item) = item {
            for attr in struct_item.attrs.iter() {
                if attr.path().is_ident("version") {
                    if let syn::Meta::List(list) = &attr.meta {
                        let tags = list.parse_args::<TagsList>().unwrap();
                    }

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

                    item.attrs = syn::parse_quote!(#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]);

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
                    item.ident = syn::Ident::new(
                        &format!("{}{}", args.name, struct_item.ident),
                        struct_item.ident.span(),
                    );

                    version_structs.push(item.clone());
                    let variant_name = struct_item.ident.clone();
                    let variant_value = item.ident.clone();

                    version_enum_variants.push(syn::parse_quote!(#variant_name (#variant_value)));

                    if args.namespaced {
                        version_provide_metadata_impls.push(quote! {
                            impl super::ProvideMetadata for #variant_value {
                                fn metadata(&self) -> super::metadata::Metadata {
                                    super::metadata::Metadata::new(self.name.clone(), self.namespace.clone())
                                }
                            }
                        });
                    } else {
                        version_provide_metadata_impls.push(quote! {
                            impl super::ProvideMetadata for #variant_value {
                                fn metadata(&self) -> super::metadata::Metadata {
                                    super::metadata::Metadata::new(self.name.clone(), None)
                                }
                            }
                        });
                    }
                } else if attr.path().is_ident("status") {
                    if status_struct.is_some() {
                        panic!("status struct already defined");
                    }

                    let mut item = struct_item.clone();
                    item.attrs = syn::parse_quote!(#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]);
                    item.fields.iter_mut().for_each(|field| {
                        field.vis = syn::Visibility::Public(syn::token::Pub { span: field.span() });
                    });
                    item.ident = syn::Ident::new(
                        &format!("{}{}", args.name, struct_item.ident),
                        struct_item.ident.span(),
                    );

                    item.vis = syn::Visibility::Public(syn::token::Pub {
                        span: item.ident.span(),
                    });

                    status_struct = Some(item.clone());
                }
            }
        }
    }

    let enum_name = syn::Ident::new(&args.name, Span::call_site());
    let collection_name = args.tag.clone();

    let provide_key_impl = if args.namespaced {
        quote! {
            impl super::ProvideKey for #enum_name {
                fn key(
                    tenant: String,
                    metadata: super::metadata::Metadata,
                ) -> crate::machinery::store::Key<#enum_name> {
                    crate::machinery::store::Key::<#enum_name>::namespaced()
                        .collection(#collection_name)
                        .tenant(tenant)
                        .namespace(metadata.namespace)
                        .key(metadata.name)
                        .as_ref()
                        .into()
                }

                fn partial_key(
                    tenant: String,
                    namespace: Option<String>,
                ) -> crate::machinery::store::PartialKey<#enum_name> {
                    let builder = crate::machinery::store::PartialKey::<#enum_name>::namespaced()
                        .collection(#collection_name)
                        .tenant(tenant);

                    if let Some(namespace) = namespace {
                        return builder.namespace(namespace).as_ref().into();
                    }

                    builder.as_ref().into()
                }
            }
        }
    } else {
        quote! {
            impl super::ProvideKey for #enum_name {
                fn key(
                    tenant: String,
                    metadata: super::metadata::Metadata,
                ) -> crate::machinery::store::Key<#enum_name> {
                    crate::machinery::store::Key::<#enum_name>::not_namespaced()
                        .collection(#collection_name)
                        .tenant(tenant)
                        .key(metadata.name)
                        .as_ref()
                        .into()
                }

                fn partial_key(
                    tenant: String,
                    _namespace: Option<String>,
                ) -> crate::machinery::store::PartialKey<#enum_name> {
                    crate::machinery::store::PartialKey::<#enum_name>::not_namespaced()
                        .collection(#collection_name)
                        .tenant(tenant)
                        .as_ref()
                        .into()
                }
            }
        }
    };

    let version_enum_variants_with_metadata = version_enum_variants.iter().map(|variant| {
        let variant_name = variant.ident.clone();
        quote! {
            #enum_name::#variant_name (item) => item.metadata(),
        }
    });

    let provide_metadata_impl = quote! {
        impl super::ProvideMetadata for #enum_name {
            fn metadata(&self) -> super::metadata::Metadata {
                match self {
                    #(#version_enum_variants_with_metadata)*
                }
            }
        }
    };

    let mut provide_key_impl_status = None;

    if let Some(status_struct) = &status_struct {
        let status_collection_name = format!("status-{}", collection_name);

        let status_struct_name = status_struct.ident.clone();
        let pk_impl = if args.namespaced {
            quote! {
                impl super::ProvideKey for #status_struct_name {
                    fn key(
                        tenant: String,
                        metadata: super::metadata::Metadata,
                    ) -> crate::machinery::store::Key<#status_struct_name> {
                        crate::machinery::store::Key::<#status_struct_name>::namespaced()
                            .collection(#status_collection_name)
                            .tenant(tenant)
                            .namespace(metadata.namespace)
                            .key(metadata.name)
                            .as_ref()
                            .into()
                    }

                    fn partial_key(
                        tenant: String,
                        namespace: Option<String>,
                    ) -> crate::machinery::store::PartialKey<#status_struct_name> {
                        let builder = crate::machinery::store::PartialKey::<#status_struct_name>::namespaced()
                            .collection(#status_collection_name)
                            .tenant(tenant);

                        if let Some(namespace) = namespace {
                            return builder.namespace(namespace).as_ref().into();
                        }

                        builder.as_ref().into()
                    }
                }
            }
        } else {
            quote! {
                impl super::ProvideKey for #status_struct_name {
                    fn key(
                        tenant: String,
                        metadata: super::metadata::Metadata,
                    ) -> crate::machinery::store::Key<#status_struct_name> {
                        crate::machinery::store::Key::<#status_struct_name>::not_namespaced()
                            .collection(#status_collection_name)
                            .tenant(tenant)
                            .key(metadata.name)
                            .as_ref()
                            .into()
                    }

                    fn partial_key(
                        tenant: String,
                        _namespace: Option<String>,
                    ) -> crate::machinery::store::PartialKey<#status_struct_name> {
                        crate::machinery::store::PartialKey::<#status_struct_name>::not_namespaced()
                            .collection(#status_collection_name)
                            .tenant(tenant)
                            .as_ref()
                            .into()
                    }
                }
            }
        };

        provide_key_impl_status = Some(pk_impl);
    }

    let enum_name_str = enum_name.to_string();
    let namespaced = args.namespaced;
    let crate_path_str = format!("crate::resources::{}", collection_name);

    let mut versions_build_info = Vec::new();

    for version_variant in &version_enum_variants {
        let variant_name = version_variant.ident.to_string();
        let struct_name = format!("{}{}", args.name, variant_name);

        versions_build_info.push(quote! {
            super::VersionBuildInfo {
                variant_name: #variant_name,
                struct_name: #struct_name,
            }
        });
    }

    let status_build_info = if let Some(status_struct) = &status_struct {
        let status_struct_name = status_struct.ident.to_string();
        let status_collection_name = format!("status-{}", collection_name);

        quote! {
            Some(super::StatusBuildInfo {
                struct_name: #status_struct_name,
                collection: #status_collection_name,
            })
        }
    } else {
        quote! {
            None
        }
    };

    let build_info_impl = quote! {
        impl super::BuildableResource for #enum_name {
            fn build_info() -> super::ResourceBuildInfo {
                super::ResourceBuildInfo {
                    name: #enum_name_str,
                    tag: #collection_name,
                    namespaced: #namespaced,
                    collection: #collection_name,
                    crate_path: #crate_path_str,
                    versions: vec![#(#versions_build_info),*],
                    status: #status_build_info,
                }
            }
        }
    };

    let output = quote::quote! {
        #(#version_structs)*

        #(#version_provide_metadata_impls)*

        #[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
        pub enum #enum_name {
            #(#version_enum_variants)*
        }

        #provide_key_impl

        #provide_metadata_impl

        #status_struct

        #provide_key_impl_status

        #build_info_impl
    };

    output.into()
}
