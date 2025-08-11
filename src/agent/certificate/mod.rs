pub mod config;

use crate::machinery::store::Store;
use anyhow::Result;
use config::CertificateAgentConfig;
use std::sync::Arc;

pub struct CertificateAgent {
    store: Arc<Store>,
    config: CertificateAgentConfig,
}

impl CertificateAgent {
    pub async fn new(store: Arc<Store>, config: CertificateAgentConfig) -> Result<Arc<Self>> {
        Ok(Arc::new(Self { store, config }))
    }
}
