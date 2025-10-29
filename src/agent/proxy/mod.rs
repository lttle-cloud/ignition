pub mod proto;
pub mod tls;

use std::{collections::HashSet, convert::Infallible, str::FromStr, sync::Arc, time::Duration};

use anyhow::{Result, bail};
use axum::http::HeaderValue;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Method, Request, StatusCode, Uri, Version, service::service_fn, upgrade::Upgraded};
use hyper_util::{
    client::legacy::{Client, connect::HttpConnector},
    rt::{TokioExecutor, TokioIo},
    server::conn::auto::Builder,
};
use papaya::HashMap;
use rustls::{ServerConfig, sign::CertifiedKey};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    spawn,
    task::JoinHandle,
};
use tokio_rustls::TlsAcceptor;
use tracing::{info, warn};

use crate::agent::{
    certificate::CertificateAgent,
    machine::{
        MachineAgent,
        machine::{Machine, TrafficAwareConnection},
    },
    proxy::{proto::SniffedProtocol, tls::ProxyTlsCertResolver},
};

#[derive(Debug, Clone)]
pub struct ProxyAgentConfig {
    pub external_bind_address: String,
    pub evergreen_external_ports: Vec<u16>,
    pub blacklisted_external_ports: Vec<u16>,
    pub default_tls_cert_path: String,
    pub default_tls_key_path: String,
    pub blacklisted_seo_domain: String,
}

#[allow(unused)]
pub struct ProxyAgent {
    config: ProxyAgentConfig,
    machine_agent: Arc<MachineAgent>,
    bindings: Arc<HashMap<String, ProxyBinding>>,
    cert_pool: Arc<HashMap<String, Arc<CertifiedKey>>>,
    default_cert: Arc<CertifiedKey>,
    tls_cert_resolver: Arc<ProxyTlsCertResolver>,
    tls_acceptor: Arc<TlsAcceptor>,
    servers: HashMap<(String, u16), ProxyServer>,
    certificate_agent: Arc<CertificateAgent>,
}

#[allow(unused)]
struct ProxyServer {
    address: String,
    port: u16,
    task: JoinHandle<Result<()>>,
    proxy_mode: ProxyServerMode,
}

enum ProxyServerMode {
    Internal,
    External,
}

#[derive(Clone, Debug)]
pub struct ProxyBinding {
    pub target_network_tag: String,
    pub target_port: u16,
    pub mode: BindingMode,
    pub inactivity_timeout: Option<Duration>,
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
    HttpHostHeader {
        host: String,
    },
    TlsSni {
        host: String,
        nested_protocol: ExternnalBindingRoutingTlsNestedProtocol,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExternnalBindingRoutingTlsNestedProtocol {
    Http,
    Unknown,
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

    pub fn public_host(&self) -> Option<String> {
        let host = match &self.mode {
            BindingMode::External { routing, port } => match routing {
                ExternalBindingRouting::HttpHostHeader { host } => Some((host.clone(), *port)),
                ExternalBindingRouting::TlsSni { host, .. } => Some((host.clone(), *port)),
            },
            _ => None,
        };

        if let Some((host, port)) = host {
            if port == 443 || port == 80 {
                return Some(host);
            }

            return Some(format!("{}:{}", host, port));
        }

        None
    }
}

impl ProxyAgent {
    pub async fn new(
        config: ProxyAgentConfig,
        machine_agent: Arc<MachineAgent>,
        certificate_agent: Arc<CertificateAgent>,
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
            std::path::PathBuf::from(certificate_agent.config().certs_base_dir.clone()),
        ));

        let mut tls_server_config =
            tls_server_config_builder.with_cert_resolver(tls_cert_resolver.clone());

        tls_server_config.alpn_protocols =
            vec![b"postgresql".to_vec(), b"h2".to_vec(), b"http/1.1".to_vec()];

        let tls_acceptor = Arc::new(TlsAcceptor::from(Arc::new(tls_server_config)));

        let agent = Arc::new(Self {
            config: config.clone(),
            machine_agent,
            bindings: Arc::new(HashMap::new()),
            servers: HashMap::new(),
            cert_pool: Arc::new(HashMap::new()),
            default_cert,
            tls_cert_resolver,
            tls_acceptor,
            certificate_agent,
        });

        for port in config.evergreen_external_ports {
            info!("Starting server for evergreen port {}", port);
            agent.start_server(
                &ProxyBinding {
                    target_network_tag: format!("internal-evergreen-{}", port),
                    target_port: port,
                    mode: BindingMode::External {
                        port,
                        routing: ExternalBindingRouting::HttpHostHeader {
                            host: format!("evergreen-{}.local", port),
                        },
                    },
                    inactivity_timeout: None,
                },
                (config.external_bind_address.clone(), port).into(),
            );
        }

        agent.evaluate_bindings().await?;

        info!("Proxy agent created successfully");
        Ok(agent)
    }

    pub fn config(&self) -> &ProxyAgentConfig {
        &self.config
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

        let evergreen_server_keys = self
            .config
            .evergreen_external_ports
            .iter()
            .map(|port| (self.config.external_bind_address.clone(), *port))
            .collect::<Vec<(String, u16)>>();

        let blacklisted_server_keys = self
            .config
            .blacklisted_external_ports
            .iter()
            .map(|port| (self.config.external_bind_address.clone(), *port))
            .collect::<Vec<(String, u16)>>();

        let mut server_keys_set = HashSet::new();
        let bindings = self.bindings.pin();

        for (_, binding) in bindings.iter() {
            let server_key = binding.proxy_server_key(&self.config);
            if blacklisted_server_keys.contains(&server_key) {
                info!(
                    "Skipping blacklisted server key: {:?} from binding: {}",
                    server_key, binding.target_network_tag
                );
                continue;
            }

            server_keys_set.insert(server_key);
        }

        let servers = self.servers.pin();
        for (server_key, _) in servers.iter() {
            if !server_keys_set.contains(server_key) && !evergreen_server_keys.contains(server_key)
            {
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
            let server_key: (String, u16) = binding.proxy_server_key(&self.config);
            if !servers.contains_key(&server_key) && !blacklisted_server_keys.contains(&server_key)
            {
                self.start_server(binding, server_key);
            }
        }

        info!("Binding evaluation completed");
        Ok(())
    }

    pub fn invalidate_cert_cache_for_domains(&self, domains: Vec<String>) {
        self.tls_cert_resolver
            .invalidate_cert_cache_for_domains(domains);
    }

    fn start_server(&self, binding: &ProxyBinding, server_key: (String, u16)) {
        let servers = self.servers.pin();

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
        let task_certificate_agent = self.certificate_agent.clone();
        let task_blacklisted_seo_domain = self.config.blacklisted_seo_domain.clone();

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
                    task_blacklisted_seo_domain,
                    task_tls_acceptor,
                    task_certificate_agent,
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
        };

        servers.insert(server_key, server);
    }
}

async fn proxy_websocket_upgrade(
    client_upgrade: Result<Upgraded, hyper::Error>,
    upstream_upgrade: Result<Upgraded, hyper::Error>,
) -> Result<()> {
    let mut client = match client_upgrade {
        Ok(upgraded) => TokioIo::new(upgraded),
        Err(e) => {
            warn!("Failed to upgrade client connection: {}", e);
            bail!("Failed to upgrade client connection");
        }
    };

    let mut upstream = match upstream_upgrade {
        Ok(upgraded) => TokioIo::new(upgraded),
        Err(e) => {
            warn!("Failed to upgrade upstream connection: {}", e);
            bail!("Failed to upgrade upstream connection");
        }
    };

    // Bidirectionally copy data between client and upstream
    match tokio::io::copy_bidirectional(&mut client, &mut upstream).await {
        Ok((client_to_upstream, upstream_to_client)) => {
            info!(
                "WebSocket connection closed. Bytes transferred - client->upstream: {}, upstream->client: {}",
                client_to_upstream, upstream_to_client
            );
        }
        Err(e) => {
            warn!("Error during WebSocket proxying: {}", e);
        }
    }

    Ok(())
}

async fn external_listener(
    addr: String,
    machine_agent: Arc<MachineAgent>,
    bindings: Arc<HashMap<String, ProxyBinding>>,
    blacklisted_seo_domain: String,
    tls_acceptor: Arc<TlsAcceptor>,
    certificate_agent: Arc<CertificateAgent>,
) -> Result<Infallible> {
    info!("Starting external listener on {}", addr);
    let listener = TcpListener::bind(addr).await?;

    loop {
        let (stream, _) = listener.accept().await?;
        let bindings = bindings.clone();
        let machine_agent = machine_agent.clone();
        let tls_acceptor = tls_acceptor.clone();
        let certificate_agent = certificate_agent.clone();
        let blacklisted_seo_domain = blacklisted_seo_domain.clone();

        spawn(async move {
            handle_external_connection(
                stream,
                bindings,
                machine_agent,
                blacklisted_seo_domain,
                tls_acceptor,
                certificate_agent,
            )
            .await
        });
    }
}

async fn handle_external_connection(
    mut stream: TcpStream,
    bindings: Arc<HashMap<String, ProxyBinding>>,
    machine_agent: Arc<MachineAgent>,
    blacklisted_seo_domain: String,
    tls_acceptor: Arc<TlsAcceptor>,
    certificate_agent: Arc<CertificateAgent>,
) -> Result<()> {
    let protocol = proto::sniff_protocol(&mut stream).await?;

    match protocol {
        SniffedProtocol::Unknown => {
            warn!("Received connection with unknown protocol");
            bail!("Unknown protocol");
        }
        SniffedProtocol::Http => {
            info!("Handling HTTP connection");
            handle_http_connection(
                stream,
                bindings,
                blacklisted_seo_domain,
                machine_agent,
                certificate_agent,
            )
            .await
        }
        SniffedProtocol::PgSsl => {
            info!("Handling PostgreSQL SSL connection");
            handle_pg_ssl_connection(
                tls_acceptor.clone(),
                stream,
                bindings,
                blacklisted_seo_domain,
                machine_agent,
            )
            .await
        }
        SniffedProtocol::Tls => {
            info!("Handling TLS connection");
            let tls_stream = tls_acceptor.accept(stream).await?;
            handle_tls_connection(tls_stream, bindings, blacklisted_seo_domain, machine_agent).await
        }
    }
}

async fn handle_http_connection(
    stream: TcpStream,
    bindings: Arc<HashMap<String, ProxyBinding>>,
    blacklisted_seo_domain: String,
    machine_agent: Arc<MachineAgent>,
    certificate_agent: Arc<CertificateAgent>,
) -> Result<()> {
    let client_ip = stream.peer_addr().ok();

    let io = TokioIo::new(stream);

    let svc = service_fn(move |mut req: Request<hyper::body::Incoming>| {
        let machine_agent = machine_agent.clone();
        let bindings = bindings.clone();
        let blacklisted_seo_domain = blacklisted_seo_domain.clone();
        let certificate_agent = certificate_agent.clone();

        let mut base = HttpConnector::new();
        base.enforce_http(true);

        let client = Client::builder(TokioExecutor::new()).build(base);

        async move {
            // Check if this is a WebSocket upgrade request
            let is_websocket_upgrade = req
                .headers()
                .get("upgrade")
                .and_then(|v| v.to_str().ok())
                .map(|v| v.eq_ignore_ascii_case("websocket"))
                .unwrap_or(false);

            // Get the upgrade future before consuming the request
            let client_upgrade = if is_websocket_upgrade {
                Some(hyper::upgrade::on(&mut req))
            } else {
                None
            };
            // Extract host from request headers
            let target_host = req
                .headers()
                .get("host")
                .and_then(|h| h.to_str().ok())
                .unwrap_or_default()
                .to_string();

            if req.uri().path().starts_with("/.well-known/acme-challenge/") {
                match certificate_agent.get_challenge_response(target_host.as_str()) {
                    Ok(Some(key_auth)) => {
                        return Ok(hyper::Response::new(
                            Full::new(Bytes::from(key_auth))
                                .map_err(|never| match never {})
                                .boxed(),
                        ));
                    }
                    Ok(None) => return Err("Challenge not found"),
                    Err(_) => return Err("Error retrieving challenge"),
                }
            }

            let Ok(binding) = find_http_binding(&bindings, &target_host) else {
                if let Ok((binding, _)) = find_tls_binding(&bindings, &target_host) {
                    if req.method() != Method::GET {
                        return Err("only GET requests are allowed to be redirected to HTTPS");
                    }

                    let Some(public_host) = binding.public_host() else {
                        return Err("no public host found for binding");
                    };

                    let path = req.uri().path();
                    let query = req
                        .uri()
                        .query()
                        .and_then(|q| Some(format!("?{}", q)))
                        .unwrap_or_default();

                    let new_uri = format!("https://{}{}{}", public_host, path, query);

                    let Ok(location) = HeaderValue::from_str(&new_uri) else {
                        return Err("failed to parse location");
                    };

                    let mut response = hyper::Response::new(
                        Full::new(Bytes::from(vec![]))
                            .map_err(|never| match never {})
                            .boxed(),
                    );
                    *response.status_mut() = StatusCode::TEMPORARY_REDIRECT;
                    response.headers_mut().insert("location", location);

                    return Ok(response);
                }

                return Err("failed to find binding for HTTP host");
            };

            let Ok(machine) = find_machine(&machine_agent, &binding.target_network_tag).await
            else {
                return Err("failed to find machine");
            };

            let machine_connection = match get_machine_connection(
                &machine,
                binding.target_port,
                binding.inactivity_timeout,
            )
            .await
            {
                Ok(conn) => conn,
                Err(e) => {
                    warn!(
                        "Failed to establish connection to machine service {}:{}: {}",
                        machine.config.network.ip_address, binding.target_port, e
                    );
                    return Err(
                        "failed to connect to machine service - service may be starting up",
                    );
                }
            };

            let upstream_uri = format!(
                "http://{}:{}",
                machine_connection.ip_address(),
                binding.target_port
            );
            info!(
                "Proxying HTTP connection from {} to {}",
                target_host, upstream_uri
            );

            let client = client.clone();
            let upstream_uri = upstream_uri.clone();

            let original_uri = req.uri();
            let path_and_query = original_uri
                .path_and_query()
                .map(|pq| pq.as_str())
                .unwrap_or("/");

            let new_uri = format!("{}{}", upstream_uri, path_and_query);
            *req.uri_mut() = Uri::from_str(&new_uri).expect("failed to parse uri");

            let headers = req.headers_mut();

            headers.remove("x-forwarded-proto");
            headers.append("x-forwarded-proto", HeaderValue::from_static("https"));

            if let Some(host) = binding.public_host().and_then(|h| h.parse().ok()) {
                headers.remove("host");
                headers.append("host", host);
            }

            if let Some(client_ip) =
                client_ip.and_then(|ip| ip.ip().to_string().parse::<HeaderValue>().ok())
            {
                headers.remove("x-forwarded-for");
                headers.append("x-forwarded-for", client_ip.clone());

                headers.remove("x-real-ip");
                headers.append("x-real-ip", client_ip);
            }

            *req.version_mut() = Version::HTTP_11;

            info!("Modified request URI: {:?}", req.uri());

            let Ok(mut response) = client.request(req).await else {
                return Err("failed to get response from origin");
            };

            if target_host.ends_with(&blacklisted_seo_domain) {
                response.headers_mut().append(
                    "X-Robots-Tag",
                    HeaderValue::from_static("noindex, nofollow"),
                );
            }

            // Handle WebSocket upgrade if this was an upgrade request
            if let Some(client_upgrade) = client_upgrade {
                if response.status() == StatusCode::SWITCHING_PROTOCOLS {
                    info!("WebSocket upgrade successful, proxying WebSocket connection");

                    // Get the upstream upgrade future
                    let upstream_upgrade = hyper::upgrade::on(&mut response);

                    // Spawn a task to handle the WebSocket proxying
                    spawn(async move {
                        if let Err(e) = proxy_websocket_upgrade(client_upgrade.await, upstream_upgrade.await).await {
                            warn!("Error proxying WebSocket: {}", e);
                        }
                    });
                }
            }

            Ok(response.map(|b| b.boxed()))
        }
    });

    if let Err(e) = Builder::new(TokioExecutor::new())
        .serve_connection_with_upgrades(io, svc)
        .await
    {
        warn!("Error proxying HTTP connection: {e}");
        bail!("Error proxying HTTP connection: {e}");
    }

    Ok(())
}

async fn handle_pg_ssl_connection(
    tls_acceptor: Arc<TlsAcceptor>,
    mut stream: TcpStream,
    bindings: Arc<HashMap<String, ProxyBinding>>,
    blacklisted_seo_domain: String,
    machine_agent: Arc<MachineAgent>,
) -> Result<()> {
    // read the SSLRequest message and accept the connection with handle_tls_connection
    let mut _throw_away_buffer = [0u8; 8];
    stream.read_exact(&mut _throw_away_buffer).await?;

    stream.write_all(b"S").await?;

    let tls_stream = tls_acceptor.accept(stream).await?;
    handle_tls_connection(tls_stream, bindings, blacklisted_seo_domain, machine_agent).await
}

async fn handle_https_connection(
    tls_stream: tokio_rustls::server::TlsStream<TcpStream>,
    bindings: Arc<HashMap<String, ProxyBinding>>,
    blacklisted_seo_domain: String,
    machine_agent: Arc<MachineAgent>,
    server_name: String,
) -> Result<()> {
    let client_ip = tls_stream.get_ref().0.peer_addr().ok();

    let io = TokioIo::new(tls_stream);

    let svc = service_fn(move |mut req: Request<hyper::body::Incoming>| {
        let machine_agent = machine_agent.clone();
        let bindings = bindings.clone();
        let blacklisted_seo_domain = blacklisted_seo_domain.clone();
        let server_name = server_name.clone();

        let mut base = HttpConnector::new();
        base.enforce_http(true);

        let client = Client::builder(TokioExecutor::new()).build(base);

        async move {
            // Check if this is a WebSocket upgrade request
            let is_websocket_upgrade = req
                .headers()
                .get("upgrade")
                .and_then(|v| v.to_str().ok())
                .map(|v| v.eq_ignore_ascii_case("websocket"))
                .unwrap_or(false);

            // Get the upgrade future before consuming the request
            let client_upgrade = if is_websocket_upgrade {
                Some(hyper::upgrade::on(&mut req))
            } else {
                None
            };
            let original_host = req.uri().host();
            let original_port = req.uri().port();

            let target_host = match original_host.clone() {
                Some(host) => match original_port {
                    Some(port) => {
                        if port == 80 || port == 443 {
                            host.to_string()
                        } else {
                            format!("{}:{}", host, port)
                        }
                    }
                    None => host.to_string(),
                },
                None => {
                    warn!("No host in request URI. Defaulting to {}", server_name);
                    server_name
                }
            };

            let Ok((binding, _)) = find_tls_binding(&bindings, &target_host) else {
                return Err("failed to find binding for HTTPS host");
            };

            let Ok(machine) = find_machine(&machine_agent, &binding.target_network_tag).await
            else {
                return Err("failed to find machine");
            };

            let machine_connection =
                match get_machine_connection(&machine, binding.target_port, None).await {
                    Ok(conn) => conn,
                    Err(e) => {
                        warn!(
                            "Failed to establish connection to machine service {}:{}: {}",
                            machine.config.network.ip_address, binding.target_port, e
                        );
                        return Err(
                            "failed to connect to machine service - service may be starting up",
                        );
                    }
                };

            let upstream_uri = format!(
                "http://{}:{}",
                machine_connection.ip_address(),
                binding.target_port
            );
            info!("Proxying HTTPS connection to {}", upstream_uri);

            let client = client.clone();
            let upstream_uri = upstream_uri.clone();

            let original_uri = req.uri();
            let path_and_query = original_uri
                .path_and_query()
                .map(|pq| pq.as_str())
                .unwrap_or("/");

            let new_uri = format!("{}{}", upstream_uri, path_and_query);
            *req.uri_mut() = Uri::from_str(&new_uri).expect("failed to parse uri");
            let headers = req.headers_mut();
            let existing_host = headers.get("host");
            info!("existing host: {:?}", existing_host);
            info!("setting host to: {:?}", binding.public_host());

            headers.remove("x-forwarded-proto");
            headers.append("x-forwarded-proto", HeaderValue::from_static("https"));

            if let Ok(host) = HeaderValue::from_str(&target_host) {
                headers.remove("host");
                headers.append("host", host.clone());
                headers.append("x-forwarded-host", host);
            }

            if let Some(client_ip) =
                client_ip.and_then(|ip| ip.ip().to_string().parse::<HeaderValue>().ok())
            {
                headers.remove("x-forwarded-for");
                headers.append("x-forwarded-for", client_ip.clone());

                headers.remove("x-real-ip");
                headers.append("x-real-ip", client_ip);
            }

            *req.version_mut() = Version::HTTP_11;

            info!("Modified request URI: {:?}", req.uri());

            let Ok(mut response) = client.request(req).await else {
                return Err("failed to get response from origin");
            };

            // TODO: Uncomment this when we have a stable HTTP -> HTTPS redirect
            // response.headers_mut().append("Strict-Transport-Security", HeaderValue::from_static("max-age=31536000; includeSubDomains; preload"));
            response.headers_mut().append(
                "Strict-Transport-Security",
                HeaderValue::from_static("max-age=86400"),
            );

            if target_host.ends_with(&blacklisted_seo_domain) {
                response.headers_mut().append(
                    "X-Robots-Tag",
                    HeaderValue::from_static("noindex, nofollow"),
                );
            }

            // Handle WebSocket upgrade if this was an upgrade request
            if let Some(client_upgrade) = client_upgrade {
                if response.status() == StatusCode::SWITCHING_PROTOCOLS {
                    info!("WebSocket upgrade successful over TLS, proxying WebSocket connection");

                    // Get the upstream upgrade future
                    let upstream_upgrade = hyper::upgrade::on(&mut response);

                    // Spawn a task to handle the WebSocket proxying
                    spawn(async move {
                        if let Err(e) = proxy_websocket_upgrade(client_upgrade.await, upstream_upgrade.await).await {
                            warn!("Error proxying WebSocket over TLS: {}", e);
                        }
                    });
                }
            }

            Ok(response)
        }
    });

    if let Err(e) = Builder::new(TokioExecutor::new())
        .serve_connection_with_upgrades(io, svc)
        .await
    {
        warn!("Error proxying HTTPS connection: {e}");
        bail!("Error proxying HTTPS connection: {e}");
    }

    Ok(())
}

async fn handle_tls_connection(
    mut tls_stream: tokio_rustls::server::TlsStream<TcpStream>,
    bindings: Arc<HashMap<String, ProxyBinding>>,
    blacklisted_seo_domain: String,
    machine_agent: Arc<MachineAgent>,
) -> Result<()> {
    let (_, server_conn) = tls_stream.get_ref();

    let Some(server_name) = server_conn.server_name().map(|s| s.to_string()) else {
        warn!("No server name in TLS connection");
        bail!("No server name in TLS connection");
    };

    let (binding, nested_protocol) = find_tls_binding(&bindings, &server_name)?;

    if nested_protocol == ExternnalBindingRoutingTlsNestedProtocol::Http {
        info!("Handling HTTP connection over TLS");
        return handle_https_connection(
            tls_stream,
            bindings,
            blacklisted_seo_domain,
            machine_agent,
            server_name,
        )
        .await;
    }

    let machine = find_machine(&machine_agent, &binding.target_network_tag).await?;
    let mut machine_connection =
        get_machine_connection(&machine, binding.target_port, binding.inactivity_timeout).await?;

    info!(
        "Proxying TLS connection from {} to machine on port {}",
        server_name, binding.target_port
    );

    machine_connection
        .proxy_from_client(&mut tls_stream)
        .await?;

    Ok(())
}

fn find_http_binding(
    bindings: &Arc<HashMap<String, ProxyBinding>>,
    target_host: &str,
) -> Result<ProxyBinding> {
    let bindings = bindings.pin_owned();
    bindings
        .values()
        .find(|b| match &b.mode {
            BindingMode::External { routing, port } => match routing {
                ExternalBindingRouting::HttpHostHeader { host }
                    if *host == target_host || format!("{}:{}", host, port) == target_host =>
                {
                    true
                }
                _ => false,
            },
            _ => false,
        })
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("No binding found for HTTP host {target_host}"))
}

fn find_tls_binding(
    bindings: &Arc<HashMap<String, ProxyBinding>>,
    server_name: &str,
) -> Result<(ProxyBinding, ExternnalBindingRoutingTlsNestedProtocol)> {
    let bindings = bindings.pin_owned();
    let binding = bindings
        .values()
        .find(|b| match &b.mode {
            BindingMode::External { routing, .. } => match routing {
                ExternalBindingRouting::TlsSni { host, .. } if *host == server_name => true,
                _ => false,
            },
            _ => false,
        })
        .cloned();

    let Some(binding) = binding else {
        bail!("No binding found for TLS server name {server_name}");
    };

    let nested_protocol = match &binding.mode {
        BindingMode::External { routing, .. } => match routing {
            ExternalBindingRouting::TlsSni {
                nested_protocol, ..
            } => nested_protocol.clone(),
            _ => bail!("No nested protocol found for TLS server name {server_name}"),
        },
        _ => bail!("No nested protocol found for TLS server name {server_name}"),
    };

    Ok((binding, nested_protocol))
}

async fn find_machine(
    machine_agent: &Arc<MachineAgent>,
    network_tag: &str,
) -> Result<Arc<Machine>> {
    machine_agent
        .get_machine_by_network_tag(network_tag)
        .await
        .ok_or_else(|| anyhow::anyhow!("No machine found for network tag {network_tag}"))
}

async fn get_machine_connection(
    machine: &Arc<Machine>,
    target_port: u16,
    inactivity_timeout: Option<Duration>,
) -> Result<TrafficAwareConnection> {
    machine
        .get_connection(target_port, inactivity_timeout)
        .await
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
        let (stream, _) = listener.accept().await?;
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
                .get_connection(binding.target_port, binding.inactivity_timeout)
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
