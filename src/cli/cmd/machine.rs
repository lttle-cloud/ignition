use meta::table;

#[table]
pub struct MachineTable {
    #[field(name = "name")]
    name: String,

    #[field(name = "status", cell_style = important)]
    status: Option<String>,
}
