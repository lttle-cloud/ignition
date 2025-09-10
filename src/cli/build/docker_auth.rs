use std::collections::HashMap;

use anyhow::Result;
use base64::{Engine, prelude::BASE64_URL_SAFE};
use serde::Serialize;

#[derive(Clone, Serialize)]
pub struct DockerAuthConfig {
    auths: HashMap<String, DockerAuth>,
}

impl DockerAuthConfig {
    pub fn internal(registry: &str, user: &str, pass: &str) -> Self {
        Self {
            auths: HashMap::from([(registry.to_string(), DockerAuth::new(user, pass))]),
        }
    }

    pub fn get_registry(&self) -> Option<&String> {
        self.auths.keys().next()
    }

    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }
}

#[derive(Clone, Serialize)]
struct DockerAuth {
    auth: String,
}

impl DockerAuth {
    fn new(user: &str, pass: &str) -> Self {
        let auth = format!("{}:{}", user, pass);
        let auth = BASE64_URL_SAFE.encode(auth.as_bytes());
        Self { auth }
    }
}
