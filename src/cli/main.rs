pub mod cmd;
pub mod ui;

use crate::{
    cmd::machine::{MachineTable, MachineTableRow},
    ui::summary::{Summary, SummaryCellStyle, SummaryRow},
};

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

    let summary = Summary {
        rows: vec![
            SummaryRow {
                name: "name".to_string(),
                cell_style: SummaryCellStyle::Default,
                value: Some("Machine 1asdfasda1asdfasda1asdfasda1asdfasda1asdfasda1asdfasda1asdfasda1asdfasda1asdfasda1asdfasda1asdfasda1asdfasda1asdfasda".to_string()),
            },
            SummaryRow {
                name: "status".to_string(),
                cell_style: SummaryCellStyle::Important,
                value: Some("running".to_string()),
            },
            SummaryRow {
                name: "ip".to_string(),
                cell_style: SummaryCellStyle::Default,
                value: Some("192.168.1.1".to_string()),
            },
            SummaryRow {
                name: "some long text to test this".to_string(),
                cell_style: SummaryCellStyle::Default,
                value: Some("00:00:00:00:00:00".to_string()),
            },
            SummaryRow {
                name: "some empty value".to_string(),
                cell_style: SummaryCellStyle::Default,
                value: None,
            },
            SummaryRow {
                name: "image".to_string(),
                cell_style: SummaryCellStyle::Default,
                value: Some("ubuntu:22.04".to_string()),
            },
        ],
    };

    summary.print();
}
