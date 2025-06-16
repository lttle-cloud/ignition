use ignition_client::{Client, ClientConfig, PrivilegedClient};
use util::result::{bail, Result};

use crate::config::Config;

pub async fn get_admin_client(config: Config) -> Result<PrivilegedClient> {
    let Some(token) = config.admin_token else {
        bail!("No admin token found in config");
    };

    let client_config = ClientConfig {
        addr: format!("tcp://{}", config.api_addr),
        token,
    };
    let admin_client = PrivilegedClient::new(client_config).await?;

    Ok(admin_client)
}

pub async fn get_client(config: Config) -> Result<Client> {
    let Some(token) = config.token else {
        bail!("No user token found in config");
    };

    let client_config = ClientConfig {
        addr: format!("tcp://{}", config.api_addr),
        token,
    };
    let client = Client::new(client_config).await?;
    Ok(client)
}
