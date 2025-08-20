use anyhow::{Result, anyhow};
use async_trait::async_trait;
use hickory_resolver::{
    TokioAsyncResolver,
    config::{ResolverConfig, ResolverOpts},
};
use instant_acme::{AuthorizationStatus, ChallengeType, OrderStatus, RetryPolicy};
use std::time::Duration;
use tracing::{error, info, warn};

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

    async fn validate_domain_dns_resolution(&self, domains: &[String]) -> Result<()> {
        let resolver =
            TokioAsyncResolver::tokio(ResolverConfig::default(), ResolverOpts::default());

        for domain in domains {
            info!("Checking DNS resolution for domain: {}", domain);

            // Try to resolve the domain to ensure it exists
            match resolver.lookup_ip(domain).await {
                Ok(lookup) => {
                    let ips: Vec<_> = lookup.iter().collect();
                    info!("Domain {} resolves to: {:?}", domain, ips);
                }
                Err(e) => {
                    warn!("DNS resolution failed for domain {}: {}", domain, e);
                    return Err(anyhow!(
                        "DNS resolution failed for domain '{}': {}. Please ensure the domain exists and is properly configured.",
                        domain,
                        e
                    ));
                }
            }
        }

        info!("All domains successfully resolved via DNS");
        Ok(())
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

        // Check if any of the requested domains are different from what we have
        if domains
            .iter()
            .any(|domain| !status.domains.contains(domain))
        {
            info!(
                "New domains detected: {:?} (current certificate covers: {:?})",
                domains, status.domains
            );
            status.state = CertificateState::Pending;
            status.domains = domains.to_vec();
            return Ok(ReconcileNext::Immediate);
        }

        // State machine for auto certificate lifecycle
        match &status.state {
            CertificateState::Pending => {
                // Initial state - transition to checking ACME account
                info!("Certificate in Pending state, transitioning to PendingAcmeAccount");
                status.state = CertificateState::PendingAcmeAccount;
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
                        info!("ACME account exists, transitioning to PendingDnsResolution");
                        status.state = CertificateState::PendingDnsResolution; // Empty string since no order URL yet
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
                                status.state = CertificateState::PendingDnsResolution; // Empty string since no order URL yet
                                Ok(ReconcileNext::Immediate)
                            }
                            Err(e) => Err(anyhow!("Failed to create ACME account: {}", e)),
                        }
                    }
                    Err(e) => Err(anyhow!("Failed to check ACME account: {}", e)),
                }
            }

            CertificateState::PendingDnsResolution => {
                info!(
                    "Certificate in PendingDnsResolution state, validating DNS resolution for domains: {:?}",
                    domains
                );

                // Validate that all domains resolve via DNS before creating ACME order
                match self.validate_domain_dns_resolution(domains).await {
                    Ok(()) => {
                        info!("DNS validation successful, transitioning to PendingOrder");
                        status.state = CertificateState::PendingOrder(None);
                        Ok(ReconcileNext::Immediate)
                    }
                    Err(e) => Err(anyhow!("DNS validation failed: {}", e)),
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
                                    warn!("Failed to resume order: {}", e);
                                    // Order might be invalid, create a new one
                                    info!("Creating new order after resume failure");
                                    match cert_agent
                                        .create_order(provider, email, domains.to_vec())
                                        .await
                                    {
                                        Ok(order) => order,
                                        Err(e) => {
                                            return Err(anyhow!(
                                                "Failed to create new ACME order: {}",
                                                e
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                        _ => return Err(anyhow!("Failed to get ACME account for order resume")),
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
                            return Err(anyhow!("Failed to create ACME order: {}", e));
                        }
                    }
                };

                // Check order status first
                let state = order.state();
                let order_status = state.status.clone();

                // Process authorizations to determine challenge type
                let mut authorizations = order.authorizations();
                let mut needs_challenge = false;

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
                                    if authz
                                        .challenges
                                        .iter()
                                        .any(|c| c.r#type == ChallengeType::Http01)
                                    {
                                        needs_challenge = true;
                                        info!("Found HTTP-01 challenge for {}", identifier);
                                    } else {
                                        return Err(anyhow!(
                                            "No supported challenge type for {}",
                                            identifier
                                        ));
                                    }
                                }
                                _ => {
                                    return Err(anyhow!(
                                        "Authorization in unexpected state: {:?}",
                                        authz.status
                                    ));
                                }
                            }
                        }
                        Err(e) => {
                            return Err(anyhow!("Failed to get authorization: {}", e));
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
                    return Ok(ReconcileNext::Immediate);
                }

                // Determine next state based on authorizations (pass order URL along)
                if needs_challenge {
                    info!("Setting up HTTP-01 challenges");
                    status.state = CertificateState::PendingChallenge(order_url);
                } else {
                    return Err(anyhow!("No valid authorization path found"));
                }

                Ok(ReconcileNext::Immediate)
            }

            CertificateState::PendingChallenge(order_url) => {
                // HTTP challenge set up, waiting for validation
                info!("Certificate in PendingChallenge state, checking HTTP challenge");
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

                let account = cert_agent.get_acme_account(provider, email).await?.unwrap();
                let mut order = account.order(order_url.clone()).await?;
                let order_status = order.poll_ready(&RetryPolicy::default()).await?;
                if order_status != OrderStatus::Ready {
                    status.state = CertificateState::Failed;
                    status.last_failure_reason = Some("Order not ready".to_string());
                    return Ok(ReconcileNext::After(Duration::from_secs(10)));
                }
                status.state = CertificateState::Issuing(order_url.clone());
                Ok(ReconcileNext::Immediate)
            }

            CertificateState::Issuing(order_url) => {
                // Challenge passed, waiting for certificate
                info!("Certificate in Issuing state, waiting for certificate issuance");
                info!("Order URL: {}", order_url);

                let account = cert_agent.get_acme_account(provider, email).await?.unwrap();
                let mut order = account.order(order_url.clone()).await?;
                let private_key_pem = order.finalize().await?;
                let cert_chain_pem = order.poll_certificate(&RetryPolicy::default()).await?;
                cert_agent
                    .store_certificate(
                        cert_chain_pem.clone(),
                        private_key_pem.clone(),
                        domains.to_vec(),
                    )
                    .await?;

                let (not_before, not_after) =
                    cert_agent.parse_certificate_validity(&cert_chain_pem)?;
                status.state = CertificateState::Ready;
                status.not_before = Some(not_before.to_rfc3339());
                status.not_after = Some(not_after.to_rfc3339());
                status.last_failure_reason = None;

                info!(
                    "Certificate issued successfully. Valid from {} to {}",
                    not_before, not_after
                );
                Ok(ReconcileNext::Immediate)
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
                    domains: cert.domains.clone(),
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
