pub mod data;
pub mod dns;
pub mod image;
pub mod job;
pub mod machine;
pub mod net;
pub mod proxy;
pub mod volume;

use std::sync::{Arc, Weak};

use anyhow::Result;

use crate::{
    agent::{
        dns::{DnsAgent, config::DnsAgentConfig},
        image::{ImageAgent, ImageAgentConfig},
        job::JobAgent,
        machine::{MachineAgent, MachineAgentConfig},
        net::{NetAgent, NetAgentConfig},
        proxy::{ProxyAgent, ProxyAgentConfig},
        volume::{VolumeAgent, VolumeAgentConfig},
    },
    controller::scheduler::Scheduler,
    machinery::store::Store,
    repository::Repository,
};

#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub store_path: String,
    pub net_config: NetAgentConfig,
    pub volume_config: VolumeAgentConfig,
    pub image_config: ImageAgentConfig,
    pub machine_config: MachineAgentConfig,
    pub proxy_config: ProxyAgentConfig,
    pub dns_config: DnsAgentConfig,
}

pub struct Agent {
    job: Arc<JobAgent>,
    net: Arc<NetAgent>,
    volume: Arc<VolumeAgent>,
    image: Arc<ImageAgent>,
    machine: Arc<MachineAgent>,
    proxy: Arc<ProxyAgent>,
    dns: Arc<DnsAgent>,
}

impl Agent {
    pub async fn new(
        config: AgentConfig,
        scheduler: Weak<Scheduler>,
        repository: Arc<Repository>,
    ) -> Result<Self> {
        let store = Arc::new(Store::new(&config.store_path).await?);

        let net = Arc::new(NetAgent::new(config.net_config.clone(), store.clone()).await?);
        let volume = Arc::new(VolumeAgent::new(config.volume_config.clone(), store.clone()).await?);

        let image = Arc::new(
            ImageAgent::new(config.image_config.clone(), store.clone(), volume.clone()).await?,
        );
        let machine =
            Arc::new(MachineAgent::new(config.machine_config.clone(), scheduler.clone()).await?);

        let proxy = ProxyAgent::new(config.proxy_config.clone(), machine.clone()).await?;

        let dns = DnsAgent::new(
            config.dns_config.clone(),
            net.clone(),
            machine.clone(),
            repository,
        )
        .await?;

        // Start the DNS server
        dns.start().await?;

        Ok(Self {
            job: Arc::new(JobAgent::new(scheduler)),
            net,
            volume,
            image,
            machine,
            proxy,
            dns,
        })
    }

    pub fn job(&self) -> Arc<JobAgent> {
        self.job.clone()
    }

    pub fn net(&self) -> Arc<NetAgent> {
        self.net.clone()
    }

    pub fn volume(&self) -> Arc<VolumeAgent> {
        self.volume.clone()
    }

    pub fn image(&self) -> Arc<ImageAgent> {
        self.image.clone()
    }

    pub fn machine(&self) -> Arc<MachineAgent> {
        self.machine.clone()
    }

    pub fn proxy(&self) -> Arc<ProxyAgent> {
        self.proxy.clone()
    }

    pub fn dns(&self) -> Arc<DnsAgent> {
        self.dns.clone()
    }
}
