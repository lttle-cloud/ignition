use meta::{summary, table};

#[table]
pub struct MachineTable {
    #[field(name = "name")]
    name: String,

    #[field(name = "status", cell_style = important)]
    status: Option<String>,
}

#[summary]
pub struct MachineSummary {
    #[field(name = "name")]
    name: String,

    #[field(name = "status", cell_style = important)]
    status: String,

    #[field(name = "ip")]
    ip: String,

    #[field(name = "some long text to test names")]
    mac: String,

    #[field(name = "maybe empty value")]
    version: Option<String>,

    #[field(name = "image")]
    image: String,
}
