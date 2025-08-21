#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct CertProvider {
    pub name: String,
    #[serde(rename = "acme-base-url")]
    pub acme_base_url: String,
    #[serde(rename = "default-email")]
    pub default_email: Option<String>,
    #[serde(rename = "api-key")]
    pub api_key: Option<String>,
    #[serde(rename = "environment")]
    pub environment: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CertificateAgentConfig {
    pub providers: Vec<CertProvider>,
    pub certs_base_dir: String,
}
