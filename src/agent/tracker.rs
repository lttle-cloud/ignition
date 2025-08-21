use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::{
    agent::data::Collections,
    constants::DEFAULT_AGENT_TENANT,
    machinery::store::{Key, Store},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum TrackedResourceKind {
    ServiceDomain(String),
    CertificateDomain(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct TrackedResourceOwner {
    pub kind: TrackedResourceKind,
    pub tenant: String,
    pub resource_name: String,
    pub resource_namespace: String,
}

impl TrackedResourceKind {
    pub fn key(&self) -> Key<TrackedResourceOwner> {
        match self {
            TrackedResourceKind::ServiceDomain(domain) => {
                Key::<TrackedResourceOwner>::not_namespaced()
                    .tenant(DEFAULT_AGENT_TENANT)
                    .collection(Collections::TrackedResourceOwner)
                    .key(format!("service_domain:{}", domain))
                    .as_ref()
                    .into()
            }
            TrackedResourceKind::CertificateDomain(domain) => {
                Key::<TrackedResourceOwner>::not_namespaced()
                    .tenant(DEFAULT_AGENT_TENANT)
                    .collection(Collections::TrackedResourceOwner)
                    .key(format!("certificate_domain:{}", domain))
                    .as_ref()
                    .into()
            }
        }
    }
}

pub struct TrackerAgent {
    pub store: Arc<Store>,
}

impl TrackerAgent {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }

    pub async fn track_resource_owner(&self, resource: TrackedResourceOwner) -> Result<()> {
        let key = resource.kind.key();
        self.store.put(key, resource)?;
        Ok(())
    }

    pub async fn untrack_resource_owner(&self, kind: TrackedResourceKind) -> Result<()> {
        let key = kind.key();
        self.store.delete(key)?;
        Ok(())
    }

    pub async fn get_tracked_resource_owner(
        &self,
        kind: TrackedResourceKind,
    ) -> Result<Option<TrackedResourceOwner>> {
        let key = kind.key();
        let resource = self.store.get(key)?;
        Ok(resource)
    }
}
