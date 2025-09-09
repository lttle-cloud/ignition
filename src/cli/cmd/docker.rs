use anyhow::Result;
use clap::Args;

use crate::{client::get_api_client, config::Config, ui::message::message_info};

#[derive(Args)]
pub struct DockerLoginArgs {}

pub async fn run_docker_login(config: &Config, _args: DockerLoginArgs) -> Result<()> {
    let api_client = get_api_client(config.try_into()?);
    let me = api_client.core().me().await?;
    let registry_robot = api_client.core().get_registry_robot().await?;

    message_info(format!(
        "You are logged in as {} (tenant: {})",
        me.sub, me.tenant
    ));

    message_info(format!("To login to the registry, run the command below:",));

    println!(
        "docker login {} -u {} -p {}",
        registry_robot.registry, registry_robot.user, registry_robot.pass
    );

    Ok(())
}
