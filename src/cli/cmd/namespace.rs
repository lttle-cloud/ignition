use std::time::Duration;

use anyhow::Result;
use ignition::{machinery::store::now_millis, resources::core::Namespace};
use meta::table;

use crate::{client::get_api_client, config::Config};

#[table]
pub struct NamespaceTable {
    #[field(name = "name")]
    name: String,

    #[field(name = "age")]
    age: String,
}

impl From<Namespace> for NamespaceTableRow {
    fn from(namespace: Namespace) -> Self {
        let age = (now_millis() - namespace.created_at) / 1000;
        let age = Duration::from_secs(age as u64);
        let age = humantime::format_duration(age).to_string();

        Self {
            name: namespace.name,
            age,
        }
    }
}

pub async fn run_namespace_list(config: &Config) -> Result<()> {
    let api_client = get_api_client(config).await?;
    let response = api_client.core().list_namespaces().await?;

    let mut table = NamespaceTable::new();

    for namespace in response.namespaces {
        table.add_row(namespace.into());
    }

    table.print();

    Ok(())
}
