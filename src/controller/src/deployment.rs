use std::{collections::HashMap, fmt::Debug, sync::Arc};

use oci_client::Reference;
use util::{
    async_runtime::{
        sync::Mutex,
        task::{self, JoinHandle},
    },
    encoding::codec,
    result::{Result, bail},
    tracing::{debug, error, info, warn},
    uuid,
};

use crate::{
    image::{ImagePool, PullPolicy},
    machine::{Machine, MachineConfig, MachineStatus, SparkSnapshotPolicy},
    net::{ip::IpPool, tap::TapPool},
};

#[codec]
#[derive(Clone, Debug, PartialEq)]
pub enum DeploymentStatus {
    New,
    PullingImage,
    CreatingInstances,
    WaitingForInstances,
    Ready,
    Suspended,
    Stopping,
    Stopped,
    // New status for when config changes require replacement
    Replacing,
}

#[codec]
#[derive(Clone, Debug, PartialEq)]
pub enum InstanceStatus {
    New,
    Creating,
    Starting,
    Running,
    Ready,
    Stopping,
    Stopped,
    Error(String),
}

#[codec]
#[derive(Clone, Debug)]
pub struct Instance {
    pub id: String,
    pub status: InstanceStatus,
    pub tap_name: String,
    pub ip_addr: String,
    pub log_file_path: String,
    pub created_at: u128,
    pub image_id: Option<String>,
    pub rootfs_volume_id: Option<String>,
}

struct Tasks {
    pub pull_image: Option<JoinHandle<Result<crate::image::Image>>>,
    pub instance_tasks: HashMap<String, JoinHandle<Result<()>>>,
}

impl Tasks {
    fn new() -> Self {
        Self {
            pull_image: None,
            instance_tasks: HashMap::new(),
        }
    }

    fn cancel_all(&mut self) {
        if let Some(task) = self.pull_image.take() {
            task.abort();
        }
        for (_, task) in self.instance_tasks.drain() {
            task.abort();
        }
    }
}

pub struct Deployment {
    pub id: String,
    pub status: DeploymentStatus,
    pub config: DeploymentConfig,
    pub gateway: String,
    pub netmask: String,
    tasks: Tasks,
    // Track all instances
    pub instances: HashMap<String, Instance>,
    // Track actual machines for each instance
    machines: HashMap<String, Machine>,
    // Configuration hash to detect changes
    config_hash: String,
}

pub type DeploymentRef = Arc<Mutex<Deployment>>;

#[codec]
#[derive(Clone, Debug)]
pub enum DeploymentMode {
    Normal,
    Spark {
        timeout_ms: u64,
        snapshot_policy: SparkSnapshotPolicy,
    },
}

#[codec]
#[derive(Clone, Debug)]
pub struct DeploymentConfig {
    pub name: String,
    pub image: String,
    pub mode: DeploymentMode,
    pub image_pull_policy: PullPolicy,
    pub vcpu_count: u8,
    pub memory_mib: usize,
    pub envs: Vec<String>,
    // Add replica count - defaults to 1
    pub replicas: u32,
}

impl DeploymentConfig {
    pub fn compute_hash(&self) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        self.name.hash(&mut hasher);
        self.image.hash(&mut hasher);
        format!("{:?}", self.image_pull_policy).hash(&mut hasher);
        self.vcpu_count.hash(&mut hasher);
        self.memory_mib.hash(&mut hasher);
        self.envs.hash(&mut hasher);
        self.replicas.hash(&mut hasher);

        format!("{:x}", hasher.finish())
    }
}

impl Deployment {
    pub async fn new(id: String, config: DeploymentConfig) -> Result<Self> {
        let config_hash = config.compute_hash();

        Ok(Self {
            id,
            status: DeploymentStatus::New,
            config,
            gateway: "".to_string(), // Will be set when needed
            netmask: "".to_string(), // Will be set when needed
            tasks: Tasks::new(),
            instances: HashMap::new(),
            machines: HashMap::new(),
            config_hash,
        })
    }

    pub async fn from_stored(stored: crate::StoredDeployment) -> Result<Self> {
        let config_hash = stored.config.compute_hash();

        // Convert stored instances to HashMap and reset their status
        // since Machine objects don't persist across restarts
        let instances: HashMap<String, Instance> = stored
            .instances
            .into_iter()
            .map(|mut instance| {
                // Reset instance status so they get recreated
                instance.status = InstanceStatus::New;
                (instance.id.clone(), instance)
            })
            .collect();

        Ok(Self {
            id: stored.id,
            status: DeploymentStatus::New, // Start from the beginning
            config: stored.config,
            gateway: stored.gateway,
            netmask: stored.netmask,
            tasks: Tasks::new(),
            instances,
            machines: HashMap::new(), // Will be reconstructed
            config_hash,
        })
    }

    pub fn into_ref(self) -> DeploymentRef {
        Arc::new(Mutex::new(self))
    }

    pub fn update_config(&mut self, new_config: DeploymentConfig) -> bool {
        let new_hash = new_config.compute_hash();
        if new_hash != self.config_hash {
            info!(
                "Configuration change detected for deployment '{}'",
                self.config.name
            );
            debug!("Old hash: {}", self.config_hash);
            debug!("New hash: {}", new_hash);

            self.config = new_config;
            self.config_hash = new_hash;

            // Handle different deployment states
            match self.status {
                DeploymentStatus::Ready => {
                    info!("Triggering instance replacement due to config change");
                    self.status = DeploymentStatus::Replacing;
                }
                DeploymentStatus::PullingImage
                | DeploymentStatus::CreatingInstances
                | DeploymentStatus::WaitingForInstances => {
                    info!("Canceling in-progress deployment due to config change");
                    // Cancel all current tasks
                    self.tasks.cancel_all();
                    // Clean up any partial instances
                    let instance_ids: Vec<_> = self.instances.keys().cloned().collect();
                    for instance_id in instance_ids {
                        if let Some(instance) = self.instances.get(&instance_id) {
                            // Only cleanup instances that are not ready yet
                            if !matches!(instance.status, InstanceStatus::Ready) {
                                debug!("Cleaning up partial instance: {}", instance_id);
                                // Remove from instances map - cleanup will happen in next progress cycle
                                self.instances.remove(&instance_id);
                            }
                        }
                    }
                    // Restart from the beginning
                    self.status = DeploymentStatus::New;
                }
                DeploymentStatus::New => {
                    // Config changed before we even started - just use new config
                    debug!("Using new configuration for deployment that hasn't started yet");
                }
                DeploymentStatus::Replacing => {
                    // Already replacing - just update config, replacement will use new config
                    debug!("Updating configuration during replacement");
                }
                DeploymentStatus::Suspended
                | DeploymentStatus::Stopping
                | DeploymentStatus::Stopped => {
                    // Don't automatically restart these
                    debug!(
                        "Configuration updated but deployment is {:?}, no action taken",
                        self.status
                    );
                }
            }
            true
        } else {
            false
        }
    }

    pub async fn progress(
        &mut self,
        image_pool: Arc<ImagePool>,
        tap_pool: TapPool,
        ip_pool: IpPool,
        logs_dir_path: String,
    ) -> Result<()> {
        debug!(
            "Progressing deployment '{}' from {:?}",
            self.config.name, self.status
        );

        match self.status {
            DeploymentStatus::New => {
                debug!("Starting new deployment - initiating image pull");

                // First, clean up any partial instances from previous attempts
                if !self.instances.is_empty() {
                    debug!(
                        "Cleaning up {} partial instances from previous attempt",
                        self.instances.len()
                    );
                    self.cleanup_partial_instances(image_pool.clone()).await?;
                }

                self.status = DeploymentStatus::PullingImage;

                let image_pool = image_pool.clone();
                let reference: Reference = self.config.image.parse()?;
                let policy = self.config.image_pull_policy.clone();

                debug!("Pulling image: {} with policy {:?}", reference, policy);

                let pull_task = task::spawn(async move {
                    image_pool.pull_image_if_needed(&reference, policy).await
                });

                self.tasks.pull_image = Some(pull_task);
            }

            DeploymentStatus::PullingImage => {
                if let Some(ref task) = self.tasks.pull_image {
                    if task.is_finished() {
                        debug!("Image pull task completed, checking result");
                        let result = self.tasks.pull_image.take().unwrap().await;
                        match result {
                            Ok(Ok(_image)) => {
                                debug!("Image pull successful, moving to instance creation");
                                self.status = DeploymentStatus::CreatingInstances;
                            }
                            Ok(Err(e)) => {
                                error!("Image pull failed: {}", e);
                                self.status = DeploymentStatus::Stopped;
                                return Err(e);
                            }
                            Err(e) => {
                                error!("Image pull task failed: {}", e);
                                self.status = DeploymentStatus::Stopped;
                                return Err(e.into());
                            }
                        }
                    } else {
                        debug!("Image pull still in progress...");
                    }
                }
            }

            DeploymentStatus::CreatingInstances => {
                debug!("Creating {} instances for deployment", self.config.replicas);

                // Set gateway and netmask from IP pool if not already set
                if self.gateway.is_empty() {
                    self.gateway = ip_pool.gateway().to_string();
                    self.netmask = ip_pool.netmask().to_string();
                }

                // Create instances if we don't have enough
                while self.instances.len() < self.config.replicas as usize {
                    let instance_id =
                        format!("{}-{}", self.config.name, uuid::Uuid::new_v4().to_string());

                    // Reserve network resources for this instance
                    let tap_name = tap_pool.create_tap().await?;
                    let ip_addr = ip_pool.reserve_tagged(&format!(
                        "deployment_{}_{}",
                        self.config.name, instance_id
                    ))?;
                    let log_file_path = format!("{}/{}", logs_dir_path, instance_id);

                    let instance = Instance {
                        id: instance_id.clone(),
                        status: InstanceStatus::New,
                        tap_name,
                        log_file_path,
                        ip_addr: ip_addr.addr.to_string(),
                        created_at: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap()
                            .as_millis(),
                        image_id: None,
                        rootfs_volume_id: None,
                    };

                    debug!(
                        "Created instance: {} (IP: {})",
                        instance_id, instance.ip_addr
                    );
                    self.instances.insert(instance_id, instance);
                }

                // Remove excess instances if we have too many
                if self.instances.len() > self.config.replicas as usize {
                    let instances_to_remove: Vec<_> = self
                        .instances
                        .keys()
                        .skip(self.config.replicas as usize)
                        .cloned()
                        .collect();

                    for instance_id in instances_to_remove {
                        debug!("Removing excess instance: {}", instance_id);
                        self.cleanup_instance(&instance_id, image_pool.clone())
                            .await?;
                    }
                }

                // Start creating machines for all instances
                self.start_all_instances(image_pool.clone()).await?;
                self.status = DeploymentStatus::WaitingForInstances;
            }

            DeploymentStatus::WaitingForInstances => {
                self.check_instance_status().await?;
            }

            DeploymentStatus::Ready => {
                // Check if all instances are still healthy
                self.check_instance_health().await?;
            }

            DeploymentStatus::Replacing => {
                debug!("Replacing all instances due to configuration change");

                // Clean up all current instances (stops machines and deletes volumes)
                self.cleanup_all_instances(image_pool.clone()).await?;

                // Clear any remaining tasks
                self.tasks.instance_tasks.clear();

                // Move back to creating instances
                self.status = DeploymentStatus::CreatingInstances;
            }

            DeploymentStatus::Suspended => {
                debug!("Deployment is suspended");
                // TODO: Handle resume logic if needed
            }

            DeploymentStatus::Stopping => {
                debug!("Stopping deployment");

                // Cancel all tasks and clean up all instances
                self.tasks.cancel_all();
                self.cleanup_all_instances(image_pool.clone()).await?;

                self.status = DeploymentStatus::Stopped;
                debug!("Deployment stopped");
            }

            DeploymentStatus::Stopped => {
                // Nothing to progress - deployment is stopped
                debug!("Deployment is stopped");
            }
        }

        Ok(())
    }

    async fn start_all_instances(&mut self, image_pool: Arc<ImagePool>) -> Result<()> {
        // Get the pulled image to use as rootfs template
        let reference: Reference = self.config.image.parse()?;

        let Some(base_image) = image_pool.get_by_reference(&reference).await? else {
            error!("Image not found after pull");
            self.status = DeploymentStatus::Stopped;
            bail!("Image not found after pull");
        };

        // Get the base volume to use as template
        let Some(base_volume) = image_pool.get_volume(&base_image.volume_id).await? else {
            error!("Volume not found for image");
            self.status = DeploymentStatus::Stopped;
            bail!("Volume not found for image");
        };

        debug!("Using base rootfs path as template: {}", base_volume.path);

        let instance_ids_to_process: Vec<String> = self
            .instances
            .iter()
            .filter_map(|(id, instance)| {
                if instance.status == InstanceStatus::New {
                    Some(id.clone())
                } else {
                    None
                }
            })
            .collect();

        for instance_id in instance_ids_to_process {
            // Update the instance status to Creating
            if let Some(instance) = self.instances.get_mut(&instance_id) {
                instance.status = InstanceStatus::Creating;
            }

            // Create a unique volume for this instance by copying the base volume
            debug!("Creating unique volume for instance {}", instance_id);
            let instance_volume = base_volume.create_copy_for_instance(&instance_id).await?;

            // Update the instance to track its volume and get its data
            let (tap_name, ip_addr, log_file_path) = {
                if let Some(instance) = self.instances.get_mut(&instance_id) {
                    instance.image_id =
                        Some(format!("{}@{}", base_image.reference, base_image.digest));
                    instance.rootfs_volume_id = Some(instance_volume.id.clone());

                    debug!(
                        "Instance {} tracking: image={}, volume={}",
                        instance_id,
                        instance.image_id.as_ref().unwrap(),
                        instance.rootfs_volume_id.as_ref().unwrap()
                    );

                    (
                        instance.tap_name.clone(),
                        instance.ip_addr.clone(),
                        instance.log_file_path.clone(),
                    )
                } else {
                    continue; // Instance was removed, skip
                }
            };

            let spark_snapshot_policy = match &self.config.mode {
                DeploymentMode::Spark {
                    snapshot_policy, ..
                } => Some(snapshot_policy.clone()),
                _ => None,
            };

            let machine_config = MachineConfig {
                memory_size_mib: self.config.memory_mib,
                vcpu_count: self.config.vcpu_count,
                rootfs_path: instance_volume.path, // Each instance gets its own volume!
                tap_name,
                ip_addr,
                gateway: self.gateway.clone(),
                netmask: self.netmask.clone(),
                envs: self.config.envs.clone(),
                log_file_path,
                spark_snapshot_policy,
            };

            debug!(
                "Starting instance {} with config: {}MB RAM, {} vCPUs (unique rootfs: {})",
                instance_id,
                machine_config.memory_size_mib,
                machine_config.vcpu_count,
                machine_config.rootfs_path
            );

            let machine = Machine::new(machine_config)?;
            machine.start().await?;

            // Update instance status to Starting
            if let Some(instance) = self.instances.get_mut(&instance_id) {
                instance.status = InstanceStatus::Starting;
            }
            self.machines.insert(instance_id.clone(), machine.clone());

            debug!("Instance {} started, waiting for ready status", instance_id);
            let instance_id_clone = instance_id.clone();
            let wait_task = task::spawn(async move {
                let mut rx = machine.status_rx().await;

                while let Ok(status) = rx.recv().await {
                    match status {
                        MachineStatus::Ready => {
                            debug!("Instance {} is ready!", instance_id_clone);
                            break;
                        }
                        MachineStatus::Error(e) => {
                            error!("Instance {} error: {}", instance_id_clone, e);
                            bail!("Instance error: {}", e);
                        }
                        _ => {
                            debug!("Instance {} status: {:?}", instance_id_clone, status);
                        }
                    }
                }
                Ok(())
            });

            self.tasks
                .instance_tasks
                .insert(instance_id.clone(), wait_task);
        }

        Ok(())
    }

    async fn check_instance_status(&mut self) -> Result<()> {
        let mut ready_count = 0;
        let mut finished_tasks = Vec::new();

        // Check each instance task
        for (instance_id, task) in &self.tasks.instance_tasks {
            if task.is_finished() {
                finished_tasks.push(instance_id.clone());
            }
        }

        // Process finished tasks
        for instance_id in finished_tasks {
            if let Some(task) = self.tasks.instance_tasks.remove(&instance_id) {
                let result = task.await;
                match result {
                    Ok(Ok(())) => {
                        if let Some(instance) = self.instances.get_mut(&instance_id) {
                            instance.status = InstanceStatus::Ready;
                            debug!("Instance {} is ready!", instance_id);
                        }
                    }
                    Ok(Err(e)) => {
                        error!("Instance {} failed: {}", instance_id, e);
                        if let Some(instance) = self.instances.get_mut(&instance_id) {
                            instance.status = InstanceStatus::Error(e.to_string());
                        }
                    }
                    Err(e) => {
                        error!("Instance {} task failed: {}", instance_id, e);
                        if let Some(instance) = self.instances.get_mut(&instance_id) {
                            instance.status = InstanceStatus::Error(e.to_string());
                        }
                    }
                }
            }
        }

        // Count ready instances
        for instance in self.instances.values() {
            if instance.status == InstanceStatus::Ready {
                ready_count += 1;
            }
        }

        debug!(
            "Instance status: {}/{} ready",
            ready_count, self.config.replicas
        );

        // Check if all instances are ready
        if ready_count == self.config.replicas {
            info!("All instances are ready! Deployment is now READY!");
            self.status = DeploymentStatus::Ready;
        } else if self.tasks.instance_tasks.is_empty() {
            // All tasks finished but not all are ready - check for errors
            let error_count = self
                .instances
                .values()
                .filter(|i| matches!(i.status, InstanceStatus::Error(_)))
                .count();

            if error_count > 0 {
                error!(
                    "{} instances failed, marking deployment as stopped",
                    error_count
                );
                self.status = DeploymentStatus::Stopped;
            }
        }

        Ok(())
    }

    async fn check_instance_health(&self) -> Result<()> {
        // Check if any instances have failed and need replacement
        // This could be extended to include health checks
        for (instance_id, instance) in &self.instances {
            if matches!(instance.status, InstanceStatus::Error(_)) {
                warn!("Instance {} has failed, may need replacement", instance_id);
            }
        }
        Ok(())
    }

    pub fn cancel(&mut self) {
        self.status = DeploymentStatus::Stopping;
        self.tasks.cancel_all();
    }

    pub fn is_finished(&self) -> bool {
        matches!(
            self.status,
            DeploymentStatus::Ready | DeploymentStatus::Stopped
        )
    }

    pub fn get_ready_instance_count(&self) -> usize {
        self.instances
            .values()
            .filter(|i| i.status == InstanceStatus::Ready)
            .count()
    }

    pub fn get_total_instance_count(&self) -> usize {
        self.instances.len()
    }

    async fn cleanup_instance(
        &mut self,
        instance_id: &str,
        image_pool: Arc<ImagePool>,
    ) -> Result<()> {
        debug!("Cleaning up instance: {}", instance_id);

        // Stop the machine if it exists
        if let Some(machine) = self.machines.remove(instance_id) {
            debug!("Stopping machine for instance: {}", instance_id);
            let _ = machine
                .stop(crate::machine::MachineStopReason::Shutdown)
                .await;
        }

        // Remove and clean up the instance volume
        if let Some(instance) = self.instances.remove(instance_id) {
            if let Some(image_id) = &instance.image_id {
                debug!("Instance {} was using image: {}", instance_id, image_id);
            }

            if let Some(volume_id) = &instance.rootfs_volume_id {
                debug!(
                    "Instance {} was using rootfs volume: {}",
                    instance_id, volume_id
                );

                // Get volume by id and delete it
                if let Some(volume) = image_pool.get_volume(volume_id).await? {
                    volume.delete().await?;
                    debug!("Volume deleted successfully: {}", volume_id);
                } else {
                    warn!("Volume not found: {}", volume_id);
                }
            }
        }

        // Remove any pending tasks
        if let Some(task) = self.tasks.instance_tasks.remove(instance_id) {
            task.abort();
        }

        Ok(())
    }

    async fn cleanup_partial_instances(&mut self, image_pool: Arc<ImagePool>) -> Result<()> {
        debug!("Cleaning up partial instances during cancellation");

        // Get instances that are not ready (partial instances)
        let partial_instance_ids: Vec<_> = self
            .instances
            .iter()
            .filter_map(|(id, instance)| {
                if !matches!(instance.status, InstanceStatus::Ready) {
                    Some(id.clone())
                } else {
                    None
                }
            })
            .collect();

        debug!(
            "Found {} partial instances to clean up",
            partial_instance_ids.len()
        );

        for instance_id in partial_instance_ids {
            self.cleanup_instance(&instance_id, image_pool.clone())
                .await?;
        }

        Ok(())
    }

    pub async fn cleanup_all_instances(&mut self, image_pool: Arc<ImagePool>) -> Result<()> {
        debug!("Cleaning up all instances for deployment");
        let instance_ids: Vec<_> = self.instances.keys().cloned().collect();

        for instance_id in instance_ids {
            self.cleanup_instance(&instance_id, image_pool.clone())
                .await?;
        }

        Ok(())
    }
}
