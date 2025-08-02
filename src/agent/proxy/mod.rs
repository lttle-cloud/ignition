pub mod proto;
pub mod tls;

use std::{collections::HashSet, convert::Infallible, sync::Arc, time::Duration};

use anyhow::{Result, bail};
use papaya::HashMap;
use rustls::{ServerConfig, sign::CertifiedKey};
use tokio::{io::AsyncWriteExt, net::TcpListener, spawn, task::JoinHandle};
use tokio_rustls::TlsAcceptor;
use tracing::{info, warn};

use crate::agent::{
    machine::MachineAgent,
    proxy::{proto::SniffedProtocol, tls::ProxyTlsCertResolver},
};

#[derive(Debug, Clone)]
pub struct ProxyAgentConfig {
    pub external_bind_address: String,
    pub evergreen_external_ports: Vec<u16>,
    pub default_tls_cert_path: String,
    pub default_tls_key_path: String,
}

pub struct ProxyAgent {
    config: ProxyAgentConfig,
    machine_agent: Arc<MachineAgent>,
    bindings: Arc<HashMap<String, ProxyBinding>>,
    cert_pool: Arc<HashMap<String, Arc<CertifiedKey>>>,
    default_cert: Arc<CertifiedKey>,
    tls_cert_resolver: Arc<ProxyTlsCertResolver>,
    tls_acceptor: Arc<TlsAcceptor>,
    servers: HashMap<(String, u16), ProxyServer>,
}

struct ProxyServer {
    address: String,
    port: u16,
    task: JoinHandle<Result<()>>,
    proxy_mode: ProxyServerMode,
    extensions: Vec<ProxyServerExtension>,
}

enum ProxyServerMode {
    Internal,
    External,
}

enum ProxyServerExtension {
    PostgresTls,
}

#[derive(Clone, Debug)]
pub struct ProxyBinding {
    pub target_network_tag: String,
    pub target_port: u16,
    pub mode: BindingMode,
}

#[derive(Clone, Debug)]
pub enum BindingMode {
    Internal {
        service_ip: String,
        service_port: u16,
    },
    External {
        port: u16,
        routing: ExternalBindingRouting,
    },
}

#[derive(Clone, Debug)]
pub enum ExternalBindingRouting {
    HttpHostHeader { host: String },
    TlsSni { host: String },
}

impl ProxyBinding {
    pub fn proxy_server_key(&self, config: &ProxyAgentConfig) -> (String, u16) {
        match &self.mode {
            BindingMode::Internal {
                service_ip,
                service_port,
            } => (service_ip.clone(), *service_port),
            BindingMode::External { port, .. } => (config.external_bind_address.clone(), *port),
        }
    }
}

impl ProxyAgent {
    pub async fn new(
        config: ProxyAgentConfig,
        machine_agent: Arc<MachineAgent>,
    ) -> Result<Arc<Self>> {
        info!(
            "Creating new proxy agent with external bind address: {}",
            config.external_bind_address
        );

        let tls_server_config_builder = ServerConfig::builder().with_no_client_auth();

        let default_cert = tls::load_cert_from_disk(
            &config.default_tls_cert_path,
            &config.default_tls_key_path,
            &tls_server_config_builder.crypto_provider(),
        )
        .await?;

        let tls_cert_resolver = Arc::new(ProxyTlsCertResolver::new(
            Arc::new(HashMap::new()),
            default_cert.clone(),
            tls_server_config_builder.crypto_provider().clone(),
        ));

        let tls_server_config =
            tls_server_config_builder.with_cert_resolver(tls_cert_resolver.clone());

        let tls_acceptor = Arc::new(TlsAcceptor::from(Arc::new(tls_server_config)));

        let agent = Arc::new(Self {
            config,
            machine_agent,
            bindings: Arc::new(HashMap::new()),
            servers: HashMap::new(),
            cert_pool: Arc::new(HashMap::new()),
            default_cert,
            tls_cert_resolver,
            tls_acceptor,
        });

        info!("Proxy agent created successfully");
        Ok(agent)
    }

    pub async fn set_binding(&self, binding_name: &str, binding: ProxyBinding) -> Result<()> {
        info!(
            "Setting binding '{}' with target network tag: {}",
            binding_name, binding.target_network_tag
        );

        let bindings = self.bindings.pin();
        let previous_binding = bindings.remove(binding_name);
        bindings.insert(binding_name.to_string(), binding);

        if let Err(e) = self.evaluate_bindings().await {
            if let Some(previous_binding) = previous_binding {
                warn!("Failed to set binding {binding_name}: {e}, reverting to previous binding");
                bindings.insert(binding_name.to_string(), previous_binding.clone());
            }

            return Err(e);
        };

        info!("Successfully set binding '{}'", binding_name);
        Ok(())
    }

    pub async fn remove_binding(&self, binding_name: &str) -> Result<()> {
        info!("Removing binding '{}'", binding_name);

        let bindings = self.bindings.pin();
        let previous_binding = bindings.remove(binding_name);

        if let Err(e) = self.evaluate_bindings().await {
            if let Some(previous_binding) = previous_binding {
                warn!(
                    "Failed to remove binding {binding_name}: {e}, reverting to previous binding"
                );
                bindings.insert(binding_name.to_string(), previous_binding.clone());
            }

            return Err(e);
        };

        info!("Successfully removed binding '{}'", binding_name);
        Ok(())
    }

    async fn evaluate_bindings(&self) -> Result<()> {
        info!("Evaluating proxy bindings");

        let mut server_keys_set = HashSet::new();
        let bindings = self.bindings.pin();

        for (_, binding) in bindings.iter() {
            let server_key = binding.proxy_server_key(&self.config);
            server_keys_set.insert(server_key);
        }

        let servers = self.servers.pin();
        for (server_key, _) in servers.iter() {
            if !server_keys_set.contains(server_key) {
                info!(
                    "Stopping server for {}:{} (no longer needed)",
                    server_key.0, server_key.1
                );
                let server = servers.remove(server_key);
                if let Some(server) = server {
                    server.task.abort();
                }
            }
        }

        for (_, binding) in bindings.iter() {
            let server_key = binding.proxy_server_key(&self.config);
            if !servers.contains_key(&server_key) {
                info!("Starting new server for {}:{}", server_key.0, server_key.1);

                let proxy_mode = match &binding.mode {
                    BindingMode::Internal { .. } => ProxyServerMode::Internal,
                    BindingMode::External { .. } => ProxyServerMode::External,
                };

                let task_server_key = server_key.clone();
                let task_machine_agent = self.machine_agent.clone();
                let task_bindings = self.bindings.clone();
                let task_tls_acceptor = self.tls_acceptor.clone();
                let task_binding = binding.clone();

                let task = match proxy_mode {
                    ProxyServerMode::Internal => spawn(async move {
                        internal_listener(
                            format!("{}:{}", task_server_key.0, task_server_key.1),
                            task_machine_agent,
                            task_binding,
                        )
                        .await?;

                        Ok(())
                    }),
                    ProxyServerMode::External => spawn(async move {
                        external_listener(
                            format!("{}:{}", task_server_key.0, task_server_key.1),
                            task_machine_agent,
                            task_bindings,
                            task_tls_acceptor,
                        )
                        .await?;

                        Ok(())
                    }),
                };

                let server = ProxyServer {
                    address: server_key.0.clone(),
                    port: server_key.1,
                    task,
                    proxy_mode,
                    extensions: Vec::new(),
                };

                servers.insert(server_key, server);
            }
        }

        info!("Binding evaluation completed");
        Ok(())
    }
}

async fn external_listener(
    addr: String,
    machine_agent: Arc<MachineAgent>,
    bindings: Arc<HashMap<String, ProxyBinding>>,
    tls_acceptor: Arc<TlsAcceptor>,
) -> Result<Infallible> {
    info!("Starting external listener on {}", addr);
    let listener = TcpListener::bind(addr).await?;

    loop {
        let (mut stream, _) = listener.accept().await?;
        let bindings = bindings.clone();
        let machine_agent = machine_agent.clone();
        let tls_acceptor = tls_acceptor.clone();

        spawn(async move {
            let protocol = proto::sniff_protocol(&mut stream).await?;

            match protocol {
                SniffedProtocol::Unknown => {
                    warn!("Received connection with unknown protocol");
                    bail!("Unknown protocol");
                }
                SniffedProtocol::Http => {
                    info!("Handling HTTP connection");
                    let (target_host, head) = proto::extract_http_host(&mut stream).await?;
                    let binding = {
                        let bindings = bindings.pin_owned();
                        bindings
                            .values()
                            .find(|b| match &b.mode {
                                BindingMode::External { routing, .. } => match routing {
                                    ExternalBindingRouting::HttpHostHeader { host }
                                        if *host == target_host =>
                                    {
                                        true
                                    }
                                    _ => false,
                                },
                                _ => false,
                            })
                            .cloned()
                    };

                    let Some(binding) = binding else {
                        warn!("No binding found for HTTP host: {}", target_host);
                        bail!("No binding found for host {target_host}");
                    };

                    let Some(machine) = machine_agent
                        .get_machine_by_network_tag(&binding.target_network_tag)
                        .await
                    else {
                        warn!(
                            "No machine found for network tag: {}",
                            binding.target_network_tag
                        );
                        bail!(
                            "No machine found for network tag {}",
                            binding.target_network_tag
                        );
                    };

                    let mut machine_connection = machine
                        .get_connection(
                            binding.target_port,
                            Duration::from_secs(5), // inactivity timeout
                        )
                        .await?;

                    info!(
                        "Proxying HTTP connection from {} to machine on port {}",
                        target_host, binding.target_port
                    );

                    machine_connection.upstream_socket.write_all(&head).await?;
                    machine_connection.proxy_from_client(stream).await?;
                }
                SniffedProtocol::Tls => {
                    info!("Handling TLS connection");
                    let tls_stream = tls_acceptor.accept(&mut stream).await?;
                    let (_tcp_stream, server_conn) = tls_stream.get_ref();

                    let Some(server_name) = server_conn.server_name() else {
                        warn!("No server name in TLS connection");
                        bail!("No server name in TLS connection");
                    };

                    let binding = {
                        let bindings = bindings.pin_owned();
                        bindings
                            .values()
                            .find(|b| match &b.mode {
                                BindingMode::External { routing, .. } => match routing {
                                    ExternalBindingRouting::TlsSni { host }
                                        if *host == server_name =>
                                    {
                                        true
                                    }
                                    _ => false,
                                },
                                _ => false,
                            })
                            .cloned()
                    };

                    let Some(binding) = binding else {
                        warn!("No binding found for TLS server name: {}", server_name);
                        bail!("No binding found for server name {server_name}");
                    };

                    let Some(machine) = machine_agent
                        .get_machine_by_network_tag(&binding.target_network_tag)
                        .await
                    else {
                        warn!(
                            "No machine found for network tag: {}",
                            binding.target_network_tag
                        );
                        bail!(
                            "No machine found for network tag {}",
                            binding.target_network_tag
                        );
                    };

                    let mut machine_connection = machine
                        .get_connection(
                            binding.target_port,
                            Duration::from_secs(5), // inactivity timeout
                        )
                        .await?;

                    info!(
                        "Proxying TLS connection from {} to machine on port {}",
                        server_name, binding.target_port
                    );
                    machine_connection.proxy_from_tls_client(tls_stream).await?;
                }
            }

            Ok(())
        });
    }
}

async fn internal_listener(
    addr: String,
    machine_agent: Arc<MachineAgent>,
    binding: ProxyBinding,
) -> Result<Infallible> {
    info!(
        "Starting internal listener on {} for network tag: {}",
        addr, binding.target_network_tag
    );
    let listener = TcpListener::bind(addr).await?;

    loop {
        let (mut stream, _) = listener.accept().await?;
        let machine_agent = machine_agent.clone();
        let binding = binding.clone();

        spawn(async move {
            let Some(machine) = machine_agent
                .get_machine_by_network_tag(&binding.target_network_tag)
                .await
            else {
                warn!(
                    "No machine found for network tag: {}",
                    binding.target_network_tag
                );
                bail!(
                    "No machine found for network tag {}",
                    binding.target_network_tag
                );
            };

            let mut machine_connection = machine
                .get_connection(
                    binding.target_port,
                    Duration::from_secs(5), // inactivity timeout
                )
                .await?;

            info!(
                "Proxying internal connection to machine on port {}",
                binding.target_port
            );
            machine_connection.proxy_from_client(stream).await?;

            Ok(())
        });
    }
}
