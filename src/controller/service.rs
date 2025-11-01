use std::{sync::Arc, time::Duration};

use anyhow::{Result, bail};
use async_trait::async_trait;
use tokio::{runtime, task::spawn_blocking};
use tracing::{error, info};

use crate::{
    agent::{
        Agent,
        net::IpReservationKind,
        proxy::{
            BindingMode, ExternalBindingRouting, ExternnalBindingRoutingTlsNestedProtocol,
            ProxyBinding,
        },
        tracker::{TrackedResourceKind, TrackedResourceOwner},
    },
    constants::{DEFAULT_NAMESPACE, DEFAULT_TRAFFIC_AWARE_INACTIVITY_TIMEOUT_SECS},
    controller::{
        AdmissionCheckBeforeDelete, AdmissionCheckBeforeSet, Controller, ReconcileNext,
        context::{ControllerContext, ControllerEvent, ControllerKey},
        machine::machine_name_from_key,
    },
    repository::Repository,
    resource_index::ResourceKind,
    resources::{
        Convert,
        metadata::{Metadata, Namespace},
        service::{
            Service, ServiceBind, ServiceBindExternalProtocol, ServiceTargetConnectionTracking,
            ServiceTargetProtocol,
        },
    },
};

pub struct ServiceController;

impl ServiceController {
    pub fn new_boxed() -> Box<Self> {
        Box::new(Self)
    }
}

fn service_name_from_key(key: &ControllerKey) -> String {
    format!("{}-{}", key.tenant, key.metadata().to_string())
}

#[async_trait]
impl Controller for ServiceController {
    async fn schedule(
        &self,
        ctx: ControllerContext,
        event: ControllerEvent,
    ) -> Result<Option<ControllerKey>> {
        info!("scheduling service controller for event: {:?}", event);
        let key = match event {
            ControllerEvent::BringUp(ResourceKind::Service, metadata) => Some(ControllerKey::new(
                ctx.tenant.clone(),
                ResourceKind::Service,
                metadata.namespace,
                metadata.name,
            )),
            ControllerEvent::ResourceChange(ResourceKind::Service, metadata) => {
                Some(ControllerKey::new(
                    ctx.tenant.clone(),
                    ResourceKind::Service,
                    metadata.namespace,
                    metadata.name,
                ))
            }
            _ => None,
        };
        Ok(key)
    }

    async fn should_reconcile(&self, _ctx: ControllerContext, key: ControllerKey) -> bool {
        info!(
            "should reconcile service controller for key: {}",
            key.to_string()
        );

        return key.kind == ResourceKind::Service;
    }

    async fn reconcile(&self, ctx: ControllerContext, key: ControllerKey) -> Result<ReconcileNext> {
        info!(
            "reconciling service controller for key: {}",
            key.to_string()
        );

        let Some((service, status)) = ctx
            .repository
            .service(ctx.tenant.clone())
            .get_with_status(key.metadata().clone())?
        else {
            // the service was deleted.

            let service_name = service_name_from_key(&key);
            let proxy_agent = ctx.agent.proxy();
            spawn_blocking(move || {
                runtime::Handle::current()
                    .block_on(async { proxy_agent.remove_binding(&service_name).await.ok() })
            })
            .await
            .ok();

            let Some(status) = ctx
                .repository
                .service(ctx.tenant.clone())
                .get_status(key.metadata().clone())?
            else {
                return Ok(ReconcileNext::done());
            };

            // we still have the status. time to clean up
            if let Some(service_ip) = status.service_ip {
                ctx.agent
                    .net()
                    .ip_reservation_delete(IpReservationKind::Service, &service_ip)
                    .ok();
            }

            if let Some(allocated_port) = status.allocated_tcp_port {
                ctx.agent
                    .port_allocator()
                    .deallocate_tcp_port(allocated_port)
                    .await
                    .ok();
            }

            ctx.repository
                .service(ctx.tenant.clone())
                .delete_status(key.metadata().clone())
                .await?;

            return Ok(ReconcileNext::done());
        };
        let service = service.latest();

        let service_ip = if let Some(service_ip) = status.service_ip {
            service_ip
        } else {
            ctx.agent
                .net()
                .ip_reservation_create(
                    IpReservationKind::Service,
                    Some(service_name_from_key(&key)),
                    ctx.tenant.clone(),
                )?
                .ip
                .clone()
        };

        let target_namespace = service
            .target
            .namespace
            .clone()
            .or(service.namespace.clone());
        let target_namespace = Namespace::from_value_or_default(target_namespace);

        let target_machine_key = ControllerKey::new(
            key.tenant.clone(),
            ResourceKind::Machine,
            target_namespace.as_value(),
            service.target.name.clone(),
        );
        let target_network_tag = machine_name_from_key(&target_machine_key);

        let internal_dns_hostname = match &service.bind {
            ServiceBind::Internal { .. } => {
                let service_namespace = service.namespace.as_deref().unwrap_or(DEFAULT_NAMESPACE);
                Some(
                    ctx.agent
                        .dns()
                        .get_internal_dns_for_svc(&service.name, service_namespace),
                )
            }
            ServiceBind::External { .. } => None,
            ServiceBind::Tcp => None,
        };

        let binding_mode = match service.bind {
            ServiceBind::Internal { port } => BindingMode::Internal {
                service_ip: service_ip.clone(),
                service_port: port.unwrap_or(service.target.port),
            },
            ServiceBind::External {
                host,
                port,
                protocol,
            } => {
                let port = port.unwrap_or(protocol.default_port(&service.target));

                let routing = match (protocol, service.target.protocol) {
                    (ServiceBindExternalProtocol::Http, ServiceTargetProtocol::Http) => {
                        ExternalBindingRouting::HttpHostHeader { host: host.clone() }
                    }
                    (ServiceBindExternalProtocol::Https, ServiceTargetProtocol::Http) => {
                        ExternalBindingRouting::TlsSni {
                            host: host.clone(),
                            nested_protocol: ExternnalBindingRoutingTlsNestedProtocol::Http,
                        }
                    }
                    (ServiceBindExternalProtocol::Tls, ServiceTargetProtocol::Http) => {
                        ExternalBindingRouting::TlsSni {
                            host: host.clone(),
                            nested_protocol: ExternnalBindingRoutingTlsNestedProtocol::Http,
                        }
                    }
                    (_, _) => ExternalBindingRouting::TlsSni {
                        host: host.clone(),
                        nested_protocol: ExternnalBindingRoutingTlsNestedProtocol::Unknown,
                    },
                };

                BindingMode::External {
                    port: port,
                    routing: routing,
                }
            }
            ServiceBind::Tcp => {
                // For TCP services, reuse existing allocated port or allocate a new one
                let port = if let Some(existing_port) = status.allocated_tcp_port {
                    // Reuse existing allocated port
                    info!(
                        "Reusing existing TCP port {} for service {}",
                        existing_port, service.name
                    );
                    existing_port
                } else {
                    // Allocate a new dynamic port
                    let allocated_port = ctx
                        .agent
                        .port_allocator()
                        .allocate_tcp_port(
                            key.tenant.clone(),
                            service.name.clone(),
                            service
                                .namespace
                                .clone()
                                .unwrap_or(DEFAULT_NAMESPACE.to_string()),
                        )
                        .await?;
                    info!(
                        "Allocated new TCP port {} for service {}",
                        allocated_port, service.name
                    );
                    allocated_port
                };

                BindingMode::External {
                    port,
                    routing: ExternalBindingRouting::TcpDirect { port },
                }
            }
        };

        let inactivity_timeout = match service.target.connection_tracking {
            Some(ServiceTargetConnectionTracking::TrafficAware { inactivity_timeout }) => {
                Some(Duration::from_secs(
                    inactivity_timeout.unwrap_or(DEFAULT_TRAFFIC_AWARE_INACTIVITY_TIMEOUT_SECS),
                ))
            }
            _ => None,
        };

        // Store allocated TCP port in status for tracking
        let allocated_tcp_port = match &binding_mode {
            BindingMode::External { port, routing } => match routing {
                ExternalBindingRouting::TcpDirect { .. } => Some(*port),
                _ => None,
            },
            _ => None,
        };

        let binding_name = service_name_from_key(&key);
        let proxy_binding = ProxyBinding {
            target_network_tag,
            target_port: service.target.port,
            mode: binding_mode,
            inactivity_timeout,
        };

        let proxy_agent = ctx.agent.proxy();
        spawn_blocking(move || {
            runtime::Handle::current().block_on(async {
                proxy_agent
                    .set_binding(&binding_name, proxy_binding)
                    .await
                    .ok()
            })
        })
        .await
        .ok();

        ctx.repository
            .service(ctx.tenant.clone())
            .patch_status(key.metadata().clone(), |status| {
                status.service_ip = Some(service_ip.clone());
                status.internal_dns_hostname = internal_dns_hostname.clone();
                status.allocated_tcp_port = allocated_tcp_port;
            })
            .await?;

        Ok(ReconcileNext::done())
    }

    async fn handle_error(
        &self,
        _ctx: ControllerContext,
        key: ControllerKey,
        err: anyhow::Error,
    ) -> ReconcileNext {
        error!(
            "handling error for service controller for key: {} error: {}",
            key.to_string(),
            err
        );

        ReconcileNext::done()
    }
}

#[async_trait]
impl AdmissionCheckBeforeSet for Service {
    async fn before_set(
        &self,
        before: Option<&Self>,
        tenant: String,
        _repo: Arc<Repository>,
        agent: Arc<Agent>,
        _metadata: Metadata,
    ) -> Result<()> {
        let resource = self.latest();

        match &resource.bind {
            ServiceBind::Tcp => {
                // TCP services don't need additional validation - they use dynamic allocation
                return Ok(());
            }
            ServiceBind::External {
                host,
                port,
                protocol,
            } => {
                // For external protocols, validate port range restrictions
                let port_allocator = agent.port_allocator();
                let actual_port = port.unwrap_or(protocol.default_port(&resource.target));

                if port_allocator.is_tcp_port_in_range(actual_port) {
                    bail!(
                        "Port {} is in the reserved TCP port range and cannot be used for {} services",
                        actual_port,
                        protocol.to_string()
                    );
                }

                let dns = agent.dns();
                if dns.is_region_domain(host) && !dns.is_tenant_owned_region_domain(&tenant, host) {
                    bail!("Your tenant does not own the domain: {}", host);
                }
            }
            ServiceBind::Internal { .. } => {
                // Internal services don't need port range validation
            }
        }

        // Only handle tracking for external services
        if let ServiceBind::External {
            host,
            port,
            protocol,
        } = &resource.bind
        {
            if let Some(before) = before {
                let before = before.latest();
                if let ServiceBind::External {
                    host: before_host,
                    port: before_port,
                    protocol: before_protocol,
                } = &before.bind
                {
                    if before_host != host || before_port != port {
                        let before_port =
                            before_port.unwrap_or(before_protocol.default_port(&before.target));

                        let before_kind = TrackedResourceKind::ServiceDomain(format!(
                            "{}:{}",
                            before_host, before_port
                        ));
                        agent.tracker().untrack_resource_owner(before_kind).await?;
                    }
                }
            }

            let port = port.unwrap_or(protocol.default_port(&resource.target));
            let kind = TrackedResourceKind::ServiceDomain(format!("{}:{}", host, port));

            let resource_owner = TrackedResourceOwner {
                kind: kind.clone(),
                tenant,
                resource_name: resource.name,
                resource_namespace: resource.namespace.unwrap_or(DEFAULT_NAMESPACE.to_string()),
            };

            if let Some(owner) = agent
                .tracker()
                .get_tracked_resource_owner(kind.clone())
                .await?
            {
                if owner != resource_owner {
                    bail!("Service domain and port is already bound to another resource")
                }
            };

            agent.tracker().track_resource_owner(resource_owner).await?;
        }

        Ok(())
    }
}

#[async_trait]
impl AdmissionCheckBeforeDelete for Service {
    async fn before_delete(
        &self,
        _tenant: String,
        _repo: Arc<Repository>,
        agent: Arc<Agent>,
        _metadata: Metadata,
    ) -> Result<()> {
        let resource = self.latest();

        match &resource.bind {
            ServiceBind::External {
                host,
                port,
                protocol,
            } => {
                let port = port.unwrap_or(protocol.default_port(&resource.target));
                let kind = TrackedResourceKind::ServiceDomain(format!("{}:{}", host, port));
                agent.tracker().untrack_resource_owner(kind).await?;
            }
            ServiceBind::Tcp => {
                // TCP services don't track domains, only port allocations
                // Port deallocation is handled in the reconcile method
            }
            ServiceBind::Internal { .. } => {
                // Internal services don't track domains
            }
        }

        Ok(())
    }
}
