use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::info;

use crate::{
    agent::{
        data::Collections,
        tracker::{TrackedResourceKind, TrackerAgent},
    },
    constants::DEFAULT_AGENT_TENANT,
    machinery::store::{Key, Store},
};

pub type TcpPortRange = (u16, u16);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct TcpPortAllocation {
    pub port: u16,
    pub tenant: String,
    pub resource_name: String,
    pub resource_namespace: String,
}

impl TcpPortAllocation {
    pub fn key(&self) -> Key<TcpPortAllocation> {
        Key::<TcpPortAllocation>::not_namespaced()
            .tenant(DEFAULT_AGENT_TENANT)
            .collection(Collections::TcpPortAllocation)
            .key(format!("tcp_port:{}", self.port))
            .as_ref()
            .into()
    }

    pub fn key_for_port(port: u16) -> Key<TcpPortAllocation> {
        Key::<TcpPortAllocation>::not_namespaced()
            .tenant(DEFAULT_AGENT_TENANT)
            .collection(Collections::TcpPortAllocation)
            .key(format!("tcp_port:{}", port))
            .as_ref()
            .into()
    }
}

pub struct PortAllocator {
    store: Arc<Store>,
    tracker: Arc<TrackerAgent>,
    tcp_port_range: Option<TcpPortRange>,
}

impl PortAllocator {
    pub fn new(
        store: Arc<Store>,
        tracker: Arc<TrackerAgent>,
        tcp_port_range: Option<TcpPortRange>,
    ) -> Self {
        Self {
            store,
            tracker,
            tcp_port_range,
        }
    }

    pub async fn allocate_tcp_port(
        &self,
        tenant: String,
        resource_name: String,
        resource_namespace: String,
    ) -> Result<u16> {
        let Some((start, end)) = &self.tcp_port_range else {
            bail!("TCP port range not configured");
        };

        info!(
            "Allocating TCP port for {}/{} in range {}..{}",
            resource_namespace, resource_name, start, end
        );

        // Generate random ports in the range and check availability
        use rand::Rng;

        // Try up to 100 times to find an available port
        for _ in 0..100 {
            let port = rand::rng().random_range(*start..=*end);
            let key = TcpPortAllocation::key_for_port(port);

            // Check if port is already allocated
            if self.store.get::<TcpPortAllocation>(key.clone())?.is_none() {
                let allocation = TcpPortAllocation {
                    port,
                    tenant: tenant.clone(),
                    resource_name: resource_name.clone(),
                    resource_namespace: resource_namespace.clone(),
                };

                // Port is available, allocate it
                self.store.put(key, allocation)?;

                // Also track it as a service domain for consistency
                let domain_key = TrackedResourceKind::ServiceDomain(format!("tcp:{}", port));
                self.tracker
                    .track_resource_owner(crate::agent::tracker::TrackedResourceOwner {
                        kind: domain_key,
                        tenant: tenant.clone(),
                        resource_name: resource_name.clone(),
                        resource_namespace: resource_namespace.clone(),
                    })
                    .await?;

                return Ok(port);
            }
        }

        bail!(
            "No available TCP ports in configured range {}..{} after 100 attempts",
            start,
            end
        );
    }

    pub async fn deallocate_tcp_port(&self, port: u16) -> Result<()> {
        let key = TcpPortAllocation::key_for_port(port);
        self.store.delete(key)?;

        // Also untrack the service domain
        let domain_key = TrackedResourceKind::ServiceDomain(format!("tcp:{}", port));
        self.tracker.untrack_resource_owner(domain_key).await?;

        Ok(())
    }

    pub async fn get_tcp_port_allocation(&self, port: u16) -> Result<Option<TcpPortAllocation>> {
        let key = TcpPortAllocation::key_for_port(port);
        self.store.get(key)
    }

    pub fn is_tcp_port_in_range(&self, port: u16) -> bool {
        if let Some((start, end)) = &self.tcp_port_range {
            port >= *start && port <= *end
        } else {
            false
        }
    }
}
