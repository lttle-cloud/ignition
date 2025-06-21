pub mod data;
pub mod net;

use std::sync::Arc;

use anyhow::Result;

use crate::{
    agent::net::{NetAgent, NetAgentConfig},
    machinery::store::Store,
};

#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub store_path: String,
    pub net_config: NetAgentConfig,
}

pub struct Agent {
    config: AgentConfig,
    store: Arc<Store>,
    net: Arc<NetAgent>,
}

impl Agent {
    pub async fn new(config: AgentConfig) -> Result<Self> {
        let store = Arc::new(Store::new(&config.store_path).await?);

        let net = Arc::new(NetAgent::new(config.net_config.clone(), store.clone()).await?);

        Ok(Self { config, store, net })
    }

    pub fn net(&self) -> &NetAgent {
        &self.net
    }
}
