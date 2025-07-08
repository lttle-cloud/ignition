use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::{
    Fields, FieldsNamed, Ident, MetaNameValue, Result, Token, Variant,
    parse::{Parse, ParseStream},
    parse_macro_input,
    punctuated::Punctuated,
    spanned::Spanned,
};

#[derive(Debug, Clone, PartialEq, Eq)]
enum VersionTag {
    Stored,
    Served,
    Latest,
}

impl Parse for VersionTag {
    fn parse(input: ParseStream) -> Result<Self> {
        let ident = input.parse::<Ident>()?;
        match ident.to_string().as_str() {
            "stored" => Ok(VersionTag::Stored),
            "served" => Ok(VersionTag::Served),
            "latest" => Ok(VersionTag::Latest),
            _ => Err(syn::Error::new(
                ident.span(),
                format!("invalid version tag {}", ident),
            )),
        }
    }
}

struct TagsList<T: Parse + Clone>(pub Punctuated<T, Token![+]>);

impl<T: Parse + Clone> Parse for TagsList<T> {
    fn parse(input: ParseStream) -> Result<Self> {
        let list = input.parse_terminated(T::parse, Token![+])?;
        Ok(TagsList(list))
    }
}

impl<T: Parse + Clone> TagsList<T> {
    pub fn tags(&self) -> Vec<T> {
        self.0.iter().cloned().collect()
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

#[derive(Debug, Clone)]
struct VersionInfo {
    pub original_ident: Ident,
    pub generated_ident: Ident,
    pub stored: bool,
    pub served: bool,
    pub latest: bool,
}

#[derive(Debug, Clone)]
struct StatusInfo {
    pub original_ident: Ident,
    pub generated_ident: Ident,
}

#[derive(Debug)]
struct ResourceAnalysis {
    pub args: ResourceArgs,
    pub versions: Vec<VersionInfo>,
    pub status: Option<StatusInfo>,
}

fn extract_version_info(struct_item: &syn::ItemStruct, args: &ResourceArgs) -> Option<VersionInfo> {
    for attr in struct_item.attrs.iter() {
        if attr.path().is_ident("version") {
            let tags = if let syn::Meta::List(list) = &attr.meta {
                list.parse_args::<TagsList<VersionTag>>()
                    .expect("failed to parse version tags")
                    .tags()
            } else {
                Vec::new()
            };

            let original_ident = struct_item.ident.clone();
            let generated_ident = syn::Ident::new(
                &format!("{}{}", args.name, original_ident),
                original_ident.span(),
            );

            return Some(VersionInfo {
                original_ident,
                generated_ident,
                stored: tags.contains(&VersionTag::Stored),
                served: tags.contains(&VersionTag::Served),
                latest: tags.contains(&VersionTag::Latest),
            });
        }
    }
    None
}

fn extract_status_info(struct_item: &syn::ItemStruct, args: &ResourceArgs) -> Option<StatusInfo> {
    for attr in struct_item.attrs.iter() {
        if attr.path().is_ident("status") {
            let original_ident = struct_item.ident.clone();
            let generated_ident = syn::Ident::new(
                &format!("{}{}", args.name, original_ident),
                original_ident.span(),
            );

            return Some(StatusInfo {
                original_ident,
                generated_ident,
            });
        }
    }
    None
}

fn analyze_resource(args: ResourceArgs, resource_mod: &syn::ItemMod) -> ResourceAnalysis {
    let mut versions = Vec::new();
    let mut status = None;

    let (_, items) = resource_mod
        .content
        .clone()
        .expect("resource module must have content");

    for item in items {
        if let syn::Item::Struct(struct_item) = item {
            if let Some(version_info) = extract_version_info(&struct_item, &args) {
                versions.push(version_info);
            } else if let Some(status_info) = extract_status_info(&struct_item, &args) {
                if status.is_some() {
                    panic!("status struct already defined");
                }
                status = Some(status_info);
            }
        }
    }

    ResourceAnalysis {
        args,
        versions,
        status,
    }
}

fn validate_versions(analysis: &ResourceAnalysis) -> Result<()> {
    let stored_count = analysis.versions.iter().filter(|v| v.stored).count();
    let served_count = analysis.versions.iter().filter(|v| v.served).count();
    let latest_count = analysis.versions.iter().filter(|v| v.latest).count();

    if stored_count != 1 {
        return Err(syn::Error::new(
            Span::call_site(),
            format!(
                "resource must have exactly one stored version, found {}",
                stored_count
            ),
        ));
    }

    if served_count == 0 {
        return Err(syn::Error::new(
            Span::call_site(),
            "resource must have at least one served version",
        ));
    }

    if latest_count != 1 {
        return Err(syn::Error::new(
            Span::call_site(),
            "resource must have exactly one latest version",
        ));
    }

    Ok(())
}

fn generate_version_struct(
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
    item.ident = version_info.generated_ident.clone();

    item
}

fn generate_status_struct(
    struct_item: &syn::ItemStruct,
    status_info: &StatusInfo,
) -> syn::ItemStruct {
    let mut item = struct_item.clone();
    item.attrs = syn::parse_quote!(#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]);
    item.fields.iter_mut().for_each(|field| {
        field.vis = syn::Visibility::Public(syn::token::Pub { span: field.span() });
    });
    item.ident = status_info.generated_ident.clone();
    item.vis = syn::Visibility::Public(syn::token::Pub {
        span: item.ident.span(),
    });
    item
}

fn generate_version_enum_variants(args: &ResourceArgs, versions: &[VersionInfo]) -> Vec<Variant> {
    versions
        .iter()
        .map(|version| {
            let variant_name = version.original_ident.clone();
            let variant_value = version.generated_ident.clone();
            let rename = format!("{}.{}", args.tag, version.original_ident).to_lowercase();
            syn::parse_quote!(
                #[serde(rename = #rename)]
                #variant_name (#variant_value)
            )
        })
        .collect()
}

fn generate_provide_metadata_impls(
    versions: &[VersionInfo],
    args: &ResourceArgs,
) -> Vec<proc_macro2::TokenStream> {
    versions
        .iter()
        .map(|version| {
            let variant_value = version.generated_ident.clone();
            if args.namespaced {
                quote! {
                    impl super::ProvideMetadata for #variant_value {
                        fn metadata(&self) -> super::metadata::Metadata {
                            super::metadata::Metadata::new(self.name.clone(), self.namespace.clone())
                        }
                    }
                }
            } else {
                quote! {
                    impl super::ProvideMetadata for #variant_value {
                        fn metadata(&self) -> super::metadata::Metadata {
                            super::metadata::Metadata::new(self.name.clone(), None)
                        }
                    }
                }
            }
        })
        .collect()
}

fn generate_provide_key_impl(analysis: &ResourceAnalysis) -> proc_macro2::TokenStream {
    let enum_name = syn::Ident::new(&analysis.args.name, Span::call_site());
    let collection_name = analysis.args.tag.clone();

    if analysis.args.namespaced {
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
    }
}

fn generate_provide_metadata_impl(analysis: &ResourceAnalysis) -> proc_macro2::TokenStream {
    let enum_name = syn::Ident::new(&analysis.args.name, Span::call_site());

    let version_enum_variants_with_metadata = analysis.versions.iter().map(|version| {
        let variant_name = version.original_ident.clone();
        quote! {
            #enum_name::#variant_name (item) => item.metadata(),
        }
    });

    quote! {
        impl super::ProvideMetadata for #enum_name {
            fn metadata(&self) -> super::metadata::Metadata {
                match self {
                    #(#version_enum_variants_with_metadata)*
                }
            }
        }
    }
}

fn generate_status_provide_key_impl(
    analysis: &ResourceAnalysis,
) -> Option<proc_macro2::TokenStream> {
    let status_info = analysis.status.as_ref()?;
    let status_struct_name = status_info.generated_ident.clone();
    let status_collection_name = format!("status-{}", analysis.args.tag);

    let pk_impl = if analysis.args.namespaced {
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

    Some(pk_impl)
}

fn generate_build_info_impl(analysis: &ResourceAnalysis) -> proc_macro2::TokenStream {
    let enum_name = syn::Ident::new(&analysis.args.name, Span::call_site());
    let enum_name_str = enum_name.to_string();
    let namespaced = analysis.args.namespaced;
    let collection_name = analysis.args.tag.clone();
    let crate_path_str = format!("resources::{}", collection_name);

    let versions_build_info = analysis.versions.iter().map(|version| {
        let variant_name = version.original_ident.to_string();
        let struct_name = version.generated_ident.to_string();
        let stored = version.stored;
        let served = version.served;
        let latest = version.latest;

        quote! {
            super::VersionBuildInfo {
                variant_name: #variant_name,
                struct_name: #struct_name,
                stored: #stored,
                served: #served,
                latest: #latest,
            }
        }
    });

    let status_build_info = if let Some(status_info) = &analysis.status {
        let status_struct_name = status_info.generated_ident.to_string();
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

    quote! {
        impl super::BuildableResource for #enum_name {
            fn build_info(configuration: super::ResourceConfiguration) -> super::ResourceBuildInfo {
                super::ResourceBuildInfo {
                    name: #enum_name_str,
                    tag: #collection_name,
                    namespaced: #namespaced,
                    collection: #collection_name,
                    crate_path: #crate_path_str,
                    versions: vec![#(#versions_build_info),*],
                    status: #status_build_info,
                    configuration,
                }
            }
        }
    }
}

fn generate_type_aliases(analysis: &ResourceAnalysis) -> proc_macro2::TokenStream {
    // Find stored version
    let stored_version = analysis.versions.iter().find(|v| v.stored);
    let stored_alias = if let Some(stored) = stored_version {
        let stored_type = stored.generated_ident.clone();
        let stored_ident =
            syn::Ident::new(&format!("{}Stored", analysis.args.name), Span::call_site());
        quote! {
            pub type #stored_ident = #stored_type;
        }
    } else {
        quote! {}
    };
    
    // Find latest version
    let latest_version = analysis.versions.iter().find(|v| v.latest);
    let latest_alias = if let Some(latest) = latest_version {
        let latest_type = latest.generated_ident.clone();
        let latest_ident =
            syn::Ident::new(&format!("{}Latest", analysis.args.name), Span::call_site());
        quote! {
            pub type #latest_ident = #latest_type;
        }
    } else {
        quote! {}
    };
    
    quote! {
        #stored_alias
        #latest_alias
    }
}

fn generate_conversion_methods(analysis: &ResourceAnalysis) -> proc_macro2::TokenStream {
    let enum_name = syn::Ident::new(&analysis.args.name, Span::call_site());
    let stored_ident = syn::Ident::new(&format!("{}Stored", analysis.args.name), Span::call_site());
    let latest_ident = syn::Ident::new(&format!("{}Latest", analysis.args.name), Span::call_site());
    
    // Find stored and latest versions
    let stored_version = analysis.versions.iter().find(|v| v.stored);
    let latest_version = analysis.versions.iter().find(|v| v.latest);
    
    let Some(stored_version) = stored_version else {
        return quote! {};
    };

    let Some(latest_version) = latest_version else {
        return quote! {};
    };

    let stored_version_index = analysis.versions.iter().position(|v| v.stored).unwrap();
    let latest_version_index = analysis.versions.iter().position(|v| v.latest).unwrap();
    
    let latest_match_arms = analysis.versions.iter().enumerate().map(|(i, version)| {        
        let variant_name = version.original_ident.clone();
        let variant_value = version.generated_ident.clone();

        if version.latest {            
            quote! {
                #enum_name::#variant_name(v) => v.clone(),
            }
        } else if i < latest_version_index {
            let next_variant_name = analysis.versions[i + 1].original_ident.clone();
            quote! {
                #enum_name::#variant_name(v) => {
                    #enum_name::#next_variant_name(#variant_value::convert_up(v.clone())).latest()
                },
            }
        } else if i > stored_version_index {
            let previous_variant_name = analysis.versions[i - 1].original_ident.clone();
            let previous_variant_value = analysis.versions[i - 1].generated_ident.clone();

            quote! {
                #enum_name::#variant_name(v) => {
                    #enum_name::#previous_variant_name(#previous_variant_value::convert_down(v.clone())).latest()
                },
            }
        } else {
            quote! {}
        }
    });
    
    let stored_match_arms = analysis.versions.iter().enumerate().map(|(i, version)| {
        let variant_name = version.original_ident.clone();
        let variant_value = version.generated_ident.clone();
        
        if version.stored {
            // This is the stored version, just clone it
            quote! {
                #enum_name::#variant_name(v) => v.clone(),
            }
        } else if i < stored_version_index {
            let next_variant_name = analysis.versions[i + 1].original_ident.clone();
            quote! {
                #enum_name::#variant_name(v) => {
                    #enum_name::#next_variant_name(#variant_value::convert_up(v.clone())).stored()
                },
            }
        } else if i > stored_version_index {
            let previous_variant_name = analysis.versions[i - 1].original_ident.clone();
            let previous_variant_value = analysis.versions[i - 1].generated_ident.clone();

            quote! {
                #enum_name::#variant_name(v) => {
                    #enum_name::#previous_variant_name(#previous_variant_value::convert_down(v.clone())).stored()
                },
            }
        } else {
            quote! {}
        }
    });

    let mut from_impls = vec![];
    for version in analysis.versions.iter() {
        let variant_name = version.original_ident.clone();
        let variant_value = version.generated_ident.clone();
        let from_impl = quote! {
            impl From<#variant_value> for #enum_name {
                fn from(value: #variant_value) -> Self {
                    #enum_name::#variant_name(value)
                }
            }
        };
        from_impls.push(from_impl);
    };


    quote! {
        impl super::Convert<#latest_ident, #stored_ident> for #enum_name {
            fn latest(&self) -> #latest_ident {
                use super::ConvertResource;
                match self {
                    #(#latest_match_arms)*
                }
            }

            fn stored(&self) -> #stored_ident {
                use super::ConvertResource;
                match self {
                    #(#stored_match_arms)*
                }
            }
        }

        #(#from_impls)*
    }

}

fn generate_output(
    analysis: ResourceAnalysis,
    resource_mod: &syn::ItemMod,
) -> proc_macro2::TokenStream {
    let enum_name = syn::Ident::new(&analysis.args.name, Span::call_site());

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
    let status_struct = if let Some(status_info) = &analysis.status {
        let (_, items) = resource_mod
            .content
            .clone()
            .expect("resource module must have content");

        let struct_item = items
            .iter()
            .find_map(|item| {
                if let syn::Item::Struct(s) = item {
                    if s.ident == status_info.original_ident {
                        Some(s)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .unwrap();

        Some(generate_status_struct(struct_item, status_info))
    } else {
        None
    };

    let version_enum_variants = generate_version_enum_variants(&analysis.args , &analysis.versions);
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

        #[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
        pub enum #enum_name {
            #(#version_enum_variants),*
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


#[proc_macro]
pub fn include_build_mod(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as syn::LitStr);

    let input_file = format!("/{}.rs", input.value());

    let output = quote::quote! {
        include!(concat!(env!("OUT_DIR"), #input_file));
    };

    output.into()
}