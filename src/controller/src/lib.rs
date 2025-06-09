pub mod deployment;
pub mod image;
pub mod machine;
pub mod net;
pub mod volume;

use std::{
    collections::HashSet,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use dashmap::DashMap;
use deployment::{Deployment, DeploymentConfig, DeploymentRef};
use image::ImagePool;
use net::{ip::IpPool, tap::TapPool};
use sds::{Collection, Store};
use util::{
    async_runtime::{
        sync::Mutex,
        task::{self, JoinHandle},
        time::sleep,
    },
    encoding::codec,
    result::{Result, bail},
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
    pub log_dir_path: String,
}

struct SparkConnectionState {
    active_count: AtomicU32,
    timeout_task: Mutex<Option<JoinHandle<()>>>,
    deployment_name: String,
    cached_ip: Mutex<Option<String>>,
}

impl SparkConnectionState {
    fn new(deployment_name: String) -> Self {
        Self {
            active_count: AtomicU32::new(0),
            timeout_task: Mutex::new(None),
            deployment_name,
            cached_ip: Mutex::new(None),
        }
    }

    async fn cancel_timeout(&self) {
        if let Some(task) = self.timeout_task.lock().await.take() {
            task.abort();
            debug!("Cancelled timeout for deployment: {}", self.deployment_name);
        }
    }

    async fn get_timeout_ms(&self, controller: &Controller) -> u64 {
        // Get timeout from deployment config
        if let Ok(Some(stored)) = controller.get_deployment(&self.deployment_name).await {
            if let deployment::DeploymentMode::Spark { timeout_ms, .. } = stored.config.mode {
                return timeout_ms;
            }
        }
        // Default timeout
        10000 // 10 seconds
    }

    async fn start_timeout(&self, controller: Arc<Controller>) {
        let deployment_name = self.deployment_name.clone();
        let timeout_ms = self.get_timeout_ms(&controller).await;

        debug!(
            "Starting timeout for deployment '{}': {}ms",
            deployment_name, timeout_ms
        );

        let task = task::spawn(async move {
            sleep(Duration::from_millis(timeout_ms)).await;
            debug!(
                "Timeout reached for deployment '{}', suspending instance",
                deployment_name
            );

            if let Some(deployment_ref) = controller.active_deployments.get(&deployment_name) {
                let mut deployment = deployment_ref.lock().await;
                if let Err(e) = deployment.suspend_spark_instance().await {
                    error!(
                        "Failed to suspend Spark instance for '{}': {}",
                        deployment_name, e
                    );
                }
            }
        });

        *self.timeout_task.lock().await = Some(task);
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
    // Spark connection management
    spark_connections: DashMap<String, Arc<SparkConnectionState>>,
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
            spark_connections: DashMap::new(),
        }))
    }

    pub async fn deploy(&self, config: DeploymentConfig) -> Result<StoredDeployment> {
        // Validate the deployment config
        config.validate()?;

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
                .cleanup_all_instances(self.image_pool.clone(), &self.tap_pool)
                .await?;

            debug!("Active deployment cleaned up");
        }

        // Clean up Spark connection state and cancel any pending timeouts
        if let Some((_key, spark_state)) = self.spark_connections.remove(id) {
            spark_state.cancel_timeout().await;
            debug!("Spark connection state cleaned up for deployment: {}", id);
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

        // First, sync machine status changes with deployment instance status
        deployment.sync_machine_status_changes().await?;

        // Update Spark-specific status transitions based on instance states
        if deployment.is_spark() {
            deployment.update_spark_status().await;
        }

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
                    self.config.log_dir_path.clone(),
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

    /// Open a connection to a Spark deployment, ensuring the instance is running
    pub async fn open_connection(self: &Arc<Self>, deployment_name: String) -> Result<String> {
        let start_time = std::time::Instant::now();

        // Get or create connection state
        let spark_state = self
            .spark_connections
            .entry(deployment_name.clone())
            .or_insert_with(|| Arc::new(SparkConnectionState::new(deployment_name.clone())));

        // Cancel any pending timeout
        spark_state.cancel_timeout().await;

        // Increment connection counter
        let count = spark_state.active_count.fetch_add(1, Ordering::SeqCst);
        debug!(
            "Opening connection to '{}', active count: {} -> {}",
            deployment_name,
            count,
            count + 1
        );

        // If first connection, ensure instance is running
        if count == 0 {
            self.ensure_spark_instance_running(&deployment_name).await?;
        }

        // Get IP address (from cache or fresh)
        let ip = self
            .get_spark_instance_ip(&deployment_name, &spark_state)
            .await?;

        let duration = start_time.elapsed();
        info!(
            "🚀 Connection opened to '{}' in {:.2}ms (IP: {})",
            deployment_name,
            duration.as_secs_f64() * 1000.0,
            ip
        );

        Ok(ip)
    }

    /// Close a connection to a Spark deployment, starting timeout if it was the last connection
    pub async fn close_connection(self: &Arc<Self>, deployment_name: String) -> Result<()> {
        if let Some(spark_state) = self.spark_connections.get(&deployment_name) {
            let count = spark_state.active_count.fetch_sub(1, Ordering::SeqCst);
            debug!(
                "Closing connection to '{}', active count: {} -> {}",
                deployment_name,
                count,
                count - 1
            );

            if count == 1 {
                // Was 1, now 0
                debug!(
                    "Last connection closed for '{}', starting timeout",
                    deployment_name
                );
                spark_state.start_timeout(self.clone()).await;
            }
        } else {
            warn!(
                "Attempted to close connection to unknown deployment: {}",
                deployment_name
            );
        }
        Ok(())
    }

    async fn ensure_spark_instance_running(&self, deployment_name: &str) -> Result<()> {
        debug!(
            "Ensuring Spark instance is running for deployment: {}",
            deployment_name
        );

        let Some(deployment_ref) = self.active_deployments.get(deployment_name) else {
            bail!("Deployment not found: {}", deployment_name);
        };

        let mut deployment = deployment_ref.lock().await;

        // Verify this is a Spark deployment
        if !deployment.is_spark() {
            bail!("Deployment '{}' is not in Spark mode", deployment_name);
        }

        // Check the actual instance status, not just deployment status
        let needs_resume = match deployment.status {
            deployment::DeploymentStatus::ReadyToResume => {
                debug!("Deployment is in ReadyToResume state");
                true
            }
            deployment::DeploymentStatus::Ready => {
                // Even if deployment is Ready, check if instance is actually suspended
                let instance_suspended = deployment
                    .instances
                    .values()
                    .any(|i| matches!(i.status, crate::deployment::InstanceStatus::Suspended));

                if instance_suspended {
                    debug!("Deployment is Ready but instance is suspended, needs resume");
                    true
                } else {
                    debug!("Spark instance already running for '{}'", deployment_name);
                    false
                }
            }
            _ => {
                bail!(
                    "Deployment '{}' is not ready for connections (status: {:?})",
                    deployment_name,
                    deployment.status
                );
            }
        };

        if needs_resume {
            debug!(
                "Resuming suspended Spark instance for '{}'",
                deployment_name
            );
            deployment.resume_spark_instance().await?;

            // Wait for the machine to actually be ready
            let instance_id = deployment.instances.keys().next().cloned();
            if let Some(instance_id) = instance_id {
                if let Some(machine) = deployment.get_machine(&instance_id) {
                    let mut rx = machine.status_rx().await;

                    loop {
                        match rx.recv().await {
                            Ok(status) => {
                                debug!("Machine status during resume: {:?}", status);
                                if matches!(status, crate::machine::MachineStatus::Ready) {
                                    debug!("Machine is ready after resume");
                                    break;
                                }
                                if matches!(status, crate::machine::MachineStatus::Error(_)) {
                                    bail!("Machine failed during resume: {:?}", status);
                                }
                            }
                            Err(_) => {
                                // ignore
                            }
                        }
                    }

                    // Now sync the status changes
                    deployment.sync_machine_status_changes().await?;
                    if deployment.is_spark() {
                        deployment.update_spark_status().await;
                    }
                }
            }
        }

        Ok(())
    }

    async fn get_spark_instance_ip(
        &self,
        deployment_name: &str,
        spark_state: &SparkConnectionState,
    ) -> Result<String> {
        // Check cache first
        {
            let cached_ip = spark_state.cached_ip.lock().await;
            if let Some(ip) = &*cached_ip {
                return Ok(ip.clone());
            }
        }

        // Get IP from deployment
        let Some(deployment_ref) = self.active_deployments.get(deployment_name) else {
            bail!("Deployment not found: {}", deployment_name);
        };

        let deployment = deployment_ref.lock().await;
        let Some(ip) = deployment.get_spark_instance_ip().await else {
            bail!("No IP found for Spark deployment: {}", deployment_name);
        };

        // Cache the IP
        {
            let mut cached_ip = spark_state.cached_ip.lock().await;
            *cached_ip = Some(ip.clone());
        }

        debug!("Got IP for '{}': {}", deployment_name, ip);
        Ok(ip)
    }
}
