use docker_credential::{CredentialRetrievalError, DockerCredential};
use oci_client::{Reference, secrets::RegistryAuth};
use util::{
    result::Result,
    tracing::{debug, warn},
};

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
