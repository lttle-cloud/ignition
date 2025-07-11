use proc_macro2::Span;
use syn::{
    Ident, Result, Token,
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    spanned::Spanned,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VersionTag {
    Stored,
    Served,
    Latest,
}

impl Parse for VersionTag {
    fn parse(input: ParseStream) -> Result<Self> {
        let ident = input.parse::<syn::Ident>()?;
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

pub struct TagsList<T: Parse + Clone>(pub Punctuated<T, Token![+]>);

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
pub struct ResourceArgs {
    pub name: String,
    pub tag: String,
    pub namespaced: bool,
}

impl Parse for ResourceArgs {
    fn parse(input: ParseStream) -> Result<Self> {
        let list = input.parse_terminated(syn::MetaNameValue::parse, Token![,])?;

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
pub struct VersionInfo {
    pub original_ident: Ident,
    pub generated_ident: Ident,
    pub stored: bool,
    pub served: bool,
    pub latest: bool,
}

#[derive(Debug, Clone)]
pub struct StatusInfo {
    pub original_ident: Ident,
    pub generated_ident: Ident,
}

#[derive(Debug)]
pub struct ResourceAnalysis {
    pub args: ResourceArgs,
    pub versions: Vec<VersionInfo>,
    pub status: Option<StatusInfo>,
}
