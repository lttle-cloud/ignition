pub mod config;
mod handler;

use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
};

use anyhow::{Result, bail};
use hickory_proto::{
    op::{Header, MessageType, OpCode, ResponseCode},
    rr::{DNSClass, Name, RData, Record, RecordType},
};
use hickory_server::{
    ServerFuture,
    authority::MessageResponseBuilder,
    server::{Request, RequestHandler, ResponseHandler, ResponseInfo},
};
use papaya::HashMap as PapayaHashMap;
use tokio::{net::UdpSocket, spawn, sync::Mutex, task::JoinHandle};
use tracing::{debug, error, info};

use crate::{
    agent::{dns::config::DnsAgentConfig, machine::MachineAgent, net::NetAgent},
    constants::DEFAULT_AGENT_TENANT,
    machinery::store::Store,
    repository::Repository,
    resources::metadata::Metadata,
};

pub struct DnsAgent {
    config: DnsAgentConfig,
    store: Arc<Store>,
    net_agent: Arc<NetAgent>,
    machine_agent: Arc<MachineAgent>,
    server_task: Arc<Mutex<Option<JoinHandle<()>>>>,
}

struct DnsHandler {
    store: Arc<Store>,
    machine_agent: Arc<MachineAgent>,
    default_ttl: u32,
    service_cache: Arc<PapayaHashMap<String, ServiceDnsEntry>>,
}

#[derive(Clone, Debug)]
struct ServiceDnsEntry {
    service_ip: Option<String>,
    target_machine: String,
    target_namespace: String,
    port: u16,
}

impl DnsAgent {
    pub async fn new(
        config: DnsAgentConfig,
        store: Arc<Store>,
        net_agent: Arc<NetAgent>,
        machine_agent: Arc<MachineAgent>,
    ) -> Result<Arc<Self>> {
        Ok(Arc::new(Self {
            config,
            store,
            net_agent,
            machine_agent,
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
            store: self.store.clone(),
            machine_agent: self.machine_agent.clone(),
            default_ttl: self.config.default_ttl,
            service_cache: Arc::new(PapayaHashMap::new()),
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
