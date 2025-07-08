use serde::{Deserialize, Serialize};

pub const DEFAULT_NAMESPACE: &str = "default";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    pub name: String,
    pub namespace: String,
}

impl Metadata {
    pub fn new(name: impl AsRef<str>, namespace: Option<impl AsRef<str>>) -> Self {
        let namespace = if let Some(namespace) = namespace {
            namespace.as_ref().to_string()
        } else {
            DEFAULT_NAMESPACE.to_string()
        };

        Self {
            name: name.as_ref().to_string(),
            namespace,
        }
    }
}
