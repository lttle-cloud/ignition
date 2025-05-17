mod net;
mod vms;

use std::{sync::Arc, time::Duration};

use net::{
    ip::{IpPool, IpPoolConfig},
    tap::{TapPool, TapPoolConfig},
};
use sds::Store;
use util::{
    async_runtime::{spawn, sync::RwLock, task::JoinHandle, time::sleep},
    result::Result,
};
use vms::instance::{Instance, InstanceConfig, MutableInstanceRef};

pub struct ControllerConfig {
    pub progress_frequency_hz: Option<u8>,
    pub vm_ip_cidr: String,
    pub svc_ip_cidr: String,
    pub bridge_name: String,
}

#[derive(Clone)]
pub struct Controller {
    vm_ip_pool: Arc<IpPool>,
    svc_ip_pool: Arc<IpPool>,
    tap_pool: Arc<TapPool>,
    instances: Arc<RwLock<Vec<MutableInstanceRef>>>,
    progress_task: Arc<RwLock<Option<JoinHandle<()>>>>,
    progress_frequency_hz: u8,
}

pub struct DeployRequest {
    pub instance_name: String,
    pub image: String,
}

impl Controller {
    pub async fn new(store: Store, config: ControllerConfig) -> Result<Self> {
        let vm_ip_pool = Arc::new(IpPool::new(
            IpPoolConfig {
                name: "vm".to_string(),
                cidr: config.vm_ip_cidr,
            },
            store.clone(),
        )?);

        let svc_ip_pool = Arc::new(IpPool::new(
            IpPoolConfig {
                name: "svc".to_string(),
                cidr: config.svc_ip_cidr,
            },
            store.clone(),
        )?);

        let tap_pool = Arc::new(
            TapPool::new(TapPoolConfig {
                bridge_name: config.bridge_name,
            })
            .await?,
        );

        let controller = Self {
            vm_ip_pool,
            svc_ip_pool,
            tap_pool,
            instances: Arc::new(RwLock::new(vec![])),
            progress_task: Arc::new(RwLock::new(None)),
            progress_frequency_hz: config.progress_frequency_hz.unwrap_or(10), // default 10hz
        };

        Ok(controller)
    }

    pub async fn get_instance(&self, name: &str) -> Option<MutableInstanceRef> {
        let instances = self.instances.read().await;
        for instance in instances.iter() {
            if instance.read().await.config.name == name {
                return Some(instance.clone());
            }
        }
        None
    }

    pub async fn deploy(&self, req: DeployRequest) -> Result<()> {
        println!("deploying instance: {}", req.instance_name);

        let Some(instance_ref) = self.get_instance(&req.instance_name).await else {
            let mut instances = self.instances.write().await;
            return self.create_instance(req, &mut instances).await;
        };

        let instance = instance_ref.read().await;
        println!("instance already exists: {}", instance.config.name);

        unimplemented!("update instance status");
    }

    async fn create_instance(
        &self,
        req: DeployRequest,
        instances: &mut Vec<MutableInstanceRef>,
    ) -> Result<()> {
        let instance = Instance::new(InstanceConfig {
            name: req.instance_name.clone(),
            image: req.image,
        })?;
        let instance_ref = instance.into_mutable_ref();
        instances.push(instance_ref);
        Ok(())
    }

    pub async fn destroy_instance(&self, name: &str) -> Result<()> {
        unimplemented!("destroy instance {}", name);
    }

    async fn progress(&self) {
        let instances = self.instances.read().await;
        for instance_ref in instances.iter() {
            let mut instance = instance_ref.write().await;
            if instance.can_progress() {
                instance.progress().await;
            }
            drop(instance);
        }

        // Sleep for a short duration to prevent busy-waiting
        sleep(Duration::from_millis(
            1000 / self.progress_frequency_hz as u64,
        ))
        .await;
    }

    pub async fn start_progress_task(&self) -> Result<()> {
        let self_clone = self.clone();
        let progress_task = spawn(async move {
            loop {
                println!("running progress loop");
                self_clone.progress().await;
            }
        });
        self.progress_task.write().await.replace(progress_task);

        Ok(())
    }

    pub async fn stop_progress_task(&self) {
        if let Some(progress_task) = self.progress_task.write().await.take() {
            progress_task.abort();
        }
    }
}
