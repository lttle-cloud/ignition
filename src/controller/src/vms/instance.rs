use std::{sync::Arc, time::Duration};

use util::{
    async_runtime::{spawn, sync::RwLock, time::sleep},
    result::Result,
};

use super::pipeline::{InstancePipeline, InstanceStatus};

#[derive(Debug, Clone)]
pub struct InstanceConfig {
    pub name: String,
    pub image: String,
}

#[derive(Debug)]
pub struct Instance {
    pub config: InstanceConfig,
    pub pipeline: InstancePipeline,
}

pub type MutableInstanceRef = Arc<RwLock<Instance>>;

impl Instance {
    pub fn new(config: InstanceConfig) -> Result<Self> {
        let pipeline = InstancePipeline::new();
        Ok(Self { config, pipeline })
    }

    pub fn into_mutable_ref(self) -> MutableInstanceRef {
        Arc::new(RwLock::new(self))
    }

    pub fn get_status(&self) -> InstanceStatus {
        self.pipeline.get_status()
    }

    pub fn can_progress(&self) -> bool {
        match self.get_status() {
            InstanceStatus::Ready | InstanceStatus::Stopped => false,
            _ => true,
        }
    }

    pub async fn progress(&mut self) {
        if let Some(status) = self.pipeline.can_progress() {
            let config = self.config.clone();

            match status {
                InstanceStatus::Pending => {
                    self.pipeline.go_to_next_status();
                }
                InstanceStatus::ImagePulling => {
                    self.pipeline.start_progress(spawn(async move {
                        println!("pulling image {}", config.image);
                        sleep(Duration::from_secs(5)).await;
                        println!("image pulled");
                    }));
                }
                InstanceStatus::ImageReady => {
                    self.pipeline.go_to_next_status();
                }
                InstanceStatus::NetworkSetup => {
                    self.pipeline.start_progress(spawn(async move {
                        println!("setting up network");
                        sleep(Duration::from_secs(5)).await;
                        println!("network setup");
                    }));
                }
                InstanceStatus::Booting => {
                    self.pipeline.start_progress(spawn(async move {
                        println!("booting instance");
                        sleep(Duration::from_secs(5)).await;
                        println!("instance booted");
                    }));
                }
                InstanceStatus::Ready => {
                    println!("instance is ready");
                    self.pipeline.go_to_status(InstanceStatus::Stopping);
                }
                InstanceStatus::Stopping => {
                    self.pipeline.start_progress(spawn(async move {
                        println!("stopping instance");
                        sleep(Duration::from_secs(2)).await;
                        println!("instance stopped");
                    }));
                }
                InstanceStatus::Stopped => {
                    println!("instance is stopped");
                }
            }
        }
    }
}
