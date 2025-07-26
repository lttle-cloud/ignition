use anyhow::Result;
use ignition::resources::machine::{MachineLatest, MachineStatus};
use meta::{summary, table};

use crate::{
    client::get_api_client,
    cmd::{GetNamespacedArgs, ListNamespacedArgs},
    config::Config,
};

#[table]
pub struct MachineTable {
    #[field(name = "name")]
    name: String,

    #[field(name = "namespace")]
    namespace: Option<String>,

    #[field(name = "status", cell_style = important)]
    status: String,
}

#[summary]
pub struct MachineSummary {
    #[field(name = "name")]
    name: String,

    #[field(name = "namespace")]
    namespace: Option<String>,

    #[field(name = "status", cell_style = important)]
    status: String,
}

impl From<(MachineLatest, MachineStatus)> for MachineSummary {
    fn from((machine, status): (MachineLatest, MachineStatus)) -> Self {
        Self {
            name: machine.name,
            namespace: machine.namespace,
            status: status.phase.to_string(),
        }
    }
}

impl From<(MachineLatest, MachineStatus)> for MachineTableRow {
    fn from((machine, status): (MachineLatest, MachineStatus)) -> Self {
        Self {
            name: machine.name,
            namespace: machine.namespace,
            status: status.phase.to_string(),
        }
    }
}

pub async fn run_machine_list(config: &Config, args: ListNamespacedArgs) -> Result<()> {
    let api_client = get_api_client(config).await?;
    let machines = api_client.machine().list(args.into()).await?;

    let mut table = MachineTable::new();

    for (machine, status) in machines {
        table.add_row(MachineTableRow::from((machine, status)));
    }

    table.print();

    Ok(())
}

pub async fn run_machine_get(config: &Config, args: GetNamespacedArgs) -> Result<()> {
    let api_client = get_api_client(config).await?;
    let (machine, status) = api_client
        .machine()
        .get(args.clone().into(), args.name)
        .await?;

    let summary = MachineSummary::from((machine, status));
    summary.print();

    Ok(())
}
