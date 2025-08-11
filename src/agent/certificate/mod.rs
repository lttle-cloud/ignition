pub mod config;

use crate::machinery::store::{Key, Store};
use anyhow::{Result, anyhow};
use config::CertificateAgentConfig;
use instant_acme::{Account, NewAccount};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Serialize, Deserialize)]
pub struct StoredAcmeAccount {
    pub credentials_json: String,
    pub account_id: String,
    pub contact_email: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StoredOrder {
    pub order_url: String,
    pub domains: Vec<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

pub struct CertificateAgent {
    store: Arc<Store>,
    config: CertificateAgentConfig,
}

impl CertificateAgent {
    pub async fn new(store: Arc<Store>, config: CertificateAgentConfig) -> Result<Arc<Self>> {
        Ok(Arc::new(Self { store, config }))
    }

    pub async fn create_acme_account(
        &self,
        tenant_id: &str,
        provider_name: &str,
        contact_email: Option<String>,
    ) -> Result<StoredAcmeAccount> {
        let provider = self
            .config
            .providers
            .iter()
            .find(|p| p.name == provider_name)
            .ok_or_else(|| anyhow!("Certificate provider '{}' not found", provider_name))?;

        let email = contact_email.or_else(|| provider.default_email.clone());

        let contact: Vec<String> = if let Some(email) = &email {
            vec![format!("mailto:{}", email)]
        } else {
            vec![]
        };

        let new_account = NewAccount {
            contact: &contact.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            terms_of_service_agreed: true,
            only_return_existing: false,
        };

        let (account, credentials) = Account::builder()?
            .create(&new_account, provider.acme_base_url.clone(), None)
            .await?;

        let credentials_json = serde_json::to_string(&credentials)?;

        let account_id = account.id().to_string();

        let stored_account = StoredAcmeAccount {
            credentials_json,
            account_id,
            contact_email: email,
            created_at: chrono::Utc::now(),
        };

        let key = Key::<StoredAcmeAccount>::not_namespaced()
            .tenant(tenant_id)
            .collection("acme_account")
            .key(provider_name);
        self.store.put(&key, &stored_account)?;

        Ok(stored_account)
    }

    pub async fn get_acme_account(
        &self,
        tenant_id: &str,
        provider_name: &str,
    ) -> Result<Option<Account>> {
        let key = Key::<StoredAcmeAccount>::not_namespaced()
            .tenant(tenant_id)
            .collection("acme_account")
            .key(provider_name);

        if let Some(stored) = self.store.get(&key)? {
            let _provider = self
                .config
                .providers
                .iter()
                .find(|p| p.name == provider_name)
                .ok_or_else(|| anyhow!("Certificate provider '{}' not found", provider_name))?;

            let credentials = serde_json::from_str(&stored.credentials_json)?;

            let account = Account::builder()?.from_credentials(credentials).await?;

            Ok(Some(account))
        } else {
            Ok(None)
        }
    }
}
