mod net;
mod vms;

use std::{collections::HashMap, sync::Arc, time::Duration};

use net::{
    ip::{IpPool, IpPoolConfig},
    tap::{TapPool, TapPoolConfig},
};
use sds::Store;
use util::{
    async_runtime::{spawn, sync::Mutex, time::sleep},
    result::Result,
};
use vms::instance::{Instance, InstanceConfig};

pub struct ControllerConfig {
    vm_ip_cidr: String,
    svc_ip_cidr: String,
    bridge_name: String,
}

#[derive(Clone)]
pub struct Controller {
    vm_ip_pool: Arc<IpPool>,
    svc_ip_pool: Arc<IpPool>,
    tap_pool: Arc<TapPool>,
    instances: Arc<Mutex<Vec<Instance>>>,
}

pub struct DeployRequest {
    instance_name: String,
    image: String,
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
            instances: Arc::new(Mutex::new(vec![])),
        };

        // Spawn the progress loop
        let controller_clone = controller.clone();
        spawn(async move {
            controller_clone.progress().await;
        });

        Ok(controller)
    }

    pub async fn deploy(&self, req: DeployRequest) -> Result<()> {
        let mut instances = self.instances.lock().await;

        let Some(instance) = instances
            .iter()
            .find(|instance| instance.config.name == req.instance_name)
            .cloned()
        else {
            return self.create_instance(req, &mut instances).await;
        };

        println!("instance already exists: {}", instance.config.name);

        unimplemented!("update instance status");
    }

    async fn create_instance(
        &self,
        req: DeployRequest,
        instances: &mut Vec<Instance>,
    ) -> Result<()> {
        let instance = Instance::new(InstanceConfig {
            name: req.instance_name.clone(),
            image: req.image,
        })?;
        instances.push(instance.clone());
        Ok(())
    }
    pub async fn list_instances(&self) -> Result<Vec<Instance>> {
        let instances = self.instances.lock().await;
        Ok(instances.clone())
    }

    pub async fn get_instance(&self, name: &str) -> Option<Instance> {
        let instances = self.instances.lock().await;
        instances
            .iter()
            .find(|instance| instance.config.name == name)
            .cloned()
    }

    pub async fn destroy_instance(&self, name: &str) -> Result<()> {
        unimplemented!("destroy instance {}", name);
    }

    async fn progress(&self) {
        let mut instances = self.instances.lock().await;

        // Drop the lock before sleeping
        drop(instances);

        // Sleep for a short duration to prevent busy-waiting
        sleep(Duration::from_millis(100)).await;
    }
}
