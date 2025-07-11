use crate::types::{ResourceAnalysis, ResourceArgs, StatusInfo, TagsList, VersionInfo, VersionTag};
use proc_macro2::Span;
use syn::Result;

pub fn extract_version_info(
    struct_item: &syn::ItemStruct,
    args: &ResourceArgs,
) -> Option<VersionInfo> {
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

pub fn extract_status_info(
    struct_item: &syn::ItemStruct,
    args: &ResourceArgs,
) -> Option<StatusInfo> {
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

pub fn analyze_resource(args: ResourceArgs, resource_mod: &syn::ItemMod) -> ResourceAnalysis {
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

pub fn validate_versions(analysis: &ResourceAnalysis) -> Result<()> {
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
