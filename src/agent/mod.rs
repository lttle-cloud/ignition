pub mod data;
pub mod image;
pub mod machine;
pub mod net;
pub mod volume;

use std::sync::Arc;

use anyhow::Result;

use crate::{
    agent::{
        net::{NetAgent, NetAgentConfig},
        volume::{VolumeAgent, VolumeAgentConfig},
    },
    machinery::store::Store,
};

#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub store_path: String,
    pub net_config: NetAgentConfig,
    pub volume_config: VolumeAgentConfig,
}

pub struct Agent {
    config: AgentConfig,
    store: Arc<Store>,
    net: Arc<NetAgent>,
    volume: Arc<VolumeAgent>,
}

impl Agent {
    pub async fn new(config: AgentConfig) -> Result<Self> {
        let store = Arc::new(Store::new(&config.store_path).await?);

        let net = Arc::new(NetAgent::new(config.net_config.clone(), store.clone()).await?);

        let volume = Arc::new(VolumeAgent::new(config.volume_config.clone(), store.clone()).await?);

        Ok(Self {
            config,
            store,
            net,
            volume,
        })
    }

    pub fn net(&self) -> &NetAgent {
        &self.net
    }

    pub fn volume(&self) -> &VolumeAgent {
        &self.volume
    }
}
