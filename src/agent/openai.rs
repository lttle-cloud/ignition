use async_openai::{Client, config::OpenAIConfig};

pub type OpenAIClient = Client<OpenAIConfig>;

#[derive(Debug, Clone)]
pub struct OpenAIAgentConfig {
    pub api_key: String,
    pub default_model: String,
}

pub struct OpenAIAgent {
    config: OpenAIAgentConfig,
}

impl OpenAIAgent {
    pub fn new(config: OpenAIAgentConfig) -> Self {
        Self { config }
    }

    pub fn get_api_client(&self) -> OpenAIClient {
        OpenAIClient::with_config(OpenAIConfig::new().with_api_key(self.config.api_key.clone()))
    }

    pub fn get_default_model(&self) -> &str {
        &self.config.default_model
    }
}
