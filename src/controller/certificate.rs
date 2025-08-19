use anyhow::{Result, anyhow};
use async_trait::async_trait;
use instant_acme::{AuthorizationStatus, ChallengeType, OrderStatus};
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
        match &status.state {
            CertificateState::Pending => {
                // Initial state - transition to checking ACME account
                info!("Certificate in Pending state, transitioning to PendingAcmeAccount");
                status.state = CertificateState::PendingAcmeAccount;
                status.last_failure_reason = None;
                Ok(ReconcileNext::Immediate)
            }

            CertificateState::PendingAcmeAccount => {
                // Ensure ACME account exists
                info!(
                    "Certificate in PendingAcmeAccount state, checking account for provider: {}",
                    provider
                );

                match cert_agent
                    .get_acme_account(provider, Some(&resolved_email))
                    .await
                {
                    Ok(Some(_account)) => {
                        info!("ACME account exists, transitioning to PendingOrder");
                        status.state = CertificateState::PendingOrder(None);
                        status.last_failure_reason = None;
                        Ok(ReconcileNext::Immediate)
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
                                status.state = CertificateState::PendingOrder(None);
                                status.last_failure_reason = None;
                                Ok(ReconcileNext::Immediate)
                            }
                            Err(e) => {
                                error!("Failed to create ACME account: {}", e);
                                status.state = CertificateState::Failed;
                                status.last_failure_reason =
                                    Some(format!("Account creation failed: {}", e));
                                Ok(ReconcileNext::After(Duration::from_secs(60)))
                            }
                        }
                    }
                    Err(e) => {
                        error!("Failed to check ACME account: {}", e);
                        status.state = CertificateState::Failed;
                        status.last_failure_reason = Some(format!("Account check failed: {}", e));
                        Ok(ReconcileNext::After(Duration::from_secs(60)))
                    }
                }
            }

            CertificateState::PendingOrder(existing_url) => {
                info!(
                    "Certificate in PendingOrder state, {} order",
                    if existing_url.is_some() {
                        "resuming"
                    } else {
                        "creating"
                    }
                );

                // Get or create the order
                let mut order = if let Some(url) = existing_url.clone() {
                    // Resume existing order
                    info!("Resuming existing order from URL: {}", url);
                    match cert_agent.get_acme_account(provider, email).await {
                        Ok(Some(account)) => {
                            match account.order(url.clone()).await {
                                Ok(order) => order,
                                Err(e) => {
                                    error!("Failed to resume order: {}", e);
                                    // Order might be invalid, create a new one
                                    info!("Creating new order after resume failure");
                                    match cert_agent
                                        .create_order(provider, email, domains.to_vec())
                                        .await
                                    {
                                        Ok(order) => order,
                                        Err(e) => {
                                            error!("Failed to create new ACME order: {}", e);
                                            status.state = CertificateState::Failed;
                                            status.last_failure_reason =
                                                Some(format!("Order creation failed: {}", e));
                                            return Ok(ReconcileNext::After(Duration::from_secs(
                                                60,
                                            )));
                                        }
                                    }
                                }
                            }
                        }
                        _ => {
                            error!("Failed to get ACME account for order resume");
                            status.state = CertificateState::Failed;
                            status.last_failure_reason =
                                Some("Account not found for order resume".to_string());
                            return Ok(ReconcileNext::After(Duration::from_secs(60)));
                        }
                    }
                } else {
                    // Create a new ACME order for the domains
                    info!("Creating new ACME order");
                    match cert_agent
                        .create_order(provider, email, domains.to_vec())
                        .await
                    {
                        Ok(order) => order,
                        Err(e) => {
                            error!("Failed to create ACME order: {}", e);
                            status.state = CertificateState::Failed;
                            status.last_failure_reason =
                                Some(format!("Order creation failed: {}", e));
                            return Ok(ReconcileNext::After(Duration::from_secs(60)));
                        }
                    }
                };

                // Check order status first
                let state = order.state();
                let order_status = state.status.clone();

                // Process authorizations to determine challenge type
                let mut authorizations = order.authorizations();
                let mut needs_dns_challenge = false;
                let mut needs_http_challenge = false;
                let mut all_valid = true;

                while let Some(result) = authorizations.next().await {
                    match result {
                        Ok(authz) => {
                            let identifier = authz.identifier().to_string();
                            match authz.status {
                                AuthorizationStatus::Valid => {
                                    info!("Authorization for {} is already valid", identifier);
                                    continue;
                                }
                                AuthorizationStatus::Pending => {
                                    all_valid = false;
                                    // Check what challenge types are available by iterating through the challenges field
                                    // AuthorizationHandle derefs to AuthorizationState which has a challenges field
                                    // Prefer HTTP-01 over DNS-01 as it's simpler to implement
                                    let has_http = authz
                                        .challenges
                                        .iter()
                                        .any(|c| c.r#type == ChallengeType::Http01);
                                    let has_dns = authz
                                        .challenges
                                        .iter()
                                        .any(|c| c.r#type == ChallengeType::Dns01);

                                    if has_http {
                                        needs_http_challenge = true;
                                        info!("Found HTTP-01 challenge for {}", identifier);
                                    } else if has_dns {
                                        needs_dns_challenge = true;
                                        info!("Found DNS-01 challenge for {}", identifier);
                                    } else {
                                        error!("No supported challenge type for {}", identifier);
                                        status.state = CertificateState::Failed;
                                        status.last_failure_reason = Some(format!(
                                            "No supported challenge type for {}",
                                            identifier
                                        ));
                                        return Ok(ReconcileNext::After(Duration::from_secs(60)));
                                    }
                                }
                                _ => {
                                    error!(
                                        "Authorization for {} in unexpected state: {:?}",
                                        identifier, authz.status
                                    );
                                    status.state = CertificateState::Failed;
                                    status.last_failure_reason = Some(format!(
                                        "Authorization in unexpected state: {:?}",
                                        authz.status
                                    ));
                                    return Ok(ReconcileNext::After(Duration::from_secs(60)));
                                }
                            }
                        }
                        Err(e) => {
                            error!("Failed to get authorization: {}", e);
                            status.state = CertificateState::Failed;
                            status.last_failure_reason =
                                Some(format!("Failed to get authorization: {}", e));
                            return Ok(ReconcileNext::After(Duration::from_secs(60)));
                        }
                    }
                }

                // Extract order URL now that we're done with the order
                let (order_url, _order_state) = order.into_parts();
                info!("Order URL: {}", order_url);

                // Check if order is already ready
                if order_status == instant_acme::OrderStatus::Ready {
                    info!("Order is already ready, transitioning to Issuing");
                    status.state = CertificateState::Issuing(order_url);
                    status.last_failure_reason = None;
                    return Ok(ReconcileNext::Immediate);
                }

                // Determine next state based on authorizations (pass order URL along)
                if all_valid {
                    info!("All authorizations are valid, transitioning to Issuing");
                    status.state = CertificateState::Issuing(order_url);
                } else if needs_http_challenge {
                    info!("Setting up HTTP-01 challenges");
                    status.state = CertificateState::PendingHttpChallenge(order_url);
                } else if needs_dns_challenge {
                    info!("Setting up DNS-01 challenges");
                    status.state = CertificateState::PendingDnsChallenge(order_url);
                } else {
                    error!("No valid authorization path found");
                    status.state = CertificateState::Failed;
                    status.last_failure_reason = Some("No valid authorization path".to_string());
                    return Ok(ReconcileNext::After(Duration::from_secs(60)));
                }

                status.last_failure_reason = None;
                Ok(ReconcileNext::Immediate)
            }

            CertificateState::PendingDnsChallenge(order_url) => {
                // DNS challenge set up, waiting for propagation
                info!("Certificate in PendingDnsChallenge state, checking DNS propagation");
                info!("Order URL: {}", order_url);

                // TODO: Implement DNS challenge validation
                // - Resume order from URL
                // - Check DNS record propagation
                // - Trigger ACME validation when ready
                // - Transition to Validating(order_url)
                Ok(ReconcileNext::After(Duration::from_secs(30)))
            }

            CertificateState::PendingHttpChallenge(order_url) => {
                // HTTP challenge set up, waiting for validation
                info!("Certificate in PendingHttpChallenge state, checking HTTP challenge");
                info!("Order URL: {}", order_url);

                let account = cert_agent.get_acme_account(provider, email).await?.unwrap();
                let mut order = account.order(order_url.clone()).await?;
                let mut authorizations = order.authorizations();
                while let Some(result) = authorizations.next().await {
                    let mut authz = result?;
                    match authz.status {
                        AuthorizationStatus::Pending => {}
                        AuthorizationStatus::Valid => continue,
                        _ => todo!(),
                    }

                    let mut challenge = authz
                        .challenge(ChallengeType::Http01)
                        .ok_or_else(|| anyhow::anyhow!("no http01 challenge found"))?;

                    let identifier = challenge.identifier();

                    let key_authorization = challenge.key_authorization();
                    let key_authorization = key_authorization.as_str();

                    info!(
                        "HTTP challenge for {} is pending, key authorization: {}",
                        identifier, key_authorization
                    );

                    cert_agent
                        .store_challenge(
                            identifier.to_string(),
                            key_authorization.to_string(),
                            "http-01".to_string(),
                        )
                        .await?;

                    challenge.set_ready().await?;
                }
                status.state = CertificateState::Validating(order_url.clone());
                Ok(ReconcileNext::Immediate)
            }

            CertificateState::Validating(order_url) => {
                // ACME server is validating the challenge
                info!("Certificate in Validating state, waiting for ACME validation");
                info!("Order URL: {}", order_url);

                // TODO: Check validation status from ACME server
                // - Resume order from URL
                // - Poll order status
                // - If ready, transition to Issuing(order_url)
                // - If still processing, stay in Validating
                Ok(ReconcileNext::After(Duration::from_secs(10)))
            }

            CertificateState::Issuing(order_url) => {
                // Challenge passed, waiting for certificate
                info!("Certificate in Issuing state, waiting for certificate issuance");
                info!("Order URL: {}", order_url);

                // TODO: Finalize order and download certificate
                // - Resume order from URL
                // - Generate CSR and finalize order
                // - Poll for certificate
                // - Store certificate and transition to Ready
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
