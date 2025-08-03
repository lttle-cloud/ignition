use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthTokenClaims {
    pub tenant: String,
    pub sub: String,
    iat: u64,
    exp: u64,
}

pub struct AuthHandler {
    jwt_secret: String,
}

impl AuthHandler {
    pub fn new(jwt_secret: impl AsRef<str>) -> Self {
        Self {
            jwt_secret: jwt_secret.as_ref().to_string(),
        }
    }

    pub fn generate_token(&self, tenant: impl AsRef<str>) -> Result<String> {
        let tenant = tenant.as_ref().to_string();
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?;

        let claims = AuthTokenClaims {
            tenant,
            sub: "test".to_string(),
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
}
