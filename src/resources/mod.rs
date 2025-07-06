use serde::{Serialize, de::DeserializeOwned};

use crate::{
    machinery::store::{Key, PartialKey},
    resources::metadata::Metadata,
};

pub mod machine;
pub mod metadata;

pub trait ConvertResource<T> {
    fn up(this: Self) -> T;
    fn down(other: T) -> Self;
}

pub trait ProvideMetadata {
    fn metadata(&self) -> Metadata;
}

pub trait ProvideKey
where
    Self: Serialize + DeserializeOwned,
{
    fn key(tenant: String, metadata: Metadata) -> Key<Self>;
    fn partial_key(tenant: String, namespace: Option<String>) -> PartialKey<Self>;
}

#[derive(Debug, Clone)]
pub struct VersionBuildInfo {
    pub variant_name: &'static str,
    pub struct_name: &'static str,
}

#[derive(Debug, Clone)]
pub struct StatusBuildInfo {
    pub struct_name: &'static str,
    pub collection: &'static str,
}

#[derive(Debug, Clone)]
pub struct ResourceBuildInfo {
    pub name: &'static str,
    pub tag: &'static str,
    pub namespaced: bool,
    pub collection: &'static str,
    pub crate_path: &'static str,
    pub versions: Vec<VersionBuildInfo>,
    pub status: Option<StatusBuildInfo>,
}

pub trait BuildableResource {
    fn build_info() -> ResourceBuildInfo;
}
