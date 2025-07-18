use std::sync::Arc;

use oci_client::Reference;

use crate::{
    machinery::store::Store,
    repository::Repository,
    resource_index::ResourceKind,
    resources::metadata::{Metadata, Namespace},
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ControllerKey {
    pub tenant: String,
    pub kind: ResourceKind,
    pub namespace: Option<String>,
    pub name: String,
}

impl ControllerKey {
    pub fn new(
        tenant: impl AsRef<str>,
        kind: ResourceKind,
        namespace: Option<impl AsRef<str>>,
        name: impl AsRef<str>,
    ) -> Self {
        Self {
            tenant: tenant.as_ref().to_string(),
            kind,
            namespace: namespace.map(|ns| ns.as_ref().to_string()),
            name: name.as_ref().to_string(),
        }
    }

    pub fn metadata(&self) -> Metadata {
        match self.kind {
            ResourceKind::Machine => Metadata::new(
                self.name.clone(),
                Namespace::from_value_or_default(self.namespace.clone()),
            ),
        }
    }
}

impl ToString for ControllerKey {
    fn to_string(&self) -> String {
        if let Some(namespace) = &self.namespace {
            return format!(
                "{}.{:?}.{}.{}",
                self.tenant, self.kind, namespace, self.name
            );
        }

        format!("{}.{:?}.{}", self.tenant, self.kind, self.name)
    }
}

#[derive(Clone)]
pub struct ControllerContext {
    pub tenant: String,
    pub store: Arc<Store>,
    pub repository: Arc<Repository>,
}

impl ControllerContext {
    pub fn new(tenant: impl AsRef<str>, store: Arc<Store>, repository: Arc<Repository>) -> Self {
        Self {
            tenant: tenant.as_ref().to_string(),
            store,
            repository,
        }
    }
}

#[derive(Debug, Clone)]
pub enum AsyncWork {
    ImagePullUpdate {
        reference: Reference,
        layer_count: u64,
        downloaded_layers: u64,
        uncompressed_layers: u64,
    },
    ImagePullComplete {
        reference: Reference,
    },
}

#[derive(Debug, Clone)]
pub enum ControllerEvent {
    ResourceChange(ResourceKind, Metadata),
    ResourceStatusChange(ResourceKind, Metadata),
    AgentTrigger,
    AsyncWorkChange(ControllerKey, AsyncWork),
}
