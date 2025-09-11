use anyhow::Result;
use atty::Stream;
use clap::Args;

use crate::{client::get_api_client, config::Config, ui::message::message_info};

#[derive(Args)]
pub struct DockerLoginArgs {}

pub async fn run_docker_login(config: &Config, _args: DockerLoginArgs) -> Result<()> {
    let api_client = get_api_client(config.try_into()?);
    let me = api_client.core().me().await?;
    let registry_robot = api_client.core().get_registry_robot().await?;

    if !atty::is(Stream::Stdout) {
        print!("{}", registry_robot.pass);

        return Ok(());
    }

    message_info(format!(
        "You are logged in as {} (tenant: {})",
        me.sub, me.tenant
    ));

    message_info(format!(
        "To login to the registry, pipe the output of this command straight to docker like this:\n",
    ));

    println!(
        "lttle docker login | docker login {} -u {} --password-stdin",
        registry_robot.registry, registry_robot.user
    );

    Ok(())
}
