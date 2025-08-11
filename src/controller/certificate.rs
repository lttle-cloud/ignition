use anyhow::{anyhow, Result};
use async_trait::async_trait;
use std::time::Duration;
use tracing::{error, info};

use crate::{
    controller::{
        context::{ControllerContext, ControllerEvent, ControllerKey},
        Controller, ReconcileNext,
    },
    resource_index::ResourceKind,
    resources::{certificate::{Certificate, CertificateIssuer, CertificateState, CertificateStatus}, metadata::Namespace},
};

pub struct CertificateController;

impl CertificateController {
    pub fn new() -> Self {
        Self
    }

    pub fn new_boxed() -> Box<dyn Controller> {
        Box::new(Self::new())
    }
}

#[async_trait]
impl Controller for CertificateController {
    async fn schedule(
        &self,
        ctx: ControllerContext,
        event: ControllerEvent,
    ) -> Result<Option<ControllerKey>> {
        match event {
            ControllerEvent::ResourceChange(ResourceKind::Certificate, metadata) => {
                Ok(Some(ControllerKey::new(
                    ctx.tenant,
                    ResourceKind::Certificate,
                    metadata.namespace.clone(),
                    metadata.name.clone(),
                )))
            }
            _ => Ok(None),
        }
    }

    async fn should_reconcile(&self, _ctx: ControllerContext, key: ControllerKey) -> bool {
        key.kind == ResourceKind::Certificate
    }

    async fn reconcile(&self, ctx: ControllerContext, key: ControllerKey) -> Result<ReconcileNext> {
        if key.kind != ResourceKind::Certificate {
            return Ok(ReconcileNext::done());
        }
        
        let metadata = key.metadata();

        let tenant = ctx.tenant.clone();
        let cert_repo = ctx.repository.certificate(tenant.clone());

        // Get the certificate resource
        let cert = cert_repo
            .get(Namespace::from_value_or_default(metadata.namespace.clone()), metadata.name.clone())?
            .ok_or_else(|| anyhow!("Certificate not found"))?;
        
        let cert = match cert {
            Certificate::V1(v1) => v1,
        };

        // Get current status
        let mut status = cert_repo
            .get_status(metadata.clone())?
            .unwrap_or_else(|| CertificateStatus {
                state: CertificateState::Pending,
                not_before: None,
                not_after: None,
                last_failure_reason: None,
                renewal_time: None,
            });

        // Handle certificate based on issuer type
        match &cert.issuer {
            CertificateIssuer::Auto { provider, email, .. } => {
                info!(
                    "Processing auto certificate for domains: {:?} with provider: {}",
                    cert.domains, provider
                );

                // Check if ACME account exists or needs to be created
                let cert_agent = ctx.agent.certificate();
                
                match cert_agent.get_acme_account(&tenant, provider.as_str()).await {
                    Ok(Some(_account)) => {
                        info!("ACME account already exists for provider: {}", provider);
                        status.state = CertificateState::Pending;
                        status.last_failure_reason = Some("Account exists, certificate issuance not yet implemented".to_string());
                    }
                    Ok(None) => {
                        info!("Creating ACME account for provider: {}", provider);
                        
                        match cert_agent.create_acme_account(&tenant, provider.as_str(), email.clone()).await {
                            Ok(account) => {
                                info!("Successfully created ACME account: {}", account.account_id);
                                status.state = CertificateState::Pending;
                                status.last_failure_reason = Some("Account created, certificate issuance not yet implemented".to_string());
                            }
                            Err(e) => {
                                error!("Failed to create ACME account: {}", e);
                                status.state = CertificateState::Failed;
                                status.last_failure_reason = Some(format!("Account creation failed: {}", e));
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to check ACME account: {}", e);
                        status.state = CertificateState::Failed;
                        status.last_failure_reason = Some(format!("Account check failed: {}", e));
                    }
                }
            }
            CertificateIssuer::Manual { cert_path, key_path, .. } => {
                info!(
                    "Manual certificate configured with cert: {} and key: {}",
                    cert_path, key_path
                );
                status.state = CertificateState::Ready;
                status.last_failure_reason = None;
            }
        }

        // Update status
        cert_repo.set_status(metadata, status).await?;

        // For now, check again in 5 minutes
        Ok(ReconcileNext::After(Duration::from_secs(300)))
    }

    async fn handle_error(
        &self,
        ctx: ControllerContext,
        key: ControllerKey,
        error: anyhow::Error,
    ) -> ReconcileNext {
        if key.kind != ResourceKind::Certificate {
            return ReconcileNext::done();
        }
        
        let metadata = key.metadata();

        error!("Certificate controller error for {:?}: {}", metadata, error);

        // Update status with error
        if let Ok(Some(mut status)) = ctx
            .repository
            .certificate(ctx.tenant.clone())
            .get_status(metadata.clone())
        {
            status.state = CertificateState::Failed;
            status.last_failure_reason = Some(error.to_string());
            
            let _ = ctx
                .repository
                .certificate(ctx.tenant)
                .set_status(metadata, status);
        }

        // Retry after 30 seconds
        ReconcileNext::After(Duration::from_secs(30))
    }
}