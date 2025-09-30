use std::str::FromStr;

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

#[derive(Debug, Clone)]
pub struct AdditionalSchemaInfo {
    pub item: syn::Item,
}

#[derive(Debug)]
pub struct ResourceAnalysis {
    pub args: ResourceArgs,
    pub versions: Vec<VersionInfo>,
    pub status: StatusInfo,
    pub additional_schemas: Vec<AdditionalSchemaInfo>,
}

// #[name = "name", max_width? = 10, cell_style? = important | default]
pub struct TableFieldArgs {
    pub name: String,
    pub max_width: Option<usize>,
    pub min_width: Option<usize>,
    pub cell_style: Option<TableCellStyle>,
}

#[derive(Debug, Clone)]
pub enum TableCellStyle {
    Default,
    Important,
}

impl FromStr for TableCellStyle {
    type Err = syn::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "default" => Ok(TableCellStyle::Default),
            "important" => Ok(TableCellStyle::Important),
            _ => Err(syn::Error::new(
                Span::call_site(),
                "invalid table cell style",
            )),
        }
    }
}

impl Parse for TableCellStyle {
    fn parse(input: ParseStream) -> Result<Self> {
        let ident = input.parse::<syn::Ident>()?;
        match ident.to_string().as_str() {
            "default" => Ok(TableCellStyle::Default),
            "important" => Ok(TableCellStyle::Important),
            _ => Err(syn::Error::new(ident.span(), "invalid table cell style")),
        }
    }
}

impl Parse for TableFieldArgs {
    fn parse(input: ParseStream) -> Result<Self> {
        let list = input.parse_terminated(syn::MetaNameValue::parse, Token![,])?;

        let mut name = None;
        let mut max_width = None;
        let mut min_width = None;
        let mut cell_style = None;

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
            } else if item.path.is_ident("max_width") {
                let syn::Expr::Lit(syn::ExprLit {
                    attrs: _,
                    lit: syn::Lit::Int(lit),
                    ..
                }) = item.value
                else {
                    return Err(syn::Error::new(
                        item.value.span(),
                        "max_width must be an integer",
                    ));
                };

                max_width = Some(lit.base10_parse::<usize>()?);
            } else if item.path.is_ident("min_width") {
                let syn::Expr::Lit(syn::ExprLit {
                    attrs: _,
                    lit: syn::Lit::Int(lit),
                    ..
                }) = item.value
                else {
                    return Err(syn::Error::new(
                        item.value.span(),
                        "min_width must be an integer",
                    ));
                };

                min_width = Some(lit.base10_parse::<usize>()?);
            } else if item.path.is_ident("cell_style") {
                let syn::Expr::Path(syn::ExprPath {
                    attrs: _,
                    qself: _,
                    path: syn::Path { segments, .. },
                }) = item.value
                else {
                    return Err(syn::Error::new(
                        item.value.span(),
                        "cell_style must be a identifier",
                    ));
                };
                if segments.len() != 1 {
                    return Err(syn::Error::new(
                        item.path.span(),
                        "cell_style must be a single identifier",
                    ));
                }

                let Some(last_segment) = segments.last() else {
                    return Err(syn::Error::new(
                        item.path.span(),
                        "cell_style must be a single identifier",
                    ));
                };

                cell_style = Some(last_segment.ident.to_string().parse::<TableCellStyle>()?);
            }
        }

        let name = name.ok_or(syn::Error::new(Span::call_site(), "name is required"))?;

        Ok(TableFieldArgs {
            name,
            max_width,
            min_width,
            cell_style,
        })
    }
}

// #[name = "name", cell_style? = important | default]
pub struct SummaryFieldArgs {
    pub name: String,
    pub cell_style: Option<SummaryCellStyle>,
    pub clip_value: bool,
}

#[derive(Debug, Clone)]
pub enum SummaryCellStyle {
    Default,
    Important,
}

impl FromStr for SummaryCellStyle {
    type Err = syn::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "default" => Ok(SummaryCellStyle::Default),
            "important" => Ok(SummaryCellStyle::Important),
            _ => Err(syn::Error::new(
                Span::call_site(),
                "invalid summary cell style",
            )),
        }
    }
}

impl Parse for SummaryCellStyle {
    fn parse(input: ParseStream) -> Result<Self> {
        let ident = input.parse::<syn::Ident>()?;
        match ident.to_string().as_str() {
            "default" => Ok(SummaryCellStyle::Default),
            "important" => Ok(SummaryCellStyle::Important),
            _ => Err(syn::Error::new(ident.span(), "invalid summary cell style")),
        }
    }
}

impl Parse for SummaryFieldArgs {
    fn parse(input: ParseStream) -> Result<Self> {
        let list = input.parse_terminated(syn::MetaNameValue::parse, Token![,])?;

        let mut name = None;
        let mut cell_style = None;
        let mut clip_value = None;

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
            } else if item.path.is_ident("cell_style") {
                let syn::Expr::Path(syn::ExprPath {
                    attrs: _,
                    qself: _,
                    path: syn::Path { segments, .. },
                }) = item.value
                else {
                    return Err(syn::Error::new(
                        item.value.span(),
                        "cell_style must be a identifier",
                    ));
                };
                if segments.len() != 1 {
                    return Err(syn::Error::new(
                        item.path.span(),
                        "cell_style must be a single identifier",
                    ));
                }

                let Some(last_segment) = segments.last() else {
                    return Err(syn::Error::new(
                        item.path.span(),
                        "cell_style must be a single identifier",
                    ));
                };

                cell_style = Some(last_segment.ident.to_string().parse::<SummaryCellStyle>()?);
            } else if item.path.is_ident("clip_value") {
                let syn::Expr::Lit(syn::ExprLit {
                    attrs: _,
                    lit: syn::Lit::Bool(lit),
                    ..
                }) = item.value
                else {
                    return Err(syn::Error::new(
                        item.value.span(),
                        "clip_value must be a boolean",
                    ));
                };
                clip_value = Some(lit.value());
            }
        }

        let name = name.ok_or(syn::Error::new(Span::call_site(), "name is required"))?;
        let clip_value = clip_value.unwrap_or(true);

        Ok(SummaryFieldArgs {
            name,
            cell_style,
            clip_value,
        })
    }
}
