use ignition_client::ignition_proto::{admin::user::Status, util::Empty};
use util::{
    result::{bail, Result},
    tracing::info,
};

use crate::{client::get_client, config::Config};

pub async fn run_who_am_i(config: Config) -> Result<()> {
    let client = get_client(&config).await?;

    let response = client.user().who_am_i(Empty::default()).await?.into_inner();

    let Some(user) = response.user else {
        bail!("No user found");
    };

    let user_status: Status = user.status.try_into()?;

    info!("API: {}", config.api_addr);
    info!(
        "User: {} ({}) {}",
        user.name,
        user.id,
        user_status.as_str_name()
    );

    Ok(())
}
