pub mod data;
pub mod image;
pub mod machine;
pub mod net;
pub mod volume;

use std::sync::Arc;

use anyhow::Result;

use crate::{
    agent::{
        image::{ImageAgent, ImageAgentConfig},
        machine::{MachineAgent, MachineAgentConfig},
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
    pub image_config: ImageAgentConfig,
    pub machine_config: MachineAgentConfig,
}

pub struct Agent {
    config: AgentConfig,
    store: Arc<Store>,
    net: Arc<NetAgent>,
    volume: Arc<VolumeAgent>,
    image: Arc<ImageAgent>,
    machine: Arc<MachineAgent>,
}

impl Agent {
    pub async fn new(config: AgentConfig) -> Result<Self> {
        let store = Arc::new(Store::new(&config.store_path).await?);

        let net = Arc::new(NetAgent::new(config.net_config.clone(), store.clone()).await?);
        let volume = Arc::new(VolumeAgent::new(config.volume_config.clone(), store.clone()).await?);

        let image = Arc::new(
            ImageAgent::new(config.image_config.clone(), store.clone(), volume.clone()).await?,
        );
        let machine = Arc::new(MachineAgent::new(config.machine_config.clone()));

        Ok(Self {
            config,
            store,
            net,
            volume,
            image,
            machine,
        })
    }

    pub fn net(&self) -> &NetAgent {
        &self.net
    }

    pub fn volume(&self) -> &VolumeAgent {
        &self.volume
    }

    pub fn image(&self) -> &ImageAgent {
        &self.image
    }

    pub fn machine(&self) -> &MachineAgent {
        &self.machine
    }
}
