use std::time::Duration;

use util::{async_runtime::time::sleep, result::Result};

#[derive(Debug, Clone)]
pub struct InstanceConfig {
    pub name: String,
    pub image: String,
}

#[derive(Debug, Clone)]
pub struct Instance {
    pub config: InstanceConfig,
}

impl Instance {
    pub fn new(config: InstanceConfig) -> Result<Self> {
        Ok(Self { config })
    }

    pub async fn bring_up(&mut self) {}

    pub async fn bring_down(&mut self) {}

    pub async fn pull_image(&mut self) {
        // TODO: Implement actual image pulling with cancellation support. sleep for 10 seconds.
        sleep(Duration::from_secs(10)).await;
    }
}
