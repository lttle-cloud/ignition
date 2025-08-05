pub mod config;
mod handler;

use std::{net::SocketAddr, sync::Arc};

use anyhow::{Result, bail};
use hickory_server::ServerFuture;
use tokio::{net::UdpSocket, spawn, sync::Mutex, task::JoinHandle};
use tracing::{error, info};

use crate::{
    agent::{dns::config::DnsAgentConfig, machine::MachineAgent, net::NetAgent},
    repository::Repository,
};

pub struct DnsAgent {
    config: DnsAgentConfig,
    net_agent: Arc<NetAgent>,
    machine_agent: Arc<MachineAgent>,
    repository: Arc<Repository>,
    server_task: Arc<Mutex<Option<JoinHandle<()>>>>,
}

struct DnsHandler {
    net_agent: Arc<NetAgent>,
    machine_agent: Arc<MachineAgent>,
    repository: Arc<Repository>,
    default_ttl: u32,
}


impl DnsAgent {
    pub async fn new(
        config: DnsAgentConfig,
        net_agent: Arc<NetAgent>,
        machine_agent: Arc<MachineAgent>,
        repository: Arc<Repository>,
    ) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            config,
            net_agent,
            machine_agent,
            repository,
            server_task: Arc::new(Mutex::new(None)),
        }))
    }

    pub async fn start(&self) -> Result<()> {
        let mut server_task = self.server_task.lock().await;
        if server_task.is_some() {
            bail!("DNS server already running");
        }

        let bind_addr = self.config.bind_address.parse::<SocketAddr>()?;
        info!("Starting DNS server on {}", bind_addr);

        let handler = DnsHandler {
            net_agent: self.net_agent.clone(),
            machine_agent: self.machine_agent.clone(),
            repository: self.repository.clone(),
            default_ttl: self.config.default_ttl,
        };

        let mut server = ServerFuture::new(handler);

        server.register_socket(UdpSocket::bind(bind_addr).await?);

        let task = spawn(async move {
            match server.block_until_done().await {
                Ok(_) => info!("DNS server stopped"),
                Err(e) => error!("DNS server error: {}", e),
            }
        });

        *server_task = Some(task);
        info!("DNS server started successfully on {}", bind_addr);

        Ok(())
    }

    pub async fn stop(&self) -> Result<()> {
        let mut server_task = self.server_task.lock().await;
        if let Some(task) = server_task.take() {
            info!("Stopping DNS server");
            task.abort();
        }
        Ok(())
    }
}
