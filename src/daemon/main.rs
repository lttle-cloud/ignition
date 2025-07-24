use std::{sync::Arc, time::Duration};

use anyhow::Result;
use ignition::{
    agent::{
        Agent, AgentConfig,
        image::ImageAgentConfig,
        machine::{
            MachineAgentConfig,
            machine::{
                MachineConfig, MachineMode, MachineResources, MachineState,
                MachineStateRetentionMode, NetworkConfig,
            },
        },
        net::{IpReservationKind, NetAgentConfig},
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
use tracing::info;

// TODO: get this from config
const TEMP_JWT_SECRET: &str = "dGVtcF9qd3Rfc2VjcmV0";

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let store = Arc::new(Store::new("data").await?);

    let scheduler = Arc::new_cyclic(|scheduler_weak| {
        let repository = Arc::new(Repository::new(store.clone(), scheduler_weak.clone()));

        let scheduler = Scheduler::new(
            store.clone(),
            repository.clone(),
            SchedulerConfig { worker_count: 1 },
            vec![MachineController::new_boxed()],
        );

        scheduler
    });

    let repository = scheduler.repository.clone();

    let auth_handler = Arc::new(AuthHandler::new(TEMP_JWT_SECRET));

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

    test_machines().await?;

    scheduler.start_workers();
    api_server.start().await?;

    Ok(())
}

async fn test_machines() -> Result<()> {
    let agent = Agent::new(AgentConfig {
        store_path: "data/agent/store".to_string(),
        net_config: NetAgentConfig {
            bridge_name: "ltbr0".to_string(),
            vm_ip_cidr: "10.0.0.0/24".to_string(),
            service_ip_cidr: "10.0.1.0/24".to_string(),
        },
        volume_config: VolumeAgentConfig {
            base_path: "data/agent/volumes".to_string(),
        },
        image_config: ImageAgentConfig {
            base_path: "data/agent/images".to_string(),
        },
        machine_config: MachineAgentConfig {
            kernel_path: "/home/lttle/linux/vmlinux".to_string(),
            initrd_path: "/home/lttle/ignition/target/takeoff.cpio".to_string(),
            kernel_cmd_init:
                "i8042.nokbd reboot=t panic=1 noapic clocksource=kvm-clock tsc=reliable console=ttyS0"
                    .to_string(),
        },
    })
    .await?;

    let image = agent.image().image_pull("caddy:latest").await?;
    let image_volume = agent.volume().volume_clone(&image.volume_id).await?;

    let ip = agent
        .net()
        .ip_reservation_create(IpReservationKind::VM, None)?;

    let tap_device = agent.net().device_create().await?;

    let machine = agent
        .machine()
        .create_machine(MachineConfig {
            name: "test".to_string(),
            mode: MachineMode::Regular,
            state_retention_mode: MachineStateRetentionMode::InMemory,
            resources: MachineResources {
                cpu: 1,
                memory: 128,
            },
            image,
            image_volume,
            envs: std::collections::HashMap::new(),
            volume_mounts: vec![],
            network: NetworkConfig {
                tap_device: tap_device.name,
                mac_address: "02:42:ac:11:00:02".to_string(),
                ip_address: ip.ip.clone(),
                gateway: "10.0.0.1".to_string(),
                netmask: "255.255.255.0".to_string(),
            },
        })
        .await
        .expect("Failed to create machine");

    let state_watcher = machine
        .watch_state()
        .await
        .expect("Failed to watch machine state");

    tokio::spawn(async move {
        while let Ok(state) = state_watcher.recv().await {
            info!("Machine state: {:?}", state);
        }
    });

    machine.start().await.expect("Failed to start machine");
    machine
        .wait_for_state(MachineState::Ready)
        .await
        .expect("Failed to wait for machine to be ready");
    tokio::time::sleep(Duration::from_secs(2)).await;

    println!("ip = {}", ip.ip);

    machine.stop().await.expect("Failed to stop machine");
    machine
        .wait_for_state(MachineState::Stopped)
        .await
        .expect("Failed to wait for machine to stop");

    println!("machine stopped");

    tokio::time::sleep(Duration::from_secs(2)).await;

    machine.start().await.expect("Failed to start machine");

    machine
        .wait_for_state(MachineState::Ready)
        .await
        .expect("Failed to wait for machine to be ready");
    let res = reqwest::get(format!("http://{}", ip.ip))
        .await
        .expect("Failed to get machine");
    println!("request is ready {}", res.status());

    let boot_duration = machine
        .get_last_boot_duration()
        .await
        .expect("Failed to get boot duration");
    println!(
        "boot duration = {:?}",
        humantime::format_duration(boot_duration)
    );

    tokio::time::sleep(Duration::from_secs(10)).await;

    Ok(())
}
