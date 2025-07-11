use serde::{Deserialize, Serialize};

pub const DEFAULT_NAMESPACE: &str = "default";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Namespace {
    Unspecified,
    Default,
    Specified(String),
}

impl Namespace {
    pub fn as_value(&self) -> Option<String> {
        match self {
            Namespace::Unspecified => None,
            Namespace::Default => Some(DEFAULT_NAMESPACE.to_string()),
            Namespace::Specified(namespace) => Some(namespace.to_string()),
        }
    }

    pub fn from_value(value: Option<String>) -> Self {
        match value {
            None => Namespace::Unspecified,
            Some(val) if val == DEFAULT_NAMESPACE => Namespace::Default,
            Some(value) => Namespace::Specified(value),
        }
    }

    pub fn from_value_or_default(value: Option<String>) -> Self {
        match value {
            None => Namespace::Default,
            Some(val) if val == DEFAULT_NAMESPACE => Namespace::Default,
            Some(value) => Namespace::Specified(value),
        }
    }

    pub fn specified(value: impl AsRef<str>) -> Self {
        let value = value.as_ref().to_string();
        match value.as_str() {
            DEFAULT_NAMESPACE => Namespace::Default,
            _ => Namespace::Specified(value),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    pub name: String,
    pub namespace: Option<String>,
}

impl Metadata {
    pub fn new(name: impl AsRef<str>, namespace: Namespace) -> Self {
        Self {
            name: name.as_ref().to_string(),
            namespace: namespace.as_value(),
        }
    }
}
