use std::{sync::Arc, time::Duration};

use crate::{
    agent::{Agent, machine::machine::MachineState},
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
            ResourceKind::Service => Metadata::new(
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
    pub agent: Arc<Agent>,
}

impl ControllerContext {
    pub fn new(
        tenant: impl AsRef<str>,
        store: Arc<Store>,
        repository: Arc<Repository>,
        agent: Arc<Agent>,
    ) -> Self {
        Self {
            tenant: tenant.as_ref().to_string(),
            store,
            repository,
            agent,
        }
    }
}

#[derive(Debug, Clone)]
pub enum AsyncWork {
    ImagePullComplete {
        id: String,
        reference: String,
    },
    MachineStateChange {
        machine_id: String,
        state: MachineState,
        first_boot_duration: Option<Duration>,
        last_boot_duration: Option<Duration>,
    },
    Error(String),
}

#[derive(Debug, Clone)]
pub enum ControllerEvent {
    BringUp(ResourceKind, Metadata),
    ResourceChange(ResourceKind, Metadata),
    ResourceStatusChange(ResourceKind, Metadata),
    AgentTrigger,
    AsyncWorkChange(ControllerKey, AsyncWork),
}
