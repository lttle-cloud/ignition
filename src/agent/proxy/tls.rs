use std::{path::Path, sync::Arc};

use anyhow::{Result, bail};
use papaya::HashMap;
use rustls::{
    crypto::CryptoProvider,
    pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject},
    server::{ClientHello, ResolvesServerCert},
    sign::{self, CertifiedKey},
};
use tracing::{info, warn};

pub async fn load_cert_from_disk(
    cert_file: impl AsRef<Path>,
    key_file: impl AsRef<Path>,
    crypto_provider: &CryptoProvider,
) -> Result<Arc<CertifiedKey>> {
    let cert_file = cert_file.as_ref();
    let key_file = key_file.as_ref();

    info!(
        "Loading TLS certificate from {:?} and key from {:?}",
        cert_file, key_file
    );

    let Ok(cert_file_iter) = CertificateDer::pem_file_iter(cert_file) else {
        warn!("Failed to load certificate from {cert_file:?}");
        bail!("Failed to load certificate from {cert_file:?}");
    };

    let Ok(cert_der) = cert_file_iter
        .map(|cert| cert)
        .collect::<Result<Vec<_>, _>>()
    else {
        warn!("Failed to parse certificate from {cert_file:?}");
        bail!("Failed to load certificate from {cert_file:?}");
    };

    let Ok(key) = PrivateKeyDer::from_pem_file(key_file) else {
        warn!("Failed to load private key from {key_file:?}");
        bail!("Failed to load key from {key_file:?}");
    };

    let Ok(cert_key) = sign::CertifiedKey::from_der(cert_der, key, crypto_provider) else {
        warn!("Failed to create certified key from certificate and private key");
        bail!("Failed to load certificate from {cert_file:?}");
    };

    info!("Successfully loaded TLS certificate and key");
    Ok(Arc::new(cert_key))
}

#[derive(Debug)]
#[allow(unused)]
pub struct ProxyTlsCertResolver {
    cert_pool: Arc<HashMap<String, Arc<CertifiedKey>>>,
    default_cert: Arc<CertifiedKey>,
    crypto_provider: Arc<CryptoProvider>,
}

impl ProxyTlsCertResolver {
    pub fn new(
        cert_pool: Arc<HashMap<String, Arc<CertifiedKey>>>,
        default_cert: Arc<CertifiedKey>,
        crypto_provider: Arc<CryptoProvider>,
    ) -> Self {
        info!(
            "Creating new TLS certificate resolver with {} certificates in pool",
            cert_pool.len()
        );
        Self {
            cert_pool,
            default_cert,
            crypto_provider,
        }
    }

    pub fn resolve_cert(&self, host: &str) -> Option<Arc<CertifiedKey>> {
        let cert_pool = self.cert_pool.pin();
        let cert = cert_pool.get(host);

        if let Some(cert) = cert {
            info!("Found specific certificate for host: {}", host);
            return Some(cert.clone());
        }

        info!("Using default certificate for host: {}", host);
        Some(self.default_cert.clone())
    }
}

impl ResolvesServerCert for ProxyTlsCertResolver {
    fn resolve(&self, client_hello: ClientHello) -> Option<Arc<CertifiedKey>> {
        let Some(server_name) = client_hello.server_name() else {
            warn!("No server name in client hello, using default certificate");
            return Some(self.default_cert.clone());
        };

        let host_name = server_name.to_string();
        info!("Resolving certificate for server name: {}", host_name);
        self.resolve_cert(&host_name)
    }
}
