use crate::types::{ResourceAnalysis, ResourceArgs, VersionInfo};
use proc_macro2::Span;
use quote::quote;

pub fn generate_provide_metadata_impls(
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
                            super::metadata::Metadata::new(self.name.clone(), crate::resources::metadata::Namespace::from_value_or_default(self.namespace.clone()))
                        }
                    }
                }
            } else {
                quote! {
                    impl super::ProvideMetadata for #variant_value {
                        fn metadata(&self) -> super::metadata::Metadata {
                            super::metadata::Metadata::new(self.name.clone(), crate::resources::metadata::Namespace::Unspecified)
                        }
                    }
                }
            }
        })
        .collect()
}

pub fn generate_provide_key_impl(analysis: &ResourceAnalysis) -> proc_macro2::TokenStream {
    let enum_name = syn::Ident::new(&analysis.args.name, Span::call_site());
    let collection_name = analysis.args.tag.clone();

    if analysis.args.namespaced {
        quote! {
            impl super::ProvideKey for #enum_name {
                fn key(
                    tenant: String,
                    metadata: super::metadata::Metadata,
                ) -> anyhow::Result<crate::machinery::store::Key<#enum_name>> {
                    let Some(namespace) = metadata.namespace else {
                        return Err(anyhow::anyhow!("namespace is required"));
                    };

                    Ok(
                        crate::machinery::store::Key::<#enum_name>::namespaced()
                            .collection(#collection_name)
                            .tenant(tenant)
                            .namespace(namespace)
                            .key(metadata.name)
                            .as_ref()
                            .into()
                    )

                }

                fn partial_key(
                    tenant: String,
                    namespace: super::metadata::Namespace,
                ) -> anyhow::Result<crate::machinery::store::PartialKey<#enum_name>> {
                    let builder = crate::machinery::store::PartialKey::<#enum_name>::namespaced()
                        .collection(#collection_name)
                        .tenant(tenant);

                    if let Some(namespace) = namespace.as_value() {
                        return Ok(builder.namespace(namespace).as_ref().into());
                    }

                    Ok(builder.as_ref().into())
                }
            }
        }
    } else {
        quote! {
            impl super::ProvideKey for #enum_name {
                fn key(
                    tenant: String,
                    metadata: super::metadata::Metadata,
                ) -> anyhow::Result<crate::machinery::store::Key<#enum_name>> {
                    crate::machinery::store::Key::<#enum_name>::not_namespaced()
                        .collection(#collection_name)
                        .tenant(tenant)
                        .key(metadata.name)
                        .as_ref()
                        .into()
                }

                fn partial_key(
                    tenant: String,
                    _namespace: super::metadata::Namespace,
                ) -> anyhow::Result<crate::machinery::store::PartialKey<#enum_name>> {
                    Ok(crate::machinery::store::PartialKey::<#enum_name>::not_namespaced()
                        .collection(#collection_name)
                        .tenant(tenant)
                        .as_ref()
                        .into())
                }
            }
        }
    }
}

pub fn generate_provide_metadata_impl(analysis: &ResourceAnalysis) -> proc_macro2::TokenStream {
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

pub fn generate_status_provide_key_impl(
    analysis: &ResourceAnalysis,
) -> Option<proc_macro2::TokenStream> {
    let status_struct_name = analysis.status.generated_ident.clone();
    let status_collection_name = format!("status-{}", analysis.args.tag);

    let pk_impl = if analysis.args.namespaced {
        quote! {
            impl super::ProvideKey for #status_struct_name {
                fn key(
                    tenant: String,
                    metadata: super::metadata::Metadata,
                ) -> anyhow::Result<crate::machinery::store::Key<#status_struct_name>> {
                    let Some(namespace) = metadata.namespace else {
                        return Err(anyhow::anyhow!("namespace is required"));
                    };

                    Ok(
                        crate::machinery::store::Key::<#status_struct_name>::namespaced()
                            .collection(#status_collection_name)
                            .tenant(tenant)
                            .namespace(namespace)
                            .key(metadata.name)
                            .as_ref()
                            .into()
                    )
                }

                fn partial_key(
                    tenant: String,
                    namespace: super::metadata::Namespace,
                ) -> anyhow::Result<crate::machinery::store::PartialKey<#status_struct_name>> {
                    let builder = crate::machinery::store::PartialKey::<#status_struct_name>::namespaced()
                        .collection(#status_collection_name)
                        .tenant(tenant);

                    if let Some(namespace) = namespace.as_value() {
                        return Ok(builder.namespace(namespace).as_ref().into());
                    }
                    Ok(builder.as_ref().into())
                }
            }
        }
    } else {
        quote! {
            impl super::ProvideKey for #status_struct_name {
                fn key(
                    tenant: String,
                    metadata: super::metadata::Metadata,
                ) -> anyhow::Result<crate::machinery::store::Key<#status_struct_name>> {
                    Ok(crate::machinery::store::Key::<#status_struct_name>::not_namespaced()
                        .collection(#status_collection_name)
                        .tenant(tenant)
                        .key(metadata.name)
                        .as_ref()
                        .into()
                    )
                }

                fn partial_key(
                    tenant: String,
                    _namespace: super::metadata::Namespace,
                ) -> anyhow::Result<crate::machinery::store::PartialKey<#status_struct_name>> {
                    Ok(crate::machinery::store::PartialKey::<#status_struct_name>::not_namespaced()
                        .collection(#status_collection_name)
                        .tenant(tenant)
                        .as_ref()
                        .into()
                    )
                }
            }
        }
    };

    Some(pk_impl)
}

pub fn generate_build_info_impl(analysis: &ResourceAnalysis) -> proc_macro2::TokenStream {
    let enum_name = syn::Ident::new(&analysis.args.name, Span::call_site());
    let enum_name_str = enum_name.to_string();
    let schema_provider = syn::Ident::new(&format!("{}Schema", enum_name), Span::call_site());
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

    let status_build_info = {
        let status_struct_name = analysis.status.generated_ident.to_string();
        let status_collection_name = format!("status-{}", collection_name);

        quote! {
            super::StatusBuildInfo {
                struct_name: #status_struct_name,
                collection: #status_collection_name,
            }
        }
    };

    let status_schema_provider = {
        let status_struct_name = analysis.status.generated_ident.clone();
        quote! {
            #status_struct_name
        }
    };

    quote! {
        impl super::BuildableResource for #enum_name {
            type SchemaProvider = #schema_provider;
            type StatusSchemaProvider = #status_schema_provider;

            fn build_info(configuration: super::ResourceConfiguration, schema: schemars::Schema, status_schema: schemars::Schema) -> super::ResourceBuildInfo {
                super::ResourceBuildInfo {
                    name: #enum_name_str,
                    tag: #collection_name,
                    namespaced: #namespaced,
                    collection: #collection_name,
                    crate_path: #crate_path_str,
                    versions: vec![#(#versions_build_info),*],
                    status: #status_build_info,
                    configuration,
                    schema,
                    status_schema,
                }
            }
        }
    }
}
