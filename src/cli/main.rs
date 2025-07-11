pub mod cmd;
pub mod ui;

use crate::cmd::machine::{MachineSummary, MachineTable, MachineTableRow};

fn main() {
    println!("\n\n\n");

    let mut machine_table = MachineTable::new();
    machine_table.add_row(MachineTableRow {
        name: "Machine 1asdfasda1asdfasda1asdfasda1asdfasda1asdfasda1asdfasda1asdfasda1asdfasda1asdfasda1asdfasda1asdfasda1asdfasda1asdfasda".to_string(),
        status: Some("running".to_string()),
    });
    machine_table.add_row(MachineTableRow {
        name: "Machine 2".to_string(),
        status: None,
    });
    machine_table.add_row(MachineTableRow {
        name: "Machine 3".to_string(),
        status: Some("stopped".to_string()),
    });
    machine_table.print();

    println!("\n\n\n");

    let machine_summary = MachineSummary {
        name: "Machine 1asdfasda1asdfasda1asdfasda1asdfasda1asdfasda1asdfasda1asdfasda1asdfasda1asdfasda1asdfasda1asdfasda1asdfasda1asdfasda".to_string(),
        status: "running".to_string(),
        ip: "192.168.1.1".to_string(),
        mac: "00:00:00:00:00:00".to_string(),
        version: None,
        image: "ubuntu:22.04".to_string(),
    };
    machine_summary.print();
}
