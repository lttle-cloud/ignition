use crate::{client::get_client, config::Config};
use comfy_table::Table;
use ignition_client::ignition_proto::{
    machine::{machine_snapshot_policy::Policy, MachineStatus},
    util::Empty,
};

use util::result::Result;

pub async fn run_machine_list(config: Config) -> Result<()> {
    let client = get_client(&config).await?;

    let machines = client.machine().list(Empty {}).await?.into_inner();

    let mut table = Table::new();
    table.set_header(vec![
        "Id",
        "Name",
        "Image",
        "Snapshot Policy",
        "IP Address",
        "Status",
    ]);

    for machine in machines.machines {
        let snapshot_policy = match machine.snapshot_policy.and_then(|p| p.policy) {
            Some(snapshot_policy) => match snapshot_policy {
                Policy::Manual(_) => "manual".to_string(),
                Policy::OnNthListenSyscall(_) => "on nth listen syscall".to_string(),
                Policy::OnListenOnPort(_) => "on listen on port".to_string(),
                Policy::OnUserspaceReady(_) => "on userspace ready".to_string(),
            },
            None => "disabled".to_string(),
        };

        let status: MachineStatus = machine.status.try_into()?;
        let status = match status {
            MachineStatus::New => "new".to_string(),
            MachineStatus::Running => "running".to_string(),
            MachineStatus::Ready => "ready".to_string(),
            MachineStatus::Suspended => "suspended".to_string(),
            MachineStatus::Stopping => "stopping".to_string(),
            MachineStatus::Stopped => "stopped".to_string(),
            MachineStatus::Error => "error".to_string(),
        };

        table.add_row(vec![
            machine.id,
            machine.name,
            machine.image_reference,
            snapshot_policy,
            machine.ip_addr.unwrap_or("none".to_string()),
            status,
        ]);
    }

    println!("{}", table);

    Ok(())
}
