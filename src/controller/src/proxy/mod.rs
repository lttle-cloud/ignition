use std::{fmt::Debug, net::Ipv4Addr, path::PathBuf, sync::Arc};

use papaya::HashMap;
use rustls::{
    ServerConfig,
    crypto::CryptoProvider,
    pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject},
    server::{ClientHello, ResolvesServerCert},
    sign,
};
use tokio_rustls::TlsAcceptor;
use util::{
    async_runtime::{
        self,
        net::{TcpListener, TcpStream},
        task::{self, JoinHandle},
    },
    result::{Result, bail},
    tracing::info,
};

use crate::Controller;

#[derive(Clone)]
pub struct ProxyConfig {
    pub external_host: String,

    pub http_port: u16,
    pub https_port: u16,
}

#[derive(Clone, Debug)]
pub struct ProxyTlsTerminationConfig {
    pub ssl_cert_path: PathBuf,
    pub ssl_key_path: PathBuf,
}

#[derive(Clone, Debug)]
pub enum ProxyServiceType {
    ExternalTls {
        port: u16,
        host: String,
        tls_termination: Option<ProxyTlsTerminationConfig>,
    },
    ExternalHttps {
        host: String,
        tls_termination: ProxyTlsTerminationConfig,
    },
    InternalTcp {
        port: u16,
        addr: Ipv4Addr,
    },
}

#[derive(Clone, Debug)]
pub struct ProxyServiceTarget {
    pub machine_name: String,
    pub port: u16,
}

#[derive(Clone, Debug)]
pub struct ProxyServiceBinding {
    pub service_name: String,
    pub service_type: ProxyServiceType,
    pub service_target: ProxyServiceTarget,
}

type ListenerTaskKey = (String, u16);
type ServiceCacheKey = (String, u16);

impl ProxyServiceType {
    pub fn get_listener_task_key(&self, config: &ProxyConfig) -> ListenerTaskKey {
        match self {
            ProxyServiceType::ExternalTls { port, .. } => (config.external_host.clone(), *port),
            ProxyServiceType::InternalTcp { port, addr } => (addr.to_string(), *port),
            ProxyServiceType::ExternalHttps { .. } => {
                (config.external_host.clone(), config.https_port)
            }
        }
    }

    pub fn get_service_cache_key(&self, config: &ProxyConfig) -> ServiceCacheKey {
        match self {
            ProxyServiceType::ExternalTls { port, host, .. } => (host.clone(), *port),
            ProxyServiceType::InternalTcp { port, addr } => (addr.to_string(), *port),
            ProxyServiceType::ExternalHttps { host, .. } => (host.clone(), config.https_port),
        }
    }

    pub fn get_tls_termination(&self) -> Option<(String, ProxyTlsTerminationConfig)> {
        match self {
            ProxyServiceType::ExternalTls {
                host,
                tls_termination,
                ..
            } if tls_termination.is_some() => {
                let tls_termination = tls_termination.clone().unwrap();
                Some((host.clone(), tls_termination))
            }
            ProxyServiceType::ExternalHttps {
                host,
                tls_termination,
            } => Some((host.clone(), tls_termination.clone())),
            _ => None,
        }
    }
}

pub struct Proxy {
    config: ProxyConfig,

    controller: Arc<Controller>,

    service_bindings: Arc<HashMap<String, ProxyServiceBinding>>,
    listener_tasks: HashMap<ListenerTaskKey, JoinHandle<()>>,

    service_cache: Arc<HashMap<ServiceCacheKey, ProxyServiceBinding>>,
    tls_termination_cache: Arc<HashMap<String, ProxyTlsTerminationConfig>>,

    tls_acceptor: Arc<TlsAcceptor>,
    tls_cert_resolver: Arc<ProxyTlsCertResolver>,
}

struct ProxyTlsCertResolver {
    crypto_provider: Arc<CryptoProvider>,
    termination_configs: Arc<HashMap<String, ProxyTlsTerminationConfig>>,
    certs_cache: HashMap<String, Arc<sign::CertifiedKey>>,
}

impl Debug for ProxyTlsCertResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TlsCertResolver")
    }
}

impl ProxyTlsCertResolver {
    pub fn new(
        termination_configs: Arc<HashMap<String, ProxyTlsTerminationConfig>>,
        crypto_provider: Arc<CryptoProvider>,
    ) -> Self {
        Self {
            termination_configs,
            certs_cache: HashMap::new(),
            crypto_provider,
        }
    }
}

impl ProxyTlsCertResolver {
    pub fn invalidate_cert_for_host(&self, host: &str) {
        let certs_cache = self.certs_cache.pin();
        certs_cache.remove(host);
    }
}

impl ResolvesServerCert for ProxyTlsCertResolver {
    fn resolve(&self, client_hello: ClientHello) -> Option<Arc<sign::CertifiedKey>> {
        let Some(server_name) = client_hello.server_name() else {
            return None;
        };

        let certs_cache = self.certs_cache.pin();
        if let Some(cert) = certs_cache.get(server_name) {
            return Some(cert.clone());
        };

        let termination_configs = self.termination_configs.pin();
        let Some(termination_config) = termination_configs.get(server_name) else {
            return None;
        };

        let Ok(cert_file_iter) = CertificateDer::pem_file_iter(&termination_config.ssl_cert_path)
        else {
            return None;
        };

        let Ok(cert_der) = cert_file_iter
            .map(|cert| cert)
            .collect::<Result<Vec<_>, _>>()
        else {
            return None;
        };

        let Ok(key) = PrivateKeyDer::from_pem_file(&termination_config.ssl_key_path) else {
            return None;
        };

        let Ok(cert_key) = sign::CertifiedKey::from_der(cert_der, key, &self.crypto_provider)
        else {
            return None;
        };

        let cert_key = Arc::new(cert_key);
        certs_cache.insert(server_name.to_string(), cert_key.clone());
        Some(cert_key)
    }
}

impl Proxy {
    pub async fn new(config: ProxyConfig, controller: Arc<Controller>) -> Result<Arc<Self>> {
        let termination_configs = Arc::new(HashMap::new());

        let tls_server_config_builder = ServerConfig::builder().with_no_client_auth();

        let tls_cert_resolver = Arc::new(ProxyTlsCertResolver::new(
            termination_configs.clone(),
            tls_server_config_builder.crypto_provider().clone(),
        ));

        let tls_server_config =
            tls_server_config_builder.with_cert_resolver(tls_cert_resolver.clone());

        let tls_acceptor = Arc::new(TlsAcceptor::from(Arc::new(tls_server_config)));

        let proxy = Self {
            config,
            controller,
            service_bindings: Arc::new(HashMap::new()),
            listener_tasks: HashMap::new(),
            service_cache: Arc::new(HashMap::new()),
            tls_termination_cache: termination_configs,
            tls_cert_resolver,
            tls_acceptor,
        };

        proxy.start_listener_tasks_for_http_and_https().await?;

        Ok(Arc::new(proxy))
    }

    pub async fn bind_service(&self, service_binding: ProxyServiceBinding) -> Result<()> {
        self.bulk_bind_services(vec![service_binding]).await
    }

    pub async fn unbind_service(&self, service_name: &str) -> Result<()> {
        let service_bindings = self.service_bindings.pin();
        let Some(service_binding) = service_bindings.remove(service_name) else {
            bail!("Service {} not found", service_name);
        };

        let service_cache = self.service_cache.pin();

        let service_cache_key = service_binding
            .service_type
            .get_service_cache_key(&self.config);
        service_cache.remove(&service_cache_key);

        let listener_tasks = self.listener_tasks.pin();
        let listener_task_key = service_binding
            .service_type
            .get_listener_task_key(&self.config);

        for (_, val) in service_bindings.iter() {
            let other_listener_task_key = val.service_type.get_listener_task_key(&self.config);
            if other_listener_task_key == listener_task_key {
                continue;
            };

            if other_listener_task_key == listener_task_key {
                // we can't remove the listener task key because there are still services bound to it
                return Ok(());
            }
        }
        let Some(task) = listener_tasks.remove(&listener_task_key) else {
            return Ok(());
        };
        task.abort();

        Ok(())
    }

    pub async fn bulk_bind_services(
        &self,
        new_service_bindings: Vec<ProxyServiceBinding>,
    ) -> Result<()> {
        let service_bindings = self.service_bindings.pin();

        for service_binding in new_service_bindings.iter() {
            if let Some(existing_service) = service_bindings.get(&service_binding.service_name) {
                info!(
                    "Unbinding existing service {:?}",
                    existing_service.service_name
                );
                self.unbind_service(&existing_service.service_name).await?;
            };

            info!("Binding service {:?}", service_binding);

            service_bindings.insert(
                service_binding.service_name.clone(),
                service_binding.clone(),
            );

            let service_cache = self.service_cache.pin();
            let service_cache_key = service_binding
                .service_type
                .get_service_cache_key(&self.config);
            service_cache.insert(service_cache_key, service_binding.clone());

            if let Some((host, tls_termination)) =
                service_binding.service_type.get_tls_termination()
            {
                let tls_termination_cache = self.tls_termination_cache.pin();
                tls_termination_cache.insert(host.clone(), tls_termination);
                self.tls_cert_resolver.invalidate_cert_for_host(&host);
            };
        }

        let listener_tasks = self.listener_tasks.pin();
        for service_binding in new_service_bindings.iter() {
            let listener_task_key = service_binding
                .service_type
                .get_listener_task_key(&self.config);

            if listener_tasks.contains_key(&listener_task_key) {
                continue;
            }

            self.start_listener_task(listener_task_key).await?;
        }

        Ok(())
    }

    pub async fn start_listener_task(&self, listener_task_key: ListenerTaskKey) -> Result<()> {
        let is_internal = listener_task_key.0 == self.config.external_host;

        info!(
            "Starting {} listener task {:?}",
            if is_internal { "internal" } else { "external" },
            &listener_task_key
        );

        let listener = TcpListener::bind(&listener_task_key).await?;

        let service_cache = self.service_cache.clone();
        let tls_acceptor = self.tls_acceptor.clone();
        let controller = self.controller.clone();

        let internal_cache_key = (listener_task_key.0.clone(), listener_task_key.1);
        let task = task::spawn(async move {
            info!("Starting listener task {:?}", &listener_task_key);

            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    continue;
                };

                let internal_cache_key = internal_cache_key.clone();
                let tls_acceptor = tls_acceptor.clone();
                let service_cache = service_cache.clone();
                let controller = controller.clone();

                task::spawn(async move {
                    let service_cache = service_cache.pin_owned();

                    info!("Accepted tcp conn");
                    // let Ok(mut tls_stream) = tls_acceptor.accept(stream).await else {
                    //     info!("Failed to accept tls conn");
                    //     return;
                    // };

                    let mut tls_stream = match tls_acceptor.accept(stream).await {
                        Ok(tls_stream) => tls_stream,
                        Err(e) => {
                            info!("Failed to accept tls conn: {:?}", e);
                            return;
                        }
                    };

                    info!("Accepted tls conn");

                    let (_tcp_stream, server_conn) = tls_stream.get_ref();

                    let service_cache_key = if let Some(sni_name) = server_conn.server_name() {
                        &(sni_name.to_string(), listener_task_key.1)
                    } else {
                        &internal_cache_key
                    };

                    info!("Service cache key look-up: {:?}", service_cache_key);

                    let Some(service_binding) = service_cache.get(service_cache_key) else {
                        info!("No service binding found for {:?}", service_cache_key);
                        return;
                    };

                    info!(
                        "Received conn for {:?} {:?}",
                        service_cache_key, service_binding
                    );

                    let Ok(ip) = controller
                        .get_ip_for_machine_name(&service_binding.service_target.machine_name)
                        .await
                    else {
                        info!(
                            "Failed to get ip for machine {}",
                            service_binding.service_target.machine_name
                        );
                        return;
                    };

                    info!(
                        "Forwarding to {}:{}",
                        ip, service_binding.service_target.port
                    );

                    let Ok(mut backend_stream) =
                        TcpStream::connect((ip.clone(), service_binding.service_target.port)).await
                    else {
                        info!(
                            "Failed to connect to {}:{}",
                            ip, service_binding.service_target.port
                        );
                        return;
                    };

                    let result =
                        async_runtime::io::copy_bidirectional(&mut tls_stream, &mut backend_stream)
                            .await;
                    info!("Forwarding result: {:?}", result);
                });
            }
        });

        let listener_tasks = self.listener_tasks.pin();
        listener_tasks.insert(
            (self.config.external_host.clone(), self.config.https_port),
            task,
        );

        Ok(())
    }

    pub async fn start_listener_tasks_for_http_and_https(&self) -> Result<()> {
        // self.start_listener_task((self.config.external_host.clone(), self.config.http_port))
        //     .await?;

        self.start_listener_task((self.config.external_host.clone(), self.config.https_port))
            .await?;

        Ok(())
    }

    pub async fn listen(&self) -> Result<()> {
        info!(
            "Starting proxy on ports {} and {}",
            self.config.http_port, self.config.https_port
        );

        Ok(())
    }
}
