use std::sync::Arc;

use anyhow::Result;
use docker_credential::{CredentialRetrievalError, DockerCredential};
use oci_client::{Reference, secrets::RegistryAuth};
use tracing::{debug, warn};

use crate::api::auth::{AuthHandler, RegistryRobotHmacClaims};

pub trait OciCredentialsProvider {
    fn get_credentials_for_reference(&self, reference: &Reference) -> Result<RegistryAuth>;
}

pub struct AnonymousOciCredentialsProvider;

impl OciCredentialsProvider for AnonymousOciCredentialsProvider {
    fn get_credentials_for_reference(&self, _reference: &Reference) -> Result<RegistryAuth> {
        Ok(RegistryAuth::Anonymous)
    }
}

pub struct DockerCredentialsProvider {}

impl OciCredentialsProvider for DockerCredentialsProvider {
    fn get_credentials_for_reference(&self, reference: &Reference) -> Result<RegistryAuth> {
        let server = reference
            .resolve_registry()
            .strip_suffix('/')
            .unwrap_or_else(|| reference.resolve_registry());

        let auth = match docker_credential::get_credential(server) {
            Err(CredentialRetrievalError::ConfigNotFound) => RegistryAuth::Anonymous,
            Err(CredentialRetrievalError::NoCredentialConfigured) => RegistryAuth::Anonymous,
            Err(e) => {
                warn!("Error handling docker configuration file: {}", e);
                RegistryAuth::Anonymous
            }
            Ok(DockerCredential::UsernamePassword(username, password)) => {
                debug!("Found docker credentials");
                RegistryAuth::Basic(username, password)
            }
            Ok(DockerCredential::IdentityToken(_)) => {
                warn!(
                    "Cannot use contents of docker config, identity token not supported. Using anonymous auth"
                );
                RegistryAuth::Anonymous
            }
        };

        Ok(auth)
    }
}

pub struct InternalCredentialsProvider {
    registry_service: String,
    auth_handler: Arc<AuthHandler>,
    tenant: String,
}

impl InternalCredentialsProvider {
    pub fn new(
        auth_handler: Arc<AuthHandler>,
        internal_registry_service: String,
        tenant: String,
    ) -> Self {
        Self {
            auth_handler,
            registry_service: internal_registry_service,
            tenant,
        }
    }
}

impl OciCredentialsProvider for InternalCredentialsProvider {
    fn get_credentials_for_reference(&self, reference: &Reference) -> Result<RegistryAuth> {
        let host = reference.resolve_registry();
        let repository = reference.repository();

        let target_tenant = repository.split('/').next();

        if host != self.registry_service {
            return Ok(RegistryAuth::Anonymous);
        }

        let Some(tenant) = target_tenant else {
            return Ok(RegistryAuth::Anonymous);
        };

        if tenant != self.tenant {
            return Ok(RegistryAuth::Anonymous);
        }

        let claims = RegistryRobotHmacClaims::new(tenant, "auto");
        let user = claims.to_string();
        let pass = self.auth_handler.generate_registry_hmac(&claims)?;

        return Ok(RegistryAuth::Basic(user, pass));
    }
}
