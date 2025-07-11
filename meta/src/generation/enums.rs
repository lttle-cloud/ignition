use crate::types::{ResourceArgs, VersionInfo};
use syn::Variant;

pub fn generate_version_enum_variants(
    args: &ResourceArgs,
    versions: &[VersionInfo],
) -> Vec<Variant> {
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

pub fn generate_schema_enum_variants(
    args: &ResourceArgs,
    versions: &[VersionInfo],
) -> Vec<Variant> {
    let mut variants = versions
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
        .collect::<Vec<_>>();

    let rename = args.tag.clone();
    let variant_value = syn::Ident::new(
        &format!("{}Latest", args.name),
        proc_macro2::Span::call_site(),
    );

    variants.insert(
        0,
        syn::parse_quote!(
            #[serde(rename = #rename)]
            Latest (#variant_value)
        ),
    );

    variants
}
