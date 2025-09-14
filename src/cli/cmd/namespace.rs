use std::time::Duration;

use ansi_term::{Color, Style};
use anyhow::Result;
use clap::Args;
use ignition::{
    machinery::store::now_millis,
    resources::core::{DeleteNamespaceParams, Namespace},
};
use meta::table;

use crate::{
    client::get_api_client,
    config::Config,
    ui::message::{message_info, message_warn},
};

#[table]
pub struct NamespaceTable {
    #[field(name = "name")]
    name: String,

    #[field(name = "age")]
    age: String,
}

#[derive(Args)]
pub struct NamespaceDeleteArgs {
    #[arg(long = "yes", short = 'y')]
    confirm: bool,

    namespace: String,
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
    let api_client = get_api_client(config.try_into()?);
    let response = api_client.core().list_namespaces().await?;

    let mut table = NamespaceTable::new();

    for namespace in response.namespaces {
        table.add_row(namespace.into());
    }

    table.print();

    Ok(())
}

pub async fn run_namespace_delete(config: &Config, args: NamespaceDeleteArgs) -> Result<()> {
    let api_client = get_api_client(config.try_into()?);
    let response = api_client
        .core()
        .delete_namespace(DeleteNamespaceParams {
            namespace: args.namespace.clone(),
            confirm: args.confirm,
        })
        .await?;

    if response.did_delete {
        message_info(format!("Namespace '{}' has been deleted.", args.namespace));
    } else {
        message_info("Resources found in namespace:");
    }

    let kind_style = Style::new().fg(Color::Yellow);
    let name_style = Style::new().fg(Color::Blue).bold();

    for namespace in response.resources {
        eprintln!(
            "→ {}: {}",
            kind_style.paint(namespace.kind),
            name_style.paint(namespace.name)
        );
    }

    if !response.did_delete {
        message_warn(format!(
            "You are about to delete the namespace '{}' and all resources listed above. This action cannot be undone. To confirm, run the command with --yes (or -y).",
            args.namespace
        ));
    }

    Ok(())
}
