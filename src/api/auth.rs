use std::{
    collections::BTreeSet,
    path::Path,
    str::FromStr,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, bail};
use base64::{Engine, prelude::BASE64_URL_SAFE_NO_PAD};
use blake3::KEY_LEN;
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthTokenClaims {
    pub tenant: String,
    pub sub: String,
    iat: u64,
    exp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryRobotHmacClaims {
    tenant: String,
    sub: String,
}

impl std::fmt::Display for RegistryRobotHmacClaims {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.tenant, self.sub)
    }
}

impl FromStr for RegistryRobotHmacClaims {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (tenant, sub) = s
            .split_once('/')
            .ok_or(anyhow::anyhow!("Invalid registry robot hmac claims"))?;

        Ok(RegistryRobotHmacClaims {
            tenant: tenant.to_string(),
            sub: sub.to_string(),
        })
    }
}

impl RegistryRobotHmacClaims {
    pub fn new(tenant: impl AsRef<str>, sub: impl AsRef<str>) -> Self {
        Self {
            tenant: tenant.as_ref().to_string(),
            sub: sub.as_ref().to_string(),
        }
    }
}

pub struct AuthHandler {
    jwt_secret: String,
    registry_robot_hmac_secret: [u8; KEY_LEN],
    pub registry_service: String,
    registry_token_key: Vec<u8>,
    registry_token_cert_der: String,
}

impl AuthHandler {
    pub fn new(
        jwt_secret: impl AsRef<str>,
        registry_robot_hmac_secret: impl AsRef<str>,
        registry_service: impl AsRef<str>,
        registry_token_key_path: Option<impl AsRef<Path>>,
        registry_token_cert_path: Option<impl AsRef<Path>>,
    ) -> Result<Self> {
        let registry_robot_hmac_secret =
            BASE64_URL_SAFE_NO_PAD.decode(registry_robot_hmac_secret.as_ref())?;

        let registry_token_key = registry_token_key_path
            .map(|path| std::fs::read(path.as_ref()))
            .transpose()?
            .unwrap_or_default();

        let registry_token_cert_der = if let Some(path) = registry_token_cert_path {
            let cert_pem = std::fs::read(path.as_ref())?;
            let mut rd = std::io::Cursor::new(cert_pem);
            let certs = rustls_pemfile::certs(&mut rd).collect::<Result<Vec<_>, _>>()?; // Vec<Vec<u8>> DER
            if certs.is_empty() {
                bail!("no certificate found in registry_token_cert_path");
            }
            let leaf_der = &certs[0];
            let registry_token_cert_der_b64 =
                base64::engine::general_purpose::STANDARD.encode(leaf_der);
            registry_token_cert_der_b64
        } else {
            String::new()
        };

        Ok(Self {
            jwt_secret: jwt_secret.as_ref().to_string(),
            registry_robot_hmac_secret: registry_robot_hmac_secret[..KEY_LEN].try_into()?,
            registry_service: registry_service.as_ref().to_string(),
            registry_token_key,
            registry_token_cert_der,
        })
    }

    pub fn generate_token(
        &self,
        tenant: impl AsRef<str>,
        subject: impl AsRef<str>,
    ) -> Result<String> {
        let tenant = tenant.as_ref().to_string();
        let sub = subject.as_ref().to_string();
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?;

        let claims = AuthTokenClaims {
            tenant,
            sub,
            iat: now.as_secs(),
            exp: now.as_secs() + 60 * 60 * 24 * 30, // TODO: hardcoded 30 days
        };

        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_base64_secret(&self.jwt_secret)?,
        )?;

        Ok(token)
    }

    pub fn verify_token(&self, token: impl AsRef<str>) -> Result<AuthTokenClaims> {
        let token = token.as_ref();
        let decoded = decode::<AuthTokenClaims>(
            token,
            &DecodingKey::from_base64_secret(&self.jwt_secret)?,
            &Validation::default(),
        )?;

        Ok(decoded.claims)
    }

    pub fn generate_registry_hmac(&self, claims: &RegistryRobotHmacClaims) -> Result<String> {
        let claims = claims.to_string();

        let hmac = blake3::keyed_hash(&self.registry_robot_hmac_secret, claims.as_bytes());
        let hmac = BASE64_URL_SAFE_NO_PAD.encode(hmac.as_bytes());

        Ok(hmac)
    }

    pub fn verify_registry_hmac(
        &self,
        hmac: impl AsRef<str>,
        claims: &RegistryRobotHmacClaims,
        service: impl AsRef<str>,
    ) -> Result<()> {
        let hmac = hmac.as_ref();
        let claims = claims.to_string();

        if service.as_ref() != self.registry_service {
            bail!("Invalid registry service");
        }

        let provided_hmac = BASE64_URL_SAFE_NO_PAD.decode(hmac)?;
        let provided_hmac = provided_hmac[..KEY_LEN].try_into()?;
        let provided_hamc = blake3::Hash::from_bytes(provided_hmac);

        let computed_hmac = blake3::keyed_hash(&self.registry_robot_hmac_secret, claims.as_bytes());

        if provided_hamc != computed_hmac {
            bail!("Invalid registry robot hmac");
        }

        Ok(())
    }

    pub fn generate_registry_token(
        &self,
        claims: &RegistryRobotHmacClaims,
        scopes: Vec<String>,
    ) -> Result<String> {
        #[derive(Serialize)]
        struct AccessEntry {
            #[serde(rename = "type")]
            typ: String,
            name: String,
            actions: Vec<String>,
        }
        #[derive(Serialize)]
        struct RegistryClaims<'a> {
            iss: &'a str,
            sub: String,
            aud: String,
            iat: u64,
            nbf: u64,
            exp: u64,
            jti: String,
            access: Vec<AccessEntry>,
        }

        // Parse scopes & enforce tenant boundary
        let tenant_prefix = format!("{}/", claims.tenant);
        let mut access: Vec<AccessEntry> = Vec::new();
        for raw in scopes {
            let mut parts = raw.splitn(3, ':');
            let typ = parts
                .next()
                .ok_or_else(|| anyhow::anyhow!("invalid scope: '{raw}'"))?;
            let name = parts
                .next()
                .ok_or_else(|| anyhow::anyhow!("invalid scope name in '{raw}'"))?;
            let actions_csv = parts
                .next()
                .ok_or_else(|| anyhow::anyhow!("invalid scope actions in '{raw}'"))?;
            if typ != "repository" {
                bail!("unsupported scope type '{typ}' in '{raw}'");
            }
            if !name.starts_with(&tenant_prefix) {
                bail!("scope '{}' is outside tenant '{}'", raw, claims.tenant);
            }
            let mut uniq = BTreeSet::new();
            for a in actions_csv.split(',') {
                match a.trim().to_lowercase().as_str() {
                    "pull" | "push" | "delete" | "*" => {
                        uniq.insert(a.trim().to_lowercase());
                    }
                    other => bail!("unsupported action '{other}' in '{raw}'"),
                }
            }
            let actions = uniq.into_iter().collect::<Vec<_>>();
            if actions.is_empty() {
                bail!("no valid actions in scope '{raw}'");
            }
            access.push(AccessEntry {
                typ: "repository".to_string(),
                name: name.to_string(),
                actions,
            });
        }

        // Build claims (add small skew)
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let iat = now.saturating_sub(30);
        let nbf = iat;
        let exp = now + 10 * 60; // 10 min token
        let sub = format!("{}/{}", claims.tenant, claims.sub);
        let jti = BASE64_URL_SAFE_NO_PAD.encode(
            blake3::hash(format!("{}:{}:{}", &sub, self.registry_service, iat).as_bytes())
                .as_bytes(),
        );

        let issuer = "lttle.cloud"; // or inject via your config
        let reg_claims = RegistryClaims {
            iss: issuer,
            sub,
            aud: self.registry_service.clone(),
            iat,
            nbf,
            exp,
            jti,
            access,
        };

        // Pick key/alg and sign
        let (enc_key, alg) = if let Ok(k) = EncodingKey::from_rsa_pem(&self.registry_token_key) {
            (k, jsonwebtoken::Algorithm::RS256)
        } else if let Ok(k) = EncodingKey::from_ec_pem(&self.registry_token_key) {
            (k, jsonwebtoken::Algorithm::ES256)
        } else {
            bail!("registry_token_key is not a valid RSA or EC private key (PEM)");
        };

        // IMPORTANT: embed x5c so the registry can fetch the signing key
        let mut header = Header::new(alg);
        header.typ = Some("JWT".into());
        // x5c wants base64 DER certs (no URL-safe here)
        header.x5c = Some(vec![self.registry_token_cert_der.clone()]);

        let token = encode(&header, &reg_claims, &enc_key)?;
        Ok(token)
    }
}
