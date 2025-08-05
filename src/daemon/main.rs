mod cmd;
mod config;

use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use ignition::{
    agent::{
        Agent, AgentConfig, dns::config::DnsAgentConfig, image::ImageAgentConfig,
        machine::MachineAgentConfig, net::NetAgentConfig, proxy::ProxyAgentConfig,
        volume::VolumeAgentConfig,
    },
    api::{ApiServer, ApiServerConfig, auth::AuthHandler, core::CoreService},
    constants::DEFAULT_KERNEL_CMD_LINE_INIT,
    controller::{
        machine::MachineController,
        scheduler::{Scheduler, SchedulerConfig},
        service::ServiceController,
    },
    machinery::store::Store,
    repository::Repository,
    services,
    utils::tracing::init_tracing,
};
use tokio::{runtime, task::block_in_place};
use tracing::info;

use crate::config::Config;

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let args = cmd::Cli::parse();

    let config = Config::load(args.config_path).await?;
    info!("Loaded config from {}", config.config_path.display());
    dbg!(&config);

    if !config.absolute_data_dir().exists() {
        tokio::fs::create_dir_all(&config.absolute_data_dir()).await?;
    }

    let store = Arc::new(Store::new(&config.absolute_data_dir()).await?);

    let scheduler = Arc::new_cyclic(|scheduler_weak| {
        let repository = Arc::new(Repository::new(store.clone(), scheduler_weak.clone()));

        let agent_scheduler = scheduler_weak.clone();
        let repository_clone = repository.clone();

        let scheduler_config = config.clone();
        let agent = block_in_place(move || {
            runtime::Handle::current().block_on(async {
                let transient_dir = scheduler_config.absolute_data_dir().join("transient");
                if transient_dir.exists() {
                    tokio::fs::remove_dir_all(&transient_dir)
                        .await
                        .expect("Failed to clear transient directory");
                }

                let agent_dir = scheduler_config.absolute_data_dir().join("agent");

                Arc::new(
                    Agent::new(
                        AgentConfig {
                            store_path: agent_dir.join("store").to_string_lossy().to_string(),
                            net_config: NetAgentConfig {
                                bridge_name: scheduler_config.net_config.bridge_name,
                                vm_ip_cidr: scheduler_config.net_config.vm_ip_cidr,
                                service_ip_cidr: scheduler_config.net_config.service_ip_cidr,
                            },
                            volume_config: VolumeAgentConfig {
                                base_path: agent_dir.join("volumes").to_string_lossy().to_string(),
                            },
                            image_config: ImageAgentConfig {
                                base_path: agent_dir.join("images").to_string_lossy().to_string(),
                            },
                            machine_config: MachineAgentConfig {
                                transient_state_path: transient_dir.to_path_buf().join("machines"),
                                kernel_path: scheduler_config
                                    .config_dir
                                    .join(&scheduler_config.machine_config.kernel_path)
                                    .to_string_lossy()
                                    .to_string(),
                                initrd_path: scheduler_config
                                    .config_dir
                                    .join(&scheduler_config.machine_config.initrd_path)
                                    .to_string_lossy()
                                    .to_string(),
                                kernel_cmd_init: format!(
                                    "{} {}",
                                    DEFAULT_KERNEL_CMD_LINE_INIT,
                                    scheduler_config
                                        .machine_config
                                        .append_cmd_line
                                        .unwrap_or_default()
                                )
                                .trim()
                                .to_string(),
                            },
                            proxy_config: ProxyAgentConfig {
                                external_bind_address: scheduler_config
                                    .proxy_config
                                    .external_bind_address,
                                default_tls_cert_path: scheduler_config
                                    .proxy_config
                                    .default_tls_cert_path,
                                default_tls_key_path: scheduler_config
                                    .proxy_config
                                    .default_tls_key_path,
                                evergreen_external_ports: vec![80, 443],
                            },
                            dns_config: DnsAgentConfig {
                                zone_suffix: scheduler_config.dns_config.zone_suffix,
                                default_ttl: scheduler_config.dns_config.default_ttl,
                                upstream_dns_servers: scheduler_config.dns_config.upstream_dns_servers,
                            },
                        },
                        agent_scheduler,
                        repository_clone,
                    )
                    .await
                    .expect("Failed to create agent"),
                )
            })
        });

        let scheduler = Scheduler::new(
            store.clone(),
            repository.clone(),
            agent,
            SchedulerConfig { worker_count: 1 },
            vec![
                MachineController::new_boxed(),
                ServiceController::new_boxed(),
            ],
        );

        scheduler
    });

    let repository = scheduler.repository.clone();

    let auth_handler = Arc::new(AuthHandler::new(&config.api_server_config.jwt_secret));

    let api_server = ApiServer::new(
        store.clone(),
        repository.clone(),
        scheduler.clone(),
        auth_handler.clone(),
        ApiServerConfig {
            host: config.api_server_config.host.clone(),
            port: config.api_server_config.port,
        },
    )
    .add_service::<CoreService>()
    .add_service::<services::MachineService>()
    .add_service::<services::ServiceService>();

    scheduler.start_workers();
    scheduler.schedule_bringup().await?;

    api_server.start().await?;

    Ok(())
}
