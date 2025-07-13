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

    #[field(name = "bleah")]
    bleah: String,

    #[field(name = "bleah2")]
    bleah2: String,

    #[field(name = "test", cell_style = important)]
    test: String,
}

#[summary]
pub struct MachineSummary {
    #[field(name = "name")]
    name: String,

    #[field(name = "namespace")]
    namespace: Option<String>,

    #[field(name = "bleah")]
    bleah: String,

    #[field(name = "second bleah")]
    bleah2: String,

    #[field(name = "status test (sum)", cell_style = important)]
    test: String,
}

impl From<(MachineLatest, MachineStatus)> for MachineSummary {
    fn from((machine, status): (MachineLatest, MachineStatus)) -> Self {
        Self {
            name: machine.name,
            namespace: machine.namespace,
            bleah: machine.bleah.to_string(),
            bleah2: machine.bleah2.to_string(),
            test: status.test.to_string(),
        }
    }
}

impl From<(MachineLatest, MachineStatus)> for MachineTableRow {
    fn from((machine, status): (MachineLatest, MachineStatus)) -> Self {
        Self {
            name: machine.name,
            namespace: machine.namespace,
            bleah: machine.bleah.to_string(),
            bleah2: machine.bleah2.to_string(),
            test: status.test.to_string(),
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
