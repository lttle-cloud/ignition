use anyhow::Result;
use ignition::api_client::{ApiClient, ApiClientConfig};

use crate::config::Config;

pub async fn get_api_client(config: &Config) -> Result<ApiClient> {
    let api_client_config: ApiClientConfig = config.try_into()?;
    let client = ApiClient::new(api_client_config);

    Ok(client)
}
