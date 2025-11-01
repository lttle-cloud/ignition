pub mod build;
pub mod certificate;
pub mod data;
pub mod dns;
pub mod image;
pub mod job;
pub mod logs;
pub mod machine;
pub mod net;
pub mod openai;
pub mod port_allocator;
pub mod proxy;
pub mod tracker;
pub mod volume;

use std::sync::{Arc, Weak};

use anyhow::{Result, bail};

use crate::{
    agent::{
        build::{BuildAgent, BuildAgentConfig},
        certificate::{CertificateAgent, config::CertificateAgentConfig},
        dns::{DnsAgent, config::DnsAgentConfig},
        image::{ImageAgent, ImageAgentConfig},
        job::JobAgent,
        logs::{LogsAgent, LogsAgentConfig},
        machine::{MachineAgent, MachineAgentConfig},
        net::{NetAgent, NetAgentConfig},
        openai::{OpenAIAgent, OpenAIAgentConfig},
        port_allocator::{PortAllocator, TcpPortRange},
        proxy::{ProxyAgent, ProxyAgentConfig},
        tracker::TrackerAgent,
        volume::{VolumeAgent, VolumeAgentConfig},
    },
    api::auth::AuthHandler,
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
    pub cert_config: CertificateAgentConfig,
    pub logs_config: LogsAgentConfig,
    pub openai_config: Option<OpenAIAgentConfig>,
    pub build_config: Option<BuildAgentConfig>,
    pub tcp_port_range: Option<TcpPortRange>,
}

pub struct Agent {
    job: Arc<JobAgent>,
    net: Arc<NetAgent>,
    volume: Arc<VolumeAgent>,
    image: Arc<ImageAgent>,
    machine: Arc<MachineAgent>,
    proxy: Arc<ProxyAgent>,
    dns: Arc<DnsAgent>,
    certificate: Arc<CertificateAgent>,
    logs: Arc<LogsAgent>,
    tracker: Arc<TrackerAgent>,
    port_allocator: Arc<PortAllocator>,
    openai: Option<Arc<OpenAIAgent>>,
    build: Option<Arc<BuildAgent>>,
}

impl Agent {
    pub async fn new(
        config: AgentConfig,
        scheduler: Weak<Scheduler>,
        repository: Arc<Repository>,
        auth_handler: Arc<AuthHandler>,
    ) -> Result<Self> {
        let store = Arc::new(Store::new(&config.store_path).await?);

        let net = Arc::new(NetAgent::new(config.net_config.clone(), store.clone()).await?);
        let volume = Arc::new(VolumeAgent::new(config.volume_config.clone(), store.clone()).await?);

        let image = Arc::new(
            ImageAgent::new(
                config.image_config.clone(),
                store.clone(),
                volume.clone(),
                auth_handler.clone(),
            )
            .await?,
        );
        let machine =
            Arc::new(MachineAgent::new(config.machine_config.clone(), scheduler.clone()).await?);

        let certificate = CertificateAgent::new(store.clone(), config.cert_config.clone()).await?;

        let proxy = ProxyAgent::new(
            config.proxy_config.clone(),
            machine.clone(),
            certificate.clone(),
        )
        .await?;

        let dns = DnsAgent::new(config.dns_config.clone(), net.clone(), repository).await?;

        let logs = Arc::new(LogsAgent::new(config.logs_config.clone()));

        let tracker = Arc::new(TrackerAgent::new(store.clone()));

        let port_allocator = Arc::new(PortAllocator::new(
            store.clone(),
            tracker.clone(),
            config.tcp_port_range.clone(),
        ));

        let build = match config.build_config {
            Some(config) => Some(Arc::new(BuildAgent::new(config)?)),
            None => None,
        };

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
            certificate,
            logs,
            tracker,
            port_allocator,
            openai: config
                .openai_config
                .map(|config| Arc::new(OpenAIAgent::new(config))),
            build,
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

    pub fn certificate(&self) -> Arc<CertificateAgent> {
        self.certificate.clone()
    }

    pub fn logs(&self) -> Arc<LogsAgent> {
        self.logs.clone()
    }

    pub fn tracker(&self) -> Arc<TrackerAgent> {
        self.tracker.clone()
    }

    pub fn port_allocator(&self) -> Arc<PortAllocator> {
        self.port_allocator.clone()
    }

    pub fn openai(&self) -> Result<Arc<OpenAIAgent>> {
        if let Some(openai) = &self.openai {
            Ok(openai.clone())
        } else {
            bail!("OpenAI agent not configured")
        }
    }

    pub fn build(&self) -> Result<Arc<BuildAgent>> {
        if let Some(build) = &self.build {
            Ok(build.clone())
        } else {
            bail!("Build agent not configured")
        }
    }
}
