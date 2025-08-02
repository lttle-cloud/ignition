use std::{path::Path, sync::Arc};

use anyhow::Result;
use ignition::{
    agent::{
        Agent, AgentConfig,
        image::ImageAgentConfig,
        machine::MachineAgentConfig,
        net::NetAgentConfig,
        proxy::{
            BindingMode, ExternalBindingRouting, ExternnalBindingRoutingTlsNestedProtocol,
            ProxyAgentConfig, ProxyBinding,
        },
        volume::VolumeAgentConfig,
    },
    api::{ApiServer, ApiServerConfig, auth::AuthHandler, core::CoreService},
    controller::{
        machine::MachineController,
        scheduler::{Scheduler, SchedulerConfig},
    },
    machinery::store::Store,
    repository::Repository,
    services,
    utils::tracing::init_tracing,
};
use tokio::{runtime, task::block_in_place};

// TODO: get this from config
const TEMP_JWT_SECRET: &str = "dGVtcF9qd3Rfc2VjcmV0";

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let transient_dir = Path::new("./data/transient");
    if transient_dir.exists() {
        tokio::fs::remove_dir_all(transient_dir).await?;
    }

    let store = Arc::new(Store::new("data").await?);

    let scheduler = Arc::new_cyclic(|scheduler_weak| {
        let repository = Arc::new(Repository::new(store.clone(), scheduler_weak.clone()));

        let agent_scheduler = scheduler_weak.clone();

        let agent = block_in_place(move || {
            runtime::Handle::current().block_on(async {
                Arc::new(
                    Agent::new(
                        AgentConfig {
                            store_path: "./data/agent/store".to_string(),
                            net_config: NetAgentConfig {
                                bridge_name: "ltbr0".to_string(),
                                vm_ip_cidr: "10.0.0.0/24".to_string(),
                                service_ip_cidr: "10.0.1.0/24".to_string(),
                            },
                            volume_config: VolumeAgentConfig {
                                base_path: "./data/agent/volumes".to_string(),
                            },
                            image_config: ImageAgentConfig {
                                base_path: "./data/agent/images".to_string(),
                            },
                            machine_config: MachineAgentConfig {
                                transient_state_path: transient_dir.to_path_buf(),
                                kernel_path: "/home/lttle/linux/vmlinux".to_string(),
                                initrd_path: "/home/lttle/ignition-v2/target/takeoff.cpio".to_string(),
                                kernel_cmd_init:
                                    "i8042.nokbd reboot=t panic=1 noapic clocksource=kvm-clock tsc=reliable console=ttyS0"
                                        .to_string(),
                            },
                            proxy_config: ProxyAgentConfig {
                                external_bind_address: "151.80.18.214".to_string(),
                                default_tls_cert_path: "./certs/server.cert".to_string(),
                                default_tls_key_path: "./certs/server.key".to_string(),
                                evergreen_external_ports: vec![80, 443],
                            },
                        },
                        agent_scheduler,
                    )
                    .await
                    .expect("Failed to create agent"))
            })
        });

        let scheduler = Scheduler::new(
            store.clone(),
            repository.clone(),
            agent,
            SchedulerConfig { worker_count: 1 },
            vec![MachineController::new_boxed()],
        );

        scheduler
    });

    let repository = scheduler.repository.clone();

    let auth_handler = Arc::new(AuthHandler::new(TEMP_JWT_SECRET));

    scheduler
        .agent
        .proxy()
        .set_binding(
            "landing-page",
            ProxyBinding {
                // target_network_tag: "test_tenant-default/caddy-test".to_string(),
                target_network_tag: "test_tenant-default/landing-page".to_string(),
                target_port: 80,
                mode: BindingMode::External {
                    port: 443,
                    routing: ExternalBindingRouting::TlsSni {
                        host: "landing.alpha1.ovh-rbx.lttle.host".to_string(),
                        nested_protocol: ExternnalBindingRoutingTlsNestedProtocol::Http,
                    },
                },
            },
        )
        .await
        .expect("failed to add test proxy binding");

    let api_server = ApiServer::new(
        store.clone(),
        repository.clone(),
        scheduler.clone(),
        auth_handler.clone(),
        ApiServerConfig {
            host: "0.0.0.0".to_string(),
            port: 5100,
        },
    )
    .add_service::<CoreService>()
    .add_service::<services::MachineService>();

    scheduler.start_workers();
    scheduler.schedule_bringup().await?;

    api_server.start().await?;

    Ok(())
}
