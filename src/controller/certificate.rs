use anyhow::{Result, anyhow};
use async_trait::async_trait;
use std::time::Duration;
use tracing::{error, info};

use crate::{
    controller::{
        Controller, ReconcileNext,
        context::{ControllerContext, ControllerEvent, ControllerKey},
    },
    resource_index::ResourceKind,
    resources::{
        certificate::{Certificate, CertificateIssuer, CertificateState, CertificateStatus},
        metadata::Namespace,
    },
};

pub struct CertificateController;

impl CertificateController {
    pub fn new() -> Self {
        Self
    }

    pub fn new_boxed() -> Box<dyn Controller> {
        Box::new(Self::new())
    }

    async fn reconcile_auto_certificate(
        &self,
        ctx: &ControllerContext,
        status: &mut CertificateStatus,
        provider: &str,
        email: Option<&str>,
        domains: &[String],
    ) -> Result<ReconcileNext> {
        let cert_agent = ctx.agent.certificate();

        let resolved_email = cert_agent.resolve_email(provider, email)?;
        // State machine for auto certificate lifecycle
        match status.state {
            CertificateState::Pending => {
                // Initial state - ensure ACME account exists, then create order
                info!(
                    "Certificate in Pending state, checking ACME account for provider: {}",
                    provider
                );

                // First ensure account exists
                let account_exists = match cert_agent
                    .get_acme_account(provider, Some(&resolved_email))
                    .await
                {
                    Ok(Some(_account)) => {
                        info!("ACME account exists");
                        true
                    }
                    Ok(None) => {
                        info!(
                            "Creating ACME account for provider: {} email: {}",
                            provider, &resolved_email
                        );
                        match cert_agent
                            .create_acme_account(provider, Some(resolved_email.clone()))
                            .await
                        {
                            Ok(account) => {
                                info!("Successfully created ACME account: {}", account.account_id);
                                true
                            }
                            Err(e) => {
                                error!("Failed to create ACME account: {}", e);
                                status.state = CertificateState::Failed;
                                status.last_failure_reason =
                                    Some(format!("Account creation failed: {}", e));
                                return Ok(ReconcileNext::After(Duration::from_secs(60)));
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to check ACME account: {}", e);
                        status.state = CertificateState::Failed;
                        status.last_failure_reason = Some(format!("Account check failed: {}", e));
                        return Ok(ReconcileNext::After(Duration::from_secs(60)));
                    }
                };

                // If account exists, create order
                if account_exists {
                    info!("Creating ACME order for domains: {:?}", domains);
                    match cert_agent
                        .create_order(provider, Some(&resolved_email), domains.to_vec())
                        .await
                    {
                        Ok(_order) => {
                            info!("Successfully created ACME order");
                            status.state = CertificateState::PendingOrder;
                            status.last_failure_reason = None;
                            Ok(ReconcileNext::Immediate)
                        }
                        Err(e) => {
                            error!("Failed to create ACME order: {}", e);
                            status.state = CertificateState::Failed;
                            status.last_failure_reason =
                                Some(format!("Order creation failed: {}", e));
                            Ok(ReconcileNext::After(Duration::from_secs(60)))
                        }
                    }
                } else {
                    Ok(ReconcileNext::After(Duration::from_secs(30)))
                }
            }

            CertificateState::PendingOrder => {
                // Order exists, need to handle authorizations
                info!("Certificate in PendingOrder state, processing authorizations");

                // TODO: Implement authorization handling
                // - Get order status
                // - Process each authorization
                // - Set up challenges (DNS or HTTP)
                status.last_failure_reason =
                    Some("Authorization handling not yet implemented".to_string());
                Ok(ReconcileNext::After(Duration::from_secs(30)))
            }

            CertificateState::PendingDnsChallenge => {
                // DNS challenge set up, waiting for propagation
                info!("Certificate in PendingDnsChallenge state, checking DNS propagation");

                // TODO: Implement DNS challenge validation
                // - Check DNS record propagation
                // - Trigger ACME validation when ready
                Ok(ReconcileNext::After(Duration::from_secs(30)))
            }

            CertificateState::PendingHttpChallenge => {
                // HTTP challenge set up, waiting for validation
                info!("Certificate in PendingHttpChallenge state, checking HTTP challenge");

                // TODO: Implement HTTP challenge validation
                // - Ensure HTTP challenge is accessible
                // - Trigger ACME validation
                Ok(ReconcileNext::After(Duration::from_secs(10)))
            }

            CertificateState::Validating => {
                // ACME server is validating the challenge
                info!("Certificate in Validating state, waiting for ACME validation");

                // TODO: Check validation status from ACME server
                Ok(ReconcileNext::After(Duration::from_secs(10)))
            }

            CertificateState::Issuing => {
                // Challenge passed, waiting for certificate
                info!("Certificate in Issuing state, waiting for certificate issuance");

                // TODO: Finalize order and download certificate
                Ok(ReconcileNext::After(Duration::from_secs(10)))
            }

            CertificateState::Ready => {
                // Certificate is active, check for renewal
                info!("Certificate in Ready state, checking renewal requirements");

                // TODO: Check certificate expiry and trigger renewal if needed
                Ok(ReconcileNext::After(Duration::from_secs(3600))) // Check hourly
            }

            CertificateState::Renewing => {
                // Renewal in progress
                info!("Certificate in Renewing state, processing renewal");

                // Renewal follows similar flow to initial issuance
                status.state = CertificateState::Pending;
                Ok(ReconcileNext::Immediate)
            }

            CertificateState::Failed => {
                // Previous attempt failed, retry
                info!("Certificate in Failed state, retrying");

                // Reset to initial state to retry
                status.state = CertificateState::Pending;
                status.last_failure_reason = None;
                Ok(ReconcileNext::After(Duration::from_secs(60)))
            }

            CertificateState::Expired => {
                // Certificate expired, need to get a new one
                info!("Certificate in Expired state, starting renewal");

                status.state = CertificateState::Renewing;
                Ok(ReconcileNext::Immediate)
            }

            CertificateState::Revoked => {
                // Certificate was revoked, need a new one
                info!("Certificate in Revoked state, getting new certificate");

                status.state = CertificateState::Pending;
                Ok(ReconcileNext::Immediate)
            }
        }
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
            .get(
                Namespace::from_value_or_default(metadata.namespace.clone()),
                metadata.name.clone(),
            )?
            .ok_or_else(|| anyhow!("Certificate not found"))?;

        let cert = match cert {
            Certificate::V1(v1) => v1,
        };

        // Get current status
        let mut status =
            cert_repo
                .get_status(metadata.clone())?
                .unwrap_or_else(|| CertificateStatus {
                    state: CertificateState::Pending,
                    not_before: None,
                    not_after: None,
                    last_failure_reason: None,
                    renewal_time: None,
                });

        // Handle based on issuer type and current state
        let next_reconcile = match &cert.issuer {
            CertificateIssuer::Auto {
                provider, email, ..
            } => {
                self.reconcile_auto_certificate(
                    &ctx,
                    &mut status,
                    provider.as_str(),
                    email.as_deref(),
                    &cert.domains,
                )
                .await?
            }
            CertificateIssuer::Manual {
                cert_path,
                key_path,
                ..
            } => {
                info!(
                    "Manual certificate configured with cert: {} and key: {}",
                    cert_path, key_path
                );
                status.state = CertificateState::Ready;
                status.last_failure_reason = None;
                ReconcileNext::After(Duration::from_secs(3600)) // Check hourly for manual certs
            }
        };

        // Update status
        cert_repo.set_status(metadata, status).await?;

        Ok(next_reconcile)
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
