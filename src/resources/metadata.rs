use serde::{Deserialize, Serialize};

const DEFAULT_NAMESPACE: &str = "default";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    pub name: String,
    pub namespace: String,
}

impl Metadata {
    pub fn new(name: String, namespace: Option<String>) -> Self {
        Self {
            name,
            namespace: namespace.unwrap_or_else(|| DEFAULT_NAMESPACE.to_string()),
        }
    }
}
