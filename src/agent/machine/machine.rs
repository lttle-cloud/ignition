use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Barrier, Weak},
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow, bail};
use event_manager::{EventManager, MutEventSubscriber};
use kvm_ioctls::VmFd;
use takeoff_proto::proto::{LogsTelemetryConfig, MountPoint, TakeoffInitArgs};
use tempfile::tempdir;
use tokio::{
    fs::create_dir_all,
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::{RwLock, broadcast, mpsc, oneshot},
    task::JoinHandle,
    time::sleep,
};
use tracing::{info, warn};
use vm_allocator::AddressAllocator;
use vm_device::device_manager::IoManager;
use vm_memory::{GuestAddress, GuestMemory, GuestMemoryMmap};

use crate::{
    agent::{
        image::Image,
        machine::{
            MachineAgentConfig,
            state_machine::{MachineStateMachine, StateCommand},
            vm::{
                constants::SERIAL_IRQ,
                devices::{
                    DeviceEvent, VmDevices, alloc::IrqAllocator, setup_devices,
                    virtio::block::get_block_mount_source_by_index,
                },
                kernel::{create_cmdline, load_kernel},
                kvm::create_and_verify_kvm,
                memory::{create_memory, create_mmio_allocator},
                vcpu::{Vcpu, VcpuEvent, VcpuEventType},
            },
        },
        volume::Volume,
    },
    controller::{context::ControllerKey, scheduler::Scheduler},
};

const CONNECTION_CHECK_INTERVAL: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MachineState {
    Idle,
    Booting,
    Ready,
    Suspending,
    Suspended,
    Stopping,
    Stopped,
    Error(String),
}

#[derive(Debug, Clone)]
pub enum MachineStateRetentionMode {
    InMemory,
    OnDisk { path: String },
}

#[derive(Debug, Clone)]
pub enum MachineMode {
    Regular,
    Flash {
        snapshot_strategy: SnapshotStrategy,
        suspend_timeout: Duration,
    },
}

#[derive(Debug, Clone)]
pub enum SnapshotStrategy {
    WaitForNthListen(u32),
    WaitForFirstListen,
    WaitForListenOnPort(u16),
    WaitForUserSpaceReady,
    Manual,
}

#[derive(Debug, Clone)]
pub struct MachineResources {
    pub cpu: u8,
    pub memory: u64,
}

#[derive(Debug, Clone)]
pub struct MachineConfig {
    pub name: String,
    pub network_tag: String,
    pub controller_key: ControllerKey,
    pub mode: MachineMode,
    pub state_retention_mode: MachineStateRetentionMode,
    pub resources: MachineResources,
    pub image: Image,
    pub envs: HashMap<String, String>,
    pub cmd: Option<Vec<String>>,
    pub volume_mounts: Vec<VolumeMountConfig>,
    pub network: NetworkConfig,
    pub logs_telemetry_config: LogsTelemetryConfig,
}

#[derive(Debug, Clone)]
pub struct VolumeMountConfig {
    pub volume: Volume,
    pub mount_at: String,
    pub read_only: bool,
    pub root: bool,
}

#[derive(Debug, Clone)]
pub struct NetworkConfig {
    pub tap_device: String,
    pub mac_address: String,
    pub ip_address: String,
    pub gateway: String,
    pub netmask: String,
    pub dns_servers: Vec<String>,
}

pub enum MachineStopReason {
    Stop,
    Suspend,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionState {
    Active,   // Strong reference - machine stays alive
    Inactive, // Weak reference - machine can suspend
}

#[derive(Debug, Clone)]
pub enum TrafficAwareMode {
    Enabled { inactivity_timeout: Duration },
    Disabled,
}

impl TrafficAwareMode {
    pub fn inactivity_timeout(&self) -> Option<Duration> {
        match self {
            TrafficAwareMode::Enabled { inactivity_timeout } => Some(*inactivity_timeout),
            TrafficAwareMode::Disabled => None,
        }
    }
}

pub struct TrafficAwareConnection {
    machine: MachineRef,
    pub upstream_socket: TcpStream,
    state: Arc<RwLock<ConnectionState>>,
    last_activity: Arc<RwLock<Instant>>,
    mode: TrafficAwareMode,
}

impl TrafficAwareConnection {
    pub async fn new(
        machine: MachineRef,
        target_port: u16,
        mode: TrafficAwareMode,
    ) -> Result<Self> {
        let machine_ip = machine.config.network.ip_address.clone();
        let address = format!("{machine_ip}:{}", target_port);

        // Send FlashLock immediately to keep machine awake during connection attempts
        machine.send_flash_lock().await?;

        // Try connecting with timeout and retry logic
        let upstream_socket =
            match Self::connect_with_retry(&address, 3, Duration::from_secs(5)).await {
                Ok(socket) => socket,
                Err(e) => {
                    // If connection fails, remove the flash lock we just added
                    let _ = machine.send_flash_unlock().await;
                    return Err(e);
                }
            };

        Ok(Self {
            machine,
            upstream_socket,
            state: Arc::new(RwLock::new(ConnectionState::Active)),
            last_activity: Arc::new(RwLock::new(Instant::now())),
            mode,
        })
    }

    async fn connect_with_retry(
        address: &str,
        max_retries: u32,
        timeout: Duration,
    ) -> Result<TcpStream> {
        let mut last_error = None;

        for attempt in 0..max_retries {
            match tokio::time::timeout(timeout, TcpStream::connect(address)).await {
                Ok(Ok(stream)) => {
                    if attempt > 0 {
                        info!(
                            "Successfully connected to {} on attempt {}",
                            address,
                            attempt + 1
                        );
                    }
                    return Ok(stream);
                }
                Ok(Err(e)) => {
                    warn!(
                        "Connection attempt {} to {} failed: {}",
                        attempt + 1,
                        address,
                        e
                    );
                    last_error = Some(e.into());
                }
                Err(_) => {
                    warn!(
                        "Connection attempt {} to {} timed out after {}s",
                        attempt + 1,
                        address,
                        timeout.as_secs()
                    );
                    last_error = Some(anyhow!("Connection timeout"));
                }
            }

            // Wait before retrying (except on last attempt)
            if attempt < max_retries - 1 {
                let delay = match attempt {
                    0 => Duration::from_millis(100),
                    1 => Duration::from_millis(500),
                    _ => Duration::from_secs(1),
                };
                info!(
                    "Retrying connection to {} in {}ms",
                    address,
                    delay.as_millis()
                );
                tokio::time::sleep(delay).await;
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow!("All connection attempts failed")))
    }

    pub fn ip_address(&self) -> String {
        self.machine.config.network.ip_address.clone()
    }

    pub fn machine(&self) -> &MachineRef {
        &self.machine
    }

    pub fn upstream_socket(&mut self) -> &mut TcpStream {
        &mut self.upstream_socket
    }

    async fn mark_active(&self) {
        let mut state = self.state.write().await;
        let mut last_activity = self.last_activity.write().await;

        if *state == ConnectionState::Inactive {
            *state = ConnectionState::Active;
            // Send FlashLock to indicate this connection is now active
            let _ = self.machine.send_flash_lock().await;
            info!(
                "Connection to machine '{}' became active",
                self.machine.config.name
            );
        }

        *last_activity = Instant::now();
    }

    async fn check_inactivity(&self) {
        let last_activity = self.last_activity.read().await;
        let elapsed = last_activity.elapsed();

        if let Some(timeout) = self.mode.inactivity_timeout() {
            if elapsed >= timeout {
                let mut state = self.state.write().await;
                if *state == ConnectionState::Active {
                    *state = ConnectionState::Inactive;
                    // Send FlashUnlock to indicate this connection is no longer active
                    let _ = self.machine.send_flash_unlock().await;
                    info!(
                        "Connection to machine '{}' became inactive (timeout: {}s)",
                        self.machine.config.name,
                        timeout.as_secs()
                    );
                }
            }
        }
    }

    pub async fn proxy_from_client<T>(&mut self, mut tls_stream: T) -> Result<()>
    where
        T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        let mut tls_buf = [0u8; 8192];
        let mut upstream_buf = [0u8; 8192];

        self.mark_active().await;

        loop {
            tokio::select! {
                // TLS -> Upstream traffic
                result = tls_stream.read(&mut tls_buf) => {
                    match result {
                        Ok(n) => {
                            if n > 0 {
                                self.mark_active().await;
                                if let Err(_) = self.upstream_socket.write_all(&tls_buf[..n]).await {
                                    // Upstream write failed, close both connections
                                    break;
                                }
                            } else {
                                // TLS stream closed, close both connections
                                break;
                            }
                        }
                        Err(_) => {
                            // TLS read error, close both connections
                            break;
                        }
                    }
                }

                // Upstream -> TLS traffic
                result = self.upstream_socket.read(&mut upstream_buf) => {
                    match result {
                        Ok(n) => {
                            if n > 0 {
                                self.mark_active().await;
                                if let Err(_) = tls_stream.write_all(&upstream_buf[..n]).await {
                                    // TLS write failed, close both connections
                                    break;
                                }
                            } else {
                                // Upstream closed, close both connections
                                break;
                            }
                        }
                        Err(_) => {
                            // Upstream read error, close both connections
                            break;
                        }
                    }
                }

                _ = sleep(CONNECTION_CHECK_INTERVAL) => {
                    if matches!(self.mode, TrafficAwareMode::Enabled { .. }) {
                        self.check_inactivity().await;
                    }
                }
            }
        }

        let _ = tls_stream.shutdown().await;
        let _ = self.upstream_socket.shutdown().await;

        Ok(())
    }
}

impl Drop for TrafficAwareConnection {
    fn drop(&mut self) {
        let machine = self.machine.clone();
        let state = self.state.clone();

        tokio::spawn(async move {
            let current_state = state.read().await;
            if *current_state == ConnectionState::Active {
                // Connection is being dropped while active, send FlashUnlock
                let _ = machine.send_flash_unlock().await;
            }
        });
    }
}

#[allow(unused)]
pub struct Machine {
    pub config: MachineConfig,

    // State machine communication
    command_tx: mpsc::Sender<StateCommand>,
    state_rx: broadcast::Receiver<MachineState>,

    // VM resources (immutable after creation)
    guest_memory: GuestMemoryMmap,
    mmio_allocator: AddressAllocator,
    kernel_start_address: GuestAddress,
    vm_fd: Arc<VmFd>,
    devices: VmDevices,
    event_manager_task: std::thread::JoinHandle<()>,

    // State machine task handle
    state_machine_task: JoinHandle<()>,
    // Current state (shared with state machine)
    current_state: Arc<tokio::sync::RwLock<MachineState>>,
    // Timing tracking (shared with state machine)
    first_boot_duration: Arc<tokio::sync::RwLock<Option<Duration>>>,
    last_start_time: Arc<tokio::sync::RwLock<Option<Instant>>>,
    last_ready_time: Arc<tokio::sync::RwLock<Option<Instant>>>,
    last_exit_code: Arc<tokio::sync::RwLock<Option<i32>>>,

    // Legacy fields for compatibility (will be removed later)
    vcpu_event_tx: async_broadcast::Sender<VcpuEvent>,
    device_event_tx: async_broadcast::Sender<DeviceEvent>,
    vcpu_start_barrier: Arc<Barrier>,
}

pub type MachineRef = Arc<Machine>;

impl Machine {
    pub async fn new(
        agent_config: &MachineAgentConfig,
        config: MachineConfig,
        scheduler: Weak<Scheduler>,
    ) -> Result<MachineRef> {
        let kvm = create_and_verify_kvm()?;
        let vm_fd = kvm.create_vm()?;

        // create memory
        let guest_memory = create_memory(&config).await?;
        let mut mmio_allocator = create_mmio_allocator()?;

        // init kernel cmdline
        let mut kernel_cmd = create_cmdline(&config)?;
        kernel_cmd.insert_str(&agent_config.kernel_cmd_init)?;

        let takeoff_args: TakeoffInitArgs = TakeoffInitArgs {
            envs: config.envs.clone(),
            cmd: config.cmd.clone(),
            mount_points: config
                .volume_mounts
                .iter()
                .enumerate()
                .map(|(index, mount)| MountPoint {
                    source: get_block_mount_source_by_index(index as u16),
                    target: mount.mount_at.clone(),
                    read_only: mount.read_only,
                })
                .collect(),
            logs_telemetry_config: config.logs_telemetry_config.clone(),
        };

        let mut io_manager = IoManager::new();
        let mut irq_allocator = IrqAllocator::new(SERIAL_IRQ)?;

        let mut event_manager =
            EventManager::<Arc<std::sync::Mutex<dyn MutEventSubscriber + Send>>>::new()?;

        let vm_fd = Arc::new(vm_fd);

        let (device_event_tx, _device_event_rx) = async_broadcast::broadcast::<DeviceEvent>(128);

        let log_dir = match &config.state_retention_mode {
            MachineStateRetentionMode::InMemory => tempdir()
                .map_err(|e| anyhow!("Failed to create temp dir for machine log: {}", e))?
                .path()
                .to_path_buf(),
            MachineStateRetentionMode::OnDisk { path } => PathBuf::from(path),
        };
        create_dir_all(&log_dir).await?;

        // setup devices
        let log_path = log_dir.join("serial.log");
        let devices = setup_devices(
            &config,
            &kvm,
            vm_fd.clone(),
            &takeoff_args,
            &guest_memory,
            &mut irq_allocator,
            &mut mmio_allocator,
            &mut io_manager,
            &mut event_manager,
            &mut kernel_cmd,
            log_path.to_string_lossy().as_ref(),
            device_event_tx.clone(),
        )
        .await?;

        let event_manager_task = std::thread::spawn(move || {
            loop {
                let event = event_manager.run();
                match event {
                    Ok(_) => {}
                    Err(e) => {
                        warn!("Error running event manager: {:?}", e);
                        break;
                    }
                }
            }
        });

        // load the kernel
        let kernel_load_result = load_kernel(
            &guest_memory,
            &agent_config.kernel_path,
            &agent_config.initrd_path,
            &kernel_cmd,
        )
        .await?;

        let Some(kernel_start_address) = guest_memory.check_address(kernel_load_result.kernel_load)
        else {
            bail!("Kernel load result is not in guest memory");
        };

        // add vcpus
        let io_manager = Arc::new(io_manager);
        let barrier = Arc::new(Barrier::new(config.resources.cpu as usize));
        let (vcpu_event_tx, _vcpu_event_rx) = async_broadcast::broadcast::<VcpuEvent>(128);

        let mut vcpus = vec![];
        for i in 0..config.resources.cpu {
            let vcpu = Vcpu::new(
                &kvm,
                &vm_fd,
                &guest_memory,
                io_manager.clone(),
                barrier.clone(),
                vcpu_event_tx.clone(),
                devices.guest_manager.clone(),
                kernel_start_address.clone(),
                config.resources.cpu as u8,
                i,
            )
            .await?;
            vcpus.push(vcpu);
        }

        // Create state machine communication channels
        let (command_tx, command_rx) = mpsc::channel(32);
        let (state_tx, state_rx) = broadcast::channel(32);

        // Create timing tracking
        let first_boot_duration = Arc::new(tokio::sync::RwLock::new(None));
        let last_start_time = Arc::new(tokio::sync::RwLock::new(None));
        let last_ready_time = Arc::new(tokio::sync::RwLock::new(None));
        let last_exit_code = Arc::new(tokio::sync::RwLock::new(None));

        // Create shared state for querying current state
        let current_state = Arc::new(tokio::sync::RwLock::new(MachineState::Idle));

        let machine = Arc::new(Self {
            config: config.clone(),
            command_tx: command_tx.clone(),
            state_rx,
            guest_memory,
            mmio_allocator,
            kernel_start_address,
            vm_fd,
            devices: devices.clone(),
            event_manager_task,
            state_machine_task: tokio::spawn(async {}), // Placeholder, will be updated
            current_state: current_state.clone(),
            first_boot_duration: first_boot_duration.clone(),
            last_start_time: last_start_time.clone(),
            last_ready_time: last_ready_time.clone(),
            last_exit_code: last_exit_code.clone(),
            vcpu_event_tx,
            device_event_tx,
            vcpu_start_barrier: barrier,
        });

        // Create state machine after machine struct is created
        let state_machine = MachineStateMachine::new(
            command_rx,
            command_tx,
            state_tx.clone(),
            current_state,
            config,
            vcpus,
            devices,
            scheduler,
            first_boot_duration,
            last_start_time,
            last_ready_time,
            last_exit_code,
        );

        let _state_machine_task = tokio::spawn(state_machine.run());

        // Update the machine's state_machine_task (this is a hack, ideally we'd restructure differently)
        // For now, we'll leave the placeholder and just rely on the task running

        // Start event watchers that send commands to state machine
        Self::start_event_watchers(&machine);

        Ok(machine)
    }

    fn start_event_watchers(machine: &MachineRef) {
        let command_tx = machine.command_tx.clone();

        // VCPU watcher - sends commands instead of direct state changes
        let vcpu_command_tx = command_tx.clone();
        let vcpu_event_rx = machine.vcpu_event_tx.new_receiver();
        let _vcpu_watcher = tokio::spawn(async move {
            let mut rx = vcpu_event_rx;
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        let command = match event.event_type {
                            VcpuEventType::Errored => StateCommand::SystemVcpuError {
                                message: format!("VCPU {} error", event.vcpu_index),
                            },
                            VcpuEventType::Stopped => StateCommand::SystemVcpuStopped,
                            VcpuEventType::Suspended => StateCommand::SystemVcpuSuspended,
                            VcpuEventType::Restarted => StateCommand::SystemVcpuRestarted,
                        };
                        let _ = vcpu_command_tx.send(command).await;
                    }
                    Err(async_broadcast::RecvError::Closed) => {
                        break;
                    }
                    Err(async_broadcast::RecvError::Overflowed(_)) => {
                        // Continue receiving - the channel is still usable after overflow
                        continue;
                    }
                }
            }
        });

        // Device watcher - sends commands instead of direct state changes
        let device_command_tx = command_tx.clone();
        let device_event_rx = machine.device_event_tx.new_receiver();
        let _device_watcher = tokio::spawn(async move {
            let mut rx = device_event_rx;
            while let Ok(event) = rx.recv().await {
                let command = match event {
                    DeviceEvent::UserSpaceReady => StateCommand::SystemDeviceReady,
                    DeviceEvent::StopRequested => StateCommand::SystemStopRequested,
                    DeviceEvent::FlashLock => StateCommand::SystemFlashLock,
                    DeviceEvent::FlashUnlock => StateCommand::SystemFlashUnlock,
                    DeviceEvent::ExitCode(code) => StateCommand::SystemExitCode { code },
                };
                let _ = device_command_tx.send(command).await;
            }
        });
    }

    // Helper method to send commands to state machine
    async fn send_command(&self, command: StateCommand) -> Result<()> {
        self.command_tx
            .send(command)
            .await
            .map_err(|_| anyhow!("State machine died"))
    }

    pub async fn get_connection(
        self: &Arc<Self>,
        target_port: u16,
        inactivity_timeout: Option<Duration>,
    ) -> Result<TrafficAwareConnection> {
        let current_state = self.get_state().await;

        let inactivity_mode = match inactivity_timeout {
            Some(timeout) => TrafficAwareMode::Enabled {
                inactivity_timeout: timeout,
            },
            None => TrafficAwareMode::Disabled,
        };

        if current_state == MachineState::Ready {
            return TrafficAwareConnection::new(self.clone(), target_port, inactivity_mode).await;
        }

        if current_state == MachineState::Booting {
            self.wait_for_state(MachineState::Ready).await?;
            return TrafficAwareConnection::new(self.clone(), target_port, inactivity_mode).await;
        }

        if !matches!(
            current_state,
            MachineState::Idle | MachineState::Stopped | MachineState::Suspended
        ) {
            bail!("Machine can't be started from state: {:?}", current_state);
        }

        // Startup synchronization is now handled by the state machine

        let state_after_lock = self.get_state().await;
        if state_after_lock == MachineState::Ready {
            return TrafficAwareConnection::new(self.clone(), target_port, inactivity_mode).await;
        }
        if state_after_lock == MachineState::Booting {
            self.wait_for_state(MachineState::Ready).await?;
            return TrafficAwareConnection::new(self.clone(), target_port, inactivity_mode).await;
        }

        self.start().await?;
        self.wait_for_state(MachineState::Ready).await?;

        TrafficAwareConnection::new(self.clone(), target_port, inactivity_mode).await
    }

    // Connection management is now handled by state machine
    // Flash lock/unlock methods for connection tracking
    async fn send_flash_lock(&self) -> Result<()> {
        self.send_command(StateCommand::SystemFlashLock).await
    }

    async fn send_flash_unlock(&self) -> Result<()> {
        self.send_command(StateCommand::SystemFlashUnlock).await
    }

    pub async fn get_state(&self) -> MachineState {
        // Get current state from shared state (always accurate)
        self.current_state.read().await.clone()
    }

    pub async fn get_last_boot_duration(&self) -> Option<Duration> {
        let last_start_time = self.last_start_time.read().await;
        let last_ready_time = self.last_ready_time.read().await;

        if let (Some(start), Some(ready)) = (*last_start_time, *last_ready_time) {
            Some(ready.duration_since(start))
        } else {
            None
        }
    }

    pub async fn get_first_boot_duration(&self) -> Option<Duration> {
        let first_boot_duration = self.first_boot_duration.read().await;
        first_boot_duration.clone()
    }

    pub async fn get_last_exit_code(&self) -> Option<i32> {
        self.last_exit_code.read().await.clone()
    }

    pub async fn start(&self) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.send_command(StateCommand::UserStart { reply: tx })
            .await?;
        rx.await.map_err(|_| anyhow!("State machine died"))?
    }

    pub async fn stop(&self) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.send_command(StateCommand::UserStop { reply: tx })
            .await?;
        rx.await.map_err(|_| anyhow!("State machine died"))?
    }

    pub async fn suspend(&self) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.send_command(StateCommand::UserSuspend { reply: tx })
            .await?;
        rx.await.map_err(|_| anyhow!("State machine died"))?
    }

    // Legacy method - now just delegates to appropriate new method
    pub async fn stop_with_reason(&self, reason: MachineStopReason) -> Result<()> {
        match reason {
            MachineStopReason::Stop => self.stop().await,
            MachineStopReason::Suspend => self.suspend().await,
        }
    }

    pub async fn watch_state(&self) -> Result<broadcast::Receiver<MachineState>> {
        Ok(self.state_rx.resubscribe())
    }

    pub async fn wait_for_state(&self, state: MachineState) -> Result<()> {
        let current_state = self.get_state().await;
        if current_state == state {
            return Ok(());
        }

        let mut rx = self.state_rx.resubscribe();
        while let Ok(new_state) = rx.recv().await {
            if new_state == state {
                return Ok(());
            }
        }

        Ok(())
    }
}
