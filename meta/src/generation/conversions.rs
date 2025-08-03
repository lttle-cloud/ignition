use proc_macro2::Span;
use quote::quote;
use crate::types::ResourceAnalysis;

pub fn generate_type_aliases(analysis: &ResourceAnalysis) -> proc_macro2::TokenStream {
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

pub fn generate_conversion_methods(analysis: &ResourceAnalysis) -> proc_macro2::TokenStream {
    let enum_name = syn::Ident::new(&analysis.args.name, Span::call_site());
    let stored_ident = syn::Ident::new(&format!("{}Stored", analysis.args.name), Span::call_site());
    let latest_ident = syn::Ident::new(&format!("{}Latest", analysis.args.name), Span::call_site());
    
    // Find stored and latest versions
    let stored_version = analysis.versions.iter().find(|v| v.stored);
    let latest_version = analysis.versions.iter().find(|v| v.latest);
    
    let Some(_) = stored_version else {
        return quote! {};
    };

    let Some(_) = latest_version else {
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