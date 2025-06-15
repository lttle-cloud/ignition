pub mod image;
pub mod job;
pub mod logs;
pub mod machine;
mod model;
pub mod net;
pub mod proxy;
pub mod service;
pub mod volume;

use std::{net::Ipv4Addr, sync::Arc, time::Duration};

use image::ImagePool;
use net::{ip::IpPool, tap::TapPool};
use oci_client::Reference;
use util::{
    async_runtime::{sync::RwLock, time::sleep},
    result::{Result, bail},
    tracing::{self, warn},
};
use vmm::config::SnapshotPolicy;

use crate::{
    image::Image,
    job::{
        Job,
        machine::{
            BringUpMachineJob, BringUpMachineJobInput, DeployMachineJob, DeployMachineJobInput,
        },
    },
    logs::LogsPool,
    machine::{MachineInfo, MachinePool},
    model::{machine::StoredMachineState, service::Service},
    proxy::{
        Proxy, ProxyServiceBinding, ProxyServiceTarget, ProxyServiceType, ProxyTlsTerminationConfig,
    },
    service::ServicePool,
};

pub use model::service::{ServiceMode, ServiceProtocol, ServiceTarget};

pub struct ControllerConfig {
    pub garbage_collection_interval_secs: u64,
    pub default_tls_termination: ProxyTlsTerminationConfig,
}

pub struct Controller {
    config: ControllerConfig,
    image_pool: Arc<ImagePool>,
    tap_pool: Arc<TapPool>,
    vm_ip_pool: Arc<IpPool>,
    svc_ip_pool: Arc<IpPool>,
    logs_pool: Arc<LogsPool>,
    machine_pool: Arc<MachinePool>,
    service_pool: Arc<ServicePool>,
    proxy: RwLock<Option<Arc<Proxy>>>,
}

pub struct DeployMachineInput {
    pub name: String,
    pub image_name: String,
    pub vcpu_count: u8,
    pub memory_size_mib: usize,
    pub envs: Vec<(String, String)>,
    pub snapshot_policy: Option<SnapshotPolicy>,
}

pub struct DeployServiceInput {
    pub name: String,
    pub target: ServiceTarget,
    pub protocol: ServiceProtocol,
    pub mode: ServiceMode,
}

impl Controller {
    pub fn new(
        config: ControllerConfig,
        image_pool: Arc<ImagePool>,
        tap_pool: Arc<TapPool>,
        vm_ip_pool: Arc<IpPool>,
        svc_ip_pool: Arc<IpPool>,
        logs_pool: Arc<LogsPool>,
        machine_pool: Arc<MachinePool>,
        service_pool: Arc<ServicePool>,
    ) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            config,
            image_pool,
            tap_pool,
            vm_ip_pool,
            svc_ip_pool,
            logs_pool,
            machine_pool,
            service_pool,
            proxy: RwLock::new(None),
        }))
    }

    pub async fn set_proxy(&self, proxy: Arc<Proxy>) {
        let mut proxy_guard = self.proxy.write().await;
        *proxy_guard = Some(proxy);
    }

    pub async fn bring_up(&self) -> Result<()> {
        self.run_garbage_collection_round().await?;

        let stored_machines = self.machine_pool.get_stored_machines().await?;

        tracing::info!("Bringing up machines");
        for stored_machine in stored_machines {
            if stored_machine.state == StoredMachineState::Stopped {
                continue;
            }

            let name = stored_machine.name.clone();
            let id = stored_machine.id.clone();

            tracing::info!("Bringing up machine: {} (id: {})", name, id);

            let job_input = BringUpMachineJobInput { stored_machine };

            let job = BringUpMachineJob::new(
                self.image_pool.clone(),
                self.tap_pool.clone(),
                self.vm_ip_pool.clone(),
                self.logs_pool.clone(),
                self.machine_pool.clone(),
                job_input,
            )?;

            let task = job.task();
            task.start().await?;

            tracing::info!("Machine brought up: {} (id: {})", name, id);
        }
        tracing::info!("Machines brought up");

        tracing::info!("Bringing up services");
        let stored_service = self.service_pool.list_services()?;
        for stored_service in stored_service {
            self.deploy_service(DeployServiceInput {
                name: stored_service.name,
                target: stored_service.target,
                protocol: stored_service.protocol,
                mode: stored_service.mode,
            })
            .await?;
        }
        tracing::info!("Services brought up");

        Ok(())
    }

    pub async fn run_garbage_collection_round(&self) -> Result<()> {
        let used_tap_names: Vec<String> = self.machine_pool.get_used_tap_names().await?;
        self.tap_pool.garbage_collect(used_tap_names).await?;

        let used_ip_addrs = self.machine_pool.get_used_ip_addrs().await?;
        self.vm_ip_pool.garbage_collect(used_ip_addrs)?;

        let used_image_ids = self.machine_pool.get_used_image_ids().await?;
        let used_image_volume_ids = self.machine_pool.get_used_image_volume_ids().await?;

        self.image_pool
            .garbage_collect_images(&used_image_ids)
            .await?;
        self.image_pool
            .garbage_collect_image_copy_volumes(&used_image_ids, &used_image_volume_ids)
            .await?;

        Ok(())
    }

    pub async fn garbage_collection_task(&self) {
        loop {
            if let Err(e) = self.run_garbage_collection_round().await {
                tracing::error!("Garbage collection round failed: {}", e);
            };

            sleep(Duration::from_secs(
                self.config.garbage_collection_interval_secs,
            ))
            .await;
        }
    }

    pub async fn pull_image_if_needed(&self, image_ref_str: &str) -> Result<(Image, bool)> {
        let image_reference = image_ref_str.parse::<Reference>()?;
        self.image_pool.pull_image_if_needed(&image_reference).await
    }

    pub async fn deploy_machine(&self, config: DeployMachineInput) -> Result<MachineInfo> {
        let job_input = DeployMachineJobInput {
            name: config.name,
            image_name: config.image_name,
            vcpu_count: config.vcpu_count,
            memory_size_mib: config.memory_size_mib,
            envs: config.envs,
            snapshot_policy: config.snapshot_policy,
        };

        let job = DeployMachineJob::new(
            self.image_pool.clone(),
            self.tap_pool.clone(),
            self.vm_ip_pool.clone(),
            self.logs_pool.clone(),
            self.machine_pool.clone(),
            job_input,
        )?;

        let task = job.task();
        let (_, stored_machine, _) = task.start().await?;

        let Some(info) = self
            .machine_pool
            .get_machine_info(&stored_machine.id)
            .await?
        else {
            bail!("Machine not found after deployment");
        };

        Ok(info)
    }

    pub async fn get_machine(&self, machine_id: &str) -> Result<Option<MachineInfo>> {
        self.machine_pool.get_machine_info(&machine_id).await
    }

    pub async fn delete_machine(&self, machine_id: &str) -> Result<()> {
        self.machine_pool.delete_machine(machine_id, true).await
    }

    pub async fn stop_machine(&self, machine_id: &str) -> Result<()> {
        self.machine_pool.stop_machine(machine_id).await
    }

    pub async fn start_machine(&self, machine_id: &str) -> Result<()> {
        self.machine_pool.start_machine(machine_id).await
    }

    pub async fn list_machines(&self) -> Result<Vec<MachineInfo>> {
        self.machine_pool.get_machines_info().await
    }

    pub async fn get_ip_for_machine_name(&self, name: &str) -> Result<String> {
        let machine = self
            .machine_pool
            .resolve_existing_machine_from_name(name)
            .await?;

        Ok(machine.config.ip_addr)
    }

    fn get_internal_ip_if_needed(&self, service: &DeployServiceInput) -> Result<Option<String>> {
        let existing_internal_ip =
            if let Some(service) = self.service_pool.get_service(&service.name)? {
                service.internal_ip
            } else {
                None
            };

        let needs_new_ip = ServiceMode::Internal == service.mode && existing_internal_ip.is_none();

        if needs_new_ip {
            let ip = self
                .svc_ip_pool
                .reserve_tagged(&format!("service-{}", service.name))?
                .addr;

            Ok(Some(ip))
        } else {
            Ok(existing_internal_ip)
        }
    }

    pub async fn deploy_service(&self, input: DeployServiceInput) -> Result<()> {
        let proxy = self.proxy.read().await;
        let Some(proxy) = proxy.as_ref() else {
            bail!("Proxy is not set");
        };

        let internal_ip = self.get_internal_ip_if_needed(&input)?;

        let service = Service {
            name: input.name.clone(),
            target: input.target.clone(),
            protocol: input.protocol.clone(),
            mode: input.mode.clone(),
            internal_ip: internal_ip.clone(),
        };

        let proxy_type = match (input.protocol, input.mode) {
            (ServiceProtocol::Tcp { port }, ServiceMode::External { host }) => {
                ProxyServiceType::ExternalTls {
                    port,
                    host,
                    tls_termination: None,
                }
            }
            (ServiceProtocol::Tls { port }, ServiceMode::External { host }) => {
                ProxyServiceType::ExternalTls {
                    port,
                    host,
                    tls_termination: Some(self.config.default_tls_termination.clone()),
                }
            }
            (ServiceProtocol::Http, ServiceMode::External { host }) => {
                ProxyServiceType::ExternalHttps {
                    host,
                    tls_termination: self.config.default_tls_termination.clone(),
                }
            }
            (ServiceProtocol::Tcp { port }, ServiceMode::Internal) => {
                let Some(internal_ip) = internal_ip else {
                    bail!("Internal IP is required for internal services");
                };

                ProxyServiceType::InternalTcp {
                    port,
                    addr: internal_ip.parse::<Ipv4Addr>()?,
                }
            }
            (ServiceProtocol::Http, ServiceMode::Internal) => {
                bail!("HTTP services are not supported for internal mode");
            }
            (ServiceProtocol::Tls { .. }, ServiceMode::Internal) => {
                bail!("TLS services are not supported for internal mode");
            }
        };

        self.service_pool.insert_service(service)?;

        let service_binding = ProxyServiceBinding {
            service_name: input.name.clone(),
            service_target: ProxyServiceTarget {
                machine_name: input.target.name.clone(),
                port: input.target.port,
            },
            service_type: proxy_type,
        };
        proxy.bind_service(service_binding).await?;

        Ok(())
    }

    pub async fn get_service(&self, name: &str) -> Result<Option<Service>> {
        self.service_pool.get_service(name)
    }

    pub async fn list_services(&self) -> Result<Vec<Service>> {
        self.service_pool.list_services()
    }

    pub async fn delete_service(&self, name: &str) -> Result<()> {
        let proxy = self.proxy.read().await;
        let Some(proxy) = proxy.as_ref() else {
            bail!("Proxy is not set");
        };

        if let Err(e) = self.svc_ip_pool.release_tag(&format!("service-{}", name)) {
            warn!("Failed to release service ip: {}", e);
        };

        proxy.unbind_service(name).await?;
        self.service_pool.remove_service(name)?;

        Ok(())
    }
}
