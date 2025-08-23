#![allow(dead_code)]

use anyhow::Result;
use schemars::{JsonSchema, Schema};
use serde::{Deserialize, Deserializer, Serialize, de::DeserializeOwned};

use crate::{
    machinery::store::{Key, PartialKey},
    resources::metadata::{Metadata, Namespace},
};

pub mod certificate;
pub mod core;
pub mod machine;
pub mod metadata;
pub mod service;
pub mod volume;

pub trait ConvertResource<T> {
    fn convert_up(this: Self) -> T;
    fn convert_down(this: T) -> Self;
}

pub trait Convert<TLatest, TStored> {
    fn latest(&self) -> TLatest;
    fn stored(&self) -> TStored;
}

impl<TLatest, TStored, T> Convert<Vec<TLatest>, Vec<TStored>> for Vec<T>
where
    T: Convert<TLatest, TStored>,
{
    fn latest(&self) -> Vec<TLatest> {
        self.iter().map(|x| x.latest()).collect()
    }

    fn stored(&self) -> Vec<TStored> {
        self.iter().map(|x| x.stored()).collect()
    }
}

pub trait ProvideMetadata {
    fn metadata(&self) -> Metadata;
}

pub trait ProvideKey
where
    Self: Serialize + DeserializeOwned,
{
    fn key(tenant: String, metadata: Metadata) -> Result<Key<Self>>;
    fn partial_key(tenant: String, namespace: Namespace) -> Result<PartialKey<Self>>;
}

#[derive(Debug, Clone)]
pub struct VersionBuildInfo {
    pub variant_name: &'static str,
    pub struct_name: &'static str,
    pub stored: bool,
    pub served: bool,
    pub latest: bool,
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
    pub status: StatusBuildInfo,
    pub configuration: ResourceConfiguration,
    pub schema: Schema,
    pub status_schema: Schema,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AdmissionRule {
    /// Custom admission status check
    StatusCheck,
    /// Before create/patch
    BeforeSet,
    /// Custom before delete check
    BeforeDelete,
}

#[derive(Debug, Clone)]
pub struct ResourceConfiguration {
    pub generate_service: bool,
    pub generate_service_get: bool,
    pub generate_service_list: bool,
    pub generate_service_set: bool,
    pub generate_service_delete: bool,
    pub generate_service_get_status: bool,
    pub admission_rules: Vec<AdmissionRule>,
}

impl ResourceConfiguration {
    pub fn new() -> Self {
        Self {
            generate_service: true,
            generate_service_get: true,
            generate_service_list: true,
            generate_service_set: true,
            generate_service_delete: true,
            generate_service_get_status: true,
            admission_rules: vec![],
        }
    }

    pub fn disable_generate_service(mut self) -> Self {
        self.generate_service = false;
        self
    }

    pub fn disable_generate_service_get(mut self) -> Self {
        self.generate_service_get = false;
        self
    }

    pub fn disable_generate_service_list(mut self) -> Self {
        self.generate_service_list = false;
        self
    }

    pub fn disable_generate_service_set(mut self) -> Self {
        self.generate_service_set = false;
        self
    }

    pub fn disable_generate_service_delete(mut self) -> Self {
        self.generate_service_delete = false;
        self
    }

    pub fn disable_generate_service_get_status(mut self) -> Self {
        self.generate_service_get_status = false;
        self
    }

    pub fn add_admission_rule(mut self, rule: AdmissionRule) -> Self {
        self.admission_rules.push(rule);
        self
    }
}

pub trait BuildableResource {
    type SchemaProvider: JsonSchema;
    type StatusSchemaProvider: JsonSchema;

    fn build_info(
        configuration: ResourceConfiguration,
        schema: Schema,
        status_schema: Schema,
    ) -> ResourceBuildInfo;
}

pub trait FromResource<T> {
    fn from_resource(resource: T) -> Result<Self>
    where
        Self: Sized;
}

pub fn de_trim_non_empty_string<'de, D>(deserializer: D) -> std::result::Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    let s = s.trim().to_string();
    if s.is_empty() {
        return Err(serde::de::Error::custom("string cannot be empty"));
    }
    Ok(s)
}

pub fn de_opt_trim_non_empty_string<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    println!("de_opt_trim_non_empty_string");
    let s = Option::<String>::deserialize(deserializer)?;
    if let Some(s) = s {
        let s = s.trim().to_string();
        if s.is_empty() {
            return Err(serde::de::Error::custom("string cannot be empty"));
        }
        Ok(Some(s))
    } else {
        Ok(None)
    }
}

pub fn de_vec_trim_non_empty_string<'de, D>(
    deserializer: D,
) -> std::result::Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = Vec::<String>::deserialize(deserializer)?;
    let s = s.iter().map(|x| x.trim().to_string()).collect();
    Ok(s)
}

pub trait AdmissionCheckStatus<TStatus>
where
    Self: Sized,
{
    fn admission_check_status(&self, status: &TStatus) -> Result<()>;
}
