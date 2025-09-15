use std::{collections::BTreeMap, sync::Arc};

use anyhow::{Result, bail};
use async_trait::async_trait;
use tracing::{error, info};

use crate::{
    agent::Agent,
    constants::DEFAULT_NAMESPACE,
    controller::{
        AdmissionCheckBeforeDelete, AdmissionCheckBeforeSet, Controller, ReconcileNext,
        context::{ControllerContext, ControllerEvent, ControllerKey},
    },
    repository::Repository,
    resource_index::ResourceKind,
    resources::{
        Convert, ProvideMetadata,
        app::{App, AppAllocatedService, AppExpose, AppV1},
        machine::{Machine, MachineV1},
        metadata::{Metadata, Namespace},
        service::{
            Service, ServiceBind, ServiceBindExternalProtocol, ServiceTarget,
            ServiceTargetProtocol, ServiceV1,
        },
    },
};

pub struct AppController;

impl AppController {
    pub fn new_boxed() -> Box<Self> {
        Box::new(Self)
    }
}

#[async_trait]
impl Controller for AppController {
    async fn schedule(
        &self,
        ctx: ControllerContext,
        event: ControllerEvent,
    ) -> Result<Option<ControllerKey>> {
        info!("scheduling app controller for event: {:?}", event);
        let key = match event {
            ControllerEvent::ResourceChange(ResourceKind::App, metadata) => {
                Some(ControllerKey::new(
                    ctx.tenant.clone(),
                    ResourceKind::App,
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
            "should reconcile app controller for key: {}",
            key.to_string()
        );

        return key.kind == ResourceKind::App;
    }

    async fn reconcile(&self, ctx: ControllerContext, key: ControllerKey) -> Result<ReconcileNext> {
        info!("reconciling app controller for key: {}", key.to_string());

        let Some((app, status)) = ctx
            .repository
            .app(ctx.tenant.clone())
            .get_with_status(key.metadata().clone())?
        else {
            // the app was deleted; cleaning up.
            let Some(status) = ctx
                .repository
                .app(ctx.tenant.clone())
                .get_status(key.metadata().clone())?
            else {
                return Ok(ReconcileNext::done());
            };

            if let Some(machine_name) = status.machine_name {
                ctx.repository
                    .machine(ctx.tenant.clone())
                    .delete(
                        Namespace::from_value(key.metadata().namespace.clone()),
                        machine_name,
                    )
                    .await
                    .ok();
            }

            for (_, allocated_service) in status.allocated_services.iter() {
                let Some(service) = ctx.repository.service(ctx.tenant.clone()).get(
                    Namespace::from_value(key.metadata().namespace.clone()),
                    allocated_service.name.clone(),
                )?
                else {
                    continue;
                };

                service
                    .before_delete(
                        ctx.tenant.clone(),
                        ctx.repository.clone(),
                        ctx.agent.clone(),
                        service.metadata().clone(),
                    )
                    .await?;

                ctx.repository
                    .service(ctx.tenant.clone())
                    .delete(
                        Namespace::from_value(key.metadata().namespace.clone()),
                        allocated_service.name.clone(),
                    )
                    .await
                    .ok();
            }

            ctx.repository
                .app(ctx.tenant.clone())
                .delete_status(key.metadata().clone())
                .await?;

            return Ok(ReconcileNext::done());
        };

        let app = app.latest();
        let resolved_namespace = Namespace::from_value(app.namespace.clone())
            .as_value()
            .unwrap_or(DEFAULT_NAMESPACE.to_string());

        let mut tags = app.tags.clone().unwrap_or_default();
        tags.push(format!(
            "ignitiond.owner={}/{}",
            resolved_namespace, app.name
        ));

        let machine = MachineV1 {
            name: app.name.clone(),
            namespace: Some(resolved_namespace.clone()),
            tags: Some(tags.clone()),
            image: app.image.clone(),
            build: None,
            resources: app.resources.clone(),
            restart_policy: app.restart_policy.clone(),
            mode: app.mode.clone(),
            volumes: app.volumes.clone(),
            command: app.command.clone(),
            environment: app.environment.clone(),
            depends_on: app.depends_on.clone(),
        };

        let exposed = app.expose.clone().unwrap_or_default();
        let mut services = BTreeMap::new();

        for (expose_name, expose) in exposed.iter() {
            let service = generate_service_from_expose(
                ctx.agent.clone(),
                ctx.tenant.as_str(),
                &app,
                expose_name,
                expose,
            )?;
            services.insert(expose_name, service);
        }

        // delete services that are in the status but not in the expose configuration
        for (allocated_service_name, allocated_service) in status.allocated_services.iter() {
            if !services.contains_key(allocated_service_name) {
                let Some(service) = ctx.repository.service(ctx.tenant.clone()).get(
                    Namespace::from_value(resolved_namespace.clone().into()),
                    allocated_service.name.clone(),
                )?
                else {
                    continue;
                };

                service
                    .before_delete(
                        ctx.tenant.clone(),
                        ctx.repository.clone(),
                        ctx.agent.clone(),
                        service.metadata().clone(),
                    )
                    .await?;

                ctx.repository
                    .service(ctx.tenant.clone())
                    .delete(
                        Namespace::from_value(resolved_namespace.clone().into()),
                        allocated_service.name.clone(),
                    )
                    .await?;

                ctx.repository
                    .app(ctx.tenant.clone())
                    .patch_status(key.metadata().clone(), |status| {
                        status.allocated_services.remove(allocated_service_name);
                    })
                    .await?;
            }
        }

        let machine_name = machine.name.clone();
        let machine_resource = Machine::V1(machine);
        let machine_hash = machine_resource.hash_with_updated_metadata();

        // we always apply the mahcine resource
        // giving the machine a chance to reconcile image digest change for the same tag
        ctx.repository
            .machine(ctx.tenant.clone())
            .set(machine_resource)
            .await?;

        ctx.repository
            .app(ctx.tenant.clone())
            .patch_status(key.metadata().clone(), |status| {
                status.machine_hash = machine_hash;
                status.machine_name = Some(machine_name.clone());
            })
            .await?;

        for (service_name, service) in services.iter() {
            let service_resource = Service::V1(service.clone());
            let service_name = service_name.to_owned().clone();

            let service_hash = service_resource.hash_with_updated_metadata();

            if let Some(allocated_service) = status.allocated_services.get(service_name.as_str()) {
                if allocated_service.hash == service_hash {
                    continue;
                }
            }

            ctx.repository
                .service(ctx.tenant.clone())
                .set(service_resource)
                .await?;

            let domain = match &service.bind {
                ServiceBind::External { host, .. } => Some(host.clone()),
                _ => None,
            };

            ctx.repository
                .app(ctx.tenant.clone())
                .patch_status(key.metadata().clone(), |status| {
                    status.allocated_services.insert(
                        service_name.to_owned().clone(),
                        AppAllocatedService {
                            name: service.name.to_owned(),
                            hash: service_hash,
                            domain: domain.clone(),
                        },
                    );
                })
                .await?;
        }

        Ok(ReconcileNext::done())
    }

    async fn handle_error(
        &self,
        _ctx: ControllerContext,
        key: ControllerKey,
        err: anyhow::Error,
    ) -> ReconcileNext {
        error!(
            "handling error for app controller for key: {} error: {}",
            key.to_string(),
            err
        );

        ReconcileNext::done()
    }
}

fn generate_service_from_expose(
    agent: Arc<Agent>,
    tenant: &str,
    app: &AppV1,
    expose_name: &str,
    expose: &AppExpose,
) -> Result<ServiceV1> {
    let resolved_namespace = Namespace::from_value(app.namespace.clone())
        .as_value()
        .unwrap_or(DEFAULT_NAMESPACE.to_string());

    let service_name = format!("{}-{}", app.name, expose_name);

    let service_target = match (expose.internal.clone(), expose.external.clone()) {
        (Some(_internal), None) => ServiceTarget {
            name: app.name.clone(),
            namespace: Some(resolved_namespace.clone()),
            port: expose.port,
            protocol: ServiceTargetProtocol::Tcp,
            connection_tracking: expose.connection_tracking.clone(),
        },
        (None, Some(external)) => ServiceTarget {
            name: app.name.clone(),
            namespace: Some(resolved_namespace.clone()),
            port: expose.port,
            protocol: match external.protocol {
                ServiceBindExternalProtocol::Http => ServiceTargetProtocol::Http,
                ServiceBindExternalProtocol::Https => ServiceTargetProtocol::Http,
                ServiceBindExternalProtocol::Tls => ServiceTargetProtocol::Tcp,
            },
            connection_tracking: expose.connection_tracking.clone(),
        },
        _ => bail!(
            "invalid expose configuration for app: {} {}",
            app.name,
            expose_name
        ),
    };

    let service_bind = match (expose.internal.clone(), expose.external.clone()) {
        (Some(internal), None) => ServiceBind::Internal {
            port: internal.port,
        },
        (None, Some(external)) => {
            let generated_domain = agent.dns().region_domain_for_service(
                tenant,
                app.name.as_str(),
                resolved_namespace.as_str(),
                expose_name,
            );

            // if we don't have a host set on external.host and the domain allocated exists and is not a region domain, reset the domain to a generated one
            let host = if let Some(host) = external.host {
                host
            } else {
                generated_domain
            };

            ServiceBind::External {
                host,
                port: external.port,
                protocol: external.protocol,
            }
        }
        _ => bail!(
            "invalid expose configuration for app: {} {}",
            app.name,
            expose_name
        ),
    };

    let service = ServiceV1 {
        name: service_name,
        namespace: Some(resolved_namespace.clone()),
        tags: Some(app.tags.clone().unwrap_or_default()),
        target: service_target,
        bind: service_bind,
    };

    Ok(service)
}

#[async_trait]
impl AdmissionCheckBeforeSet for App {
    async fn before_set(
        &self,
        _before: Option<&Self>,
        tenant: String,
        repo: Arc<Repository>,
        agent: Arc<Agent>,
        _metadata: Metadata,
    ) -> Result<()> {
        let resource = self.latest();

        if resource.build.is_some() {
            bail!("app builds must be resolved by client");
        }

        if resource.image.is_none() {
            bail!("image is not set for app: {}", resource.name);
        }

        for (expose_name, expose) in resource.expose.clone().unwrap_or_default().iter() {
            if expose.internal.is_some() && expose.external.is_some() {
                bail!(
                    "app: {} expose: {} cannot have both internal and external",
                    resource.name,
                    expose_name
                );
            }

            // expose name must start with a letter and contain only alphanumeric, single hyphen or underscore
            if !expose_name.chars().next().unwrap().is_ascii_alphabetic()
                || !expose_name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                || expose_name.contains("--")
            {
                bail!(
                    "app: {} expose: {} name must start with a letter and contain only alphanumeric, single hyphen or underscore",
                    resource.name,
                    expose_name
                );
            }

            let service = generate_service_from_expose(
                agent.clone(),
                tenant.as_str(),
                &resource,
                expose_name,
                expose,
            )?;

            let service_resource = Service::V1(service);

            if let Err(e) = service_resource
                .before_set(
                    Some(&service_resource),
                    tenant.clone(),
                    repo.clone(),
                    agent.clone(),
                    service_resource.metadata(),
                )
                .await
            {
                bail!(
                    "failed before set hook for service {}: {}",
                    service_resource.metadata().name,
                    e
                );
            };
        }

        Ok(())
    }
}
