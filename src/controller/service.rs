use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use tokio::{runtime, task::spawn_blocking};
use tracing::{error, info};

use crate::{
    agent::{
        net::IpReservationKind,
        proxy::{
            BindingMode, ExternalBindingRouting, ExternnalBindingRoutingTlsNestedProtocol,
            ProxyBinding,
        },
    },
    constants::DEFAULT_TRAFFIC_AWARE_INACTIVITY_TIMEOUT_SECS,
    controller::{
        Controller, ReconcileNext,
        context::{ControllerContext, ControllerEvent, ControllerKey},
        machine::machine_name_from_key,
    },
    resource_index::ResourceKind,
    resources::{
        Convert,
        metadata::Namespace,
        service::{
            ServiceBind, ServiceBindExternalProtocol, ServiceTargetConnectionTracking,
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
                    Some(ctx.tenant.clone()),
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

        let binding_mode = match service.bind {
            ServiceBind::Internal { port } => BindingMode::Internal {
                service_ip: service_ip.clone(),
                service_port: port.unwrap_or(service.target.port),
            },
            ServiceBind::External {
                host,
                port,
                protocol,
                certificate: _certificate,
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
        };

        let inactivity_timeout = match service.target.connection_tracking {
            Some(ServiceTargetConnectionTracking::TrafficAware { inactivity_timeout }) => {
                Some(Duration::from_secs(
                    inactivity_timeout.unwrap_or(DEFAULT_TRAFFIC_AWARE_INACTIVITY_TIMEOUT_SECS),
                ))
            }
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
