use std::{path::Path, sync::Arc};

use anyhow::Result;
use ignition::{
    agent::{
        Agent, AgentConfig, image::ImageAgentConfig, machine::MachineAgentConfig,
        net::NetAgentConfig, volume::VolumeAgentConfig,
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
use tokio::{
    runtime,
    task::{block_in_place, spawn_blocking},
};

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
                                kernel_path: "/home/lttle/linux/vmlinux".to_string(),
                                initrd_path: "/home/lttle/ignition-v2/target/takeoff.cpio".to_string(),
                                kernel_cmd_init:
                                    "i8042.nokbd reboot=t panic=1 noapic clocksource=kvm-clock tsc=reliable console=ttyS0"
                                        .to_string(),
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
    api_server.start().await?;

    Ok(())
}

// async fn test_machines() -> Result<()> {
//     let image = agent.image().image_pull("caddy:latest").await?;
//     let image_volume = agent
//         .volume()
//         .volume_clone_with_overlay(&image.volume_id)
//         .await?;

// let Some(image_volume) = agent
//     .volume()
//     .volume("0246ea95-6221-4337-b069-10442a660480")?
// else {
//     bail!("Image volume not found");
// };
// println!("image_volume = {:?}", image_volume.id);

// let ip = agent
//     .net()
//     .ip_reservation_create(IpReservationKind::VM, None)?;

// let tap_device = agent.net().device_create().await?;

// let machine = agent
//     .machine()
//     .create_machine(MachineConfig {
//         name: "test".to_string(),
//         // mode: MachineMode::Flash(SnapshotStrategy::WaitForListenOnPort(80)), // snapshot when the app listens on port 80
//         mode: MachineMode::Flash(SnapshotStrategy::WaitForUserSpaceReady),
//         state_retention_mode: MachineStateRetentionMode::OnDisk {
//             path: transient_dir
//                 .join("machines/test")
//                 .to_string_lossy()
//                 .to_string(),
//         },
//         resources: MachineResources {
//             cpu: 1,
//             memory: 128,
//         },
//         image,
//         envs: std::collections::HashMap::new(),
//         volume_mounts: vec![VolumeMountConfig {
//             volume: image_volume.clone(),
//             mount_at: "/".to_string(),
//             read_only: false,
//             root: true,
//         }],
//         network: NetworkConfig {
//             tap_device: tap_device.name,
//             mac_address: "02:42:ac:11:00:02".to_string(),
//             ip_address: ip.ip.clone(),
//             gateway: "10.0.0.1".to_string(),
//             netmask: "255.255.255.0".to_string(),
//         },
//     })
//     .await
//     .expect("Failed to create machine");

//     let mut watcher = machine.watch_state().await.expect("Failed to watch state");
//     tokio::spawn(async move {
//         while let Ok(state) = watcher.recv().await {
//             println!("machine state = {:?}", state);
//         }
//     });

//     machine.start().await.expect("Failed to start machine");
//     machine
//         .wait_for_state(MachineState::Suspended)
//         .await
//         .expect("Failed to wait for machine to be ready");
//     tokio::time::sleep(Duration::from_secs(2)).await;

//     println!("ip = {}", ip.ip);

//     machine.start().await.expect("Failed to start machine");

//     machine
//         .wait_for_state(MachineState::Ready)
//         .await
//         .expect("Failed to wait for machine to be ready");
//     tokio::time::sleep(Duration::from_secs(2)).await;

//     let res = reqwest::get(format!("http://{}", ip.ip))
//         .await
//         .expect("Failed to get machine");
//     println!("request is ready {}", res.status());

//     let boot_duration = machine
//         .get_last_boot_duration()
//         .await
//         .expect("Failed to get boot duration");
//     println!(
//         "boot duration = {:?}",
//         humantime::format_duration(boot_duration)
//     );

//     tokio::time::sleep(Duration::from_secs(10)).await;

//     Ok(())
// }
