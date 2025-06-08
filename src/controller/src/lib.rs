pub mod deployment;
pub mod image;
pub mod machine;
pub mod net;
pub mod volume;

use std::{collections::HashSet, sync::Arc, time::Duration};

use dashmap::DashMap;
use deployment::{Deployment, DeploymentConfig, DeploymentRef};
use image::ImagePool;
use net::{ip::IpPool, tap::TapPool};
use sds::{Collection, Store};
use util::{
    async_runtime::time::sleep,
    encoding::codec,
    result::Result,
    tracing::{debug, error, info, warn},
};

#[codec]
#[derive(Clone, Debug)]
pub struct StoredDeployment {
    pub id: String,
    pub config: DeploymentConfig,
    pub gateway: String,
    pub netmask: String,
    pub status: deployment::DeploymentStatus,
    pub created_at: u128,
    pub updated_at: u128,
    // Store instances separately
    pub instances: Vec<deployment::Instance>,
}

impl From<&Deployment> for StoredDeployment {
    fn from(deployment: &Deployment) -> Self {
        Self {
            id: deployment.id.clone(),
            config: deployment.config.clone(),
            gateway: deployment.gateway.clone(),
            netmask: deployment.netmask.clone(),
            status: deployment.status.clone(),
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis(),
            updated_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis(),
            instances: deployment.instances.values().cloned().collect(),
        }
    }
}

#[derive(Clone)]
pub struct ControllerConfig {
    pub reconcile_interval_secs: u64,
}

impl Default for ControllerConfig {
    fn default() -> Self {
        Self {
            reconcile_interval_secs: 5,
        }
    }
}

pub struct Controller {
    config: ControllerConfig,
    store: Store,
    image_pool: Arc<ImagePool>,
    tap_pool: TapPool,
    ip_pool: IpPool,
    deployments_collection: Collection<StoredDeployment>,
    // Active deployments being reconciled
    active_deployments: DashMap<String, DeploymentRef>,
}

impl Controller {
    pub fn new(
        config: ControllerConfig,
        store: Store,
        image_pool: Arc<ImagePool>,
        tap_pool: TapPool,
        ip_pool: IpPool,
    ) -> Result<Arc<Self>> {
        let deployments_collection = store.collection::<StoredDeployment>("deployments")?;

        Ok(Arc::new(Self {
            config,
            store,
            image_pool,
            tap_pool,
            ip_pool,
            deployments_collection,
            active_deployments: DashMap::new(),
        }))
    }

    pub async fn deploy(&self, config: DeploymentConfig) -> Result<StoredDeployment> {
        info!("Processing deployment: {}", config.name);
        debug!("Image: {}", config.image);
        debug!(
            "Resources: {}MB RAM, {} vCPUs, {} replicas",
            config.memory_mib, config.vcpu_count, config.replicas
        );

        // Check if deployment already exists
        let txn = self.store.read_txn()?;
        let existing_stored = txn.get(&self.deployments_collection, &config.name);
        drop(txn);

        if let Some(existing_stored) = existing_stored {
            debug!(
                "Deployment {} already exists, checking configuration...",
                config.name
            );

            // Compare configurations
            let new_hash = config.compute_hash();
            let existing_hash = existing_stored.config.compute_hash();

            if new_hash == existing_hash {
                debug!("Configuration unchanged, returning existing deployment");
                return Ok(existing_stored);
            } else {
                info!("Configuration changed, updating deployment");
                debug!(
                    "Old replicas: {}, New replicas: {}",
                    existing_stored.config.replicas, config.replicas
                );
                if existing_stored.config.image != config.image {
                    debug!(
                        "Image changed: {} -> {}",
                        existing_stored.config.image, config.image
                    );
                }

                // Update the active deployment if it exists
                if let Some(deployment_ref) = self.active_deployments.get(&config.name) {
                    let mut deployment = deployment_ref.lock().await;
                    deployment.update_config(config.clone());
                } else {
                    // Deployment exists in store but not in active deployments, recreate it
                    debug!("Deployment not active, recreating from stored state");
                    let deployment = Deployment::from_stored(existing_stored.clone()).await?;
                    let deployment_guard = deployment.into_ref();
                    {
                        let mut deployment = deployment_guard.lock().await;
                        deployment.update_config(config.clone());
                    }
                    self.active_deployments
                        .insert(config.name.clone(), deployment_guard);
                }

                // Return updated stored deployment
                let updated_stored = StoredDeployment {
                    config,
                    ..existing_stored
                };

                // Store the updated config
                let mut txn = self.store.write_txn()?;
                txn.put(
                    &self.deployments_collection,
                    &updated_stored.id,
                    &updated_stored,
                )?;
                txn.commit()?;

                debug!("Deployment {} configuration updated", updated_stored.id);
                return Ok(updated_stored);
            }
        }

        // Create new deployment
        debug!("Creating new deployment");
        let deployment_id = config.name.clone(); //TODO: use uuid
        let deployment = Deployment::new(deployment_id, config).await?;
        let stored_deployment = StoredDeployment::from(&deployment);

        debug!(
            "Instances: {} will be created",
            stored_deployment.config.replicas
        );
        debug!("Gateway: {}", stored_deployment.gateway);

        // Store it
        let mut txn = self.store.write_txn()?;
        txn.put(
            &self.deployments_collection,
            &stored_deployment.id,
            &stored_deployment,
        )?;
        txn.commit()?;

        // Add to active deployments
        self.active_deployments
            .insert(stored_deployment.id.clone(), deployment.into_ref());

        info!("Deployment {} created successfully", stored_deployment.id);
        Ok(stored_deployment)
    }

    pub async fn get_deployment(&self, name: &str) -> Result<Option<StoredDeployment>> {
        let txn = self.store.read_txn()?;
        let deployment = txn.get(&self.deployments_collection, name);
        Ok(deployment)
    }

    pub async fn list_deployments(&self) -> Result<Vec<StoredDeployment>> {
        let txn = self.store.read_txn()?;
        let deployments = txn.get_all_values(&self.deployments_collection)?;
        Ok(deployments)
    }

    pub async fn list_tracked_resources(&self) -> Result<(Vec<String>, Vec<String>)> {
        let deployments = self.list_deployments().await?;
        let mut images = HashSet::new();
        let mut volumes = HashSet::new();

        for deployment in deployments {
            for instance in deployment.instances {
                if let Some(image_id) = instance.image_id {
                    images.insert(image_id);
                }
                if let Some(volume_id) = instance.rootfs_volume_id {
                    volumes.insert(volume_id);
                }
            }
        }

        Ok((images.into_iter().collect(), volumes.into_iter().collect()))
    }

    pub async fn delete_deployment(&self, id: &str) -> Result<()> {
        info!("Deleting deployment: {}", id);

        // Stop and clean up the deployment if it's active
        if let Some((_key, deployment_ref)) = self.active_deployments.remove(id) {
            let mut deployment = deployment_ref.lock().await;
            deployment.cancel();

            // Wait a moment for graceful shutdown
            util::async_runtime::time::sleep(std::time::Duration::from_millis(100)).await;

            // Force cleanup all instances (this will delete volumes)
            deployment
                .cleanup_all_instances(self.image_pool.clone())
                .await?;

            debug!("Active deployment cleaned up");
        }

        // Remove from persistent storage
        let mut txn = self.store.write_txn()?;
        txn.del(&self.deployments_collection, id)?;
        txn.commit()?;

        info!("Deployment '{}' deleted successfully", id);
        Ok(())
    }

    pub async fn run_reconciliation(&self) -> Result<()> {
        debug!(
            "Starting reconciliation loop with interval {}s",
            self.config.reconcile_interval_secs
        );

        // Load existing deployments from store
        self.load_deployments_from_store().await?;
        let interval = Duration::from_secs(self.config.reconcile_interval_secs);

        loop {
            debug!("Running reconciliation cycle...");
            match self.reconcile_once_simple().await {
                Ok(()) => {
                    debug!("Reconciliation cycle completed");
                }
                Err(e) => {
                    error!("Reconciliation error: {}", e);
                }
            }

            sleep(interval).await;
        }
    }

    async fn load_deployments_from_store(&self) -> Result<()> {
        let stored_deployments = self.list_deployments().await?;

        for stored in stored_deployments {
            if matches!(stored.status, deployment::DeploymentStatus::Stopped) {
                continue; // Skip stopped deployments
            }

            match Deployment::from_stored(stored.clone()).await {
                Ok(deployment) => {
                    self.active_deployments
                        .insert(deployment.config.name.clone(), deployment.into_ref());
                }
                Err(e) => {
                    warn!("Failed to restore deployment {}: {}", stored.id, e);
                }
            }
        }

        info!(
            "Loaded {} active deployments from store",
            self.active_deployments.len()
        );
        Ok(())
    }

    // Simplified reconciliation that doesn't maintain active state
    async fn reconcile_once_simple(&self) -> Result<()> {
        for deployment in self.active_deployments.iter() {
            let mut deployment = deployment.lock().await;

            // Skip finished deployments
            if matches!(
                deployment.status,
                deployment::DeploymentStatus::Ready | deployment::DeploymentStatus::Stopped
            ) {
                continue;
            }

            debug!(
                "Reconciling deployment: {} (status: {:?})",
                deployment.config.name, deployment.status
            );

            let before_status = deployment.status.clone();
            let result = self.reconcile_deployment(&mut deployment).await;

            match result {
                Ok(()) => {
                    if deployment.status != before_status {
                        info!(
                            "Status changed: {:?} -> {:?}",
                            before_status, deployment.status
                        );
                        // Update the stored deployment status
                        let deployment: &Deployment = &deployment;
                        let mut stored = StoredDeployment::from(deployment);
                        stored.status = deployment.status.clone();
                        stored.updated_at = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_millis();

                        let mut txn = self.store.write_txn()?;
                        txn.put(&self.deployments_collection, &stored.id, &stored)?;
                        txn.commit()?;

                        debug!(
                            "Updated deployment {} status to {:?}",
                            stored.id, deployment.status
                        );
                    } else {
                        debug!("No status change for deployment {}", deployment.id);
                    }
                }
                Err(e) => {
                    error!("Failed to reconcile deployment {}: {}", deployment.id, e);

                    // Mark as stopped on error
                    let deployment: &Deployment = &deployment;
                    let mut stored = StoredDeployment::from(deployment);
                    stored.status = deployment::DeploymentStatus::Stopped;
                    stored.updated_at = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_millis();

                    let mut txn = self.store.write_txn()?;
                    txn.put(&self.deployments_collection, &stored.id, &stored)?;
                    txn.commit()?;

                    warn!("Marked deployment {} as stopped due to error", stored.id);
                }
            }
        }

        Ok(())
    }

    // Reconcile a single deployment and return its new status
    async fn reconcile_deployment(&self, deployment: &mut Deployment) -> Result<()> {
        debug!(
            "Reconciling deployment {} in state {:?}",
            deployment.config.name, deployment.status
        );

        if !deployment.is_finished() {
            debug!(
                "Progressing deployment {} (not finished)",
                deployment.config.name
            );
            deployment
                .progress(
                    self.image_pool.clone(),
                    self.tap_pool.clone(),
                    self.ip_pool.clone(),
                )
                .await?;
        } else {
            debug!(
                "Deployment {} is finished with status {:?}",
                deployment.config.name, deployment.status
            );
        }

        Ok(())
    }
}
