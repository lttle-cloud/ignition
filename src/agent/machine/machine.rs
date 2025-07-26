use std::{
    collections::HashMap,
    sync::{Arc, Barrier},
    time::{Duration, Instant},
};

use anyhow::{Result, bail};
use event_manager::{EventManager, MutEventSubscriber};
use futures_util::future::join_all;
use kvm_ioctls::VmFd;
use takeoff_proto::proto::{MountPoint, TakeoffInitArgs};
use tokio::{
    sync::{Mutex, RwLock},
    task::JoinHandle,
};
use tracing::warn;
use vm_allocator::AddressAllocator;
use vm_device::device_manager::IoManager;
use vm_memory::{GuestAddress, GuestMemory, GuestMemoryMmap};

use crate::agent::{
    image::Image,
    machine::{
        MachineAgentConfig,
        vm::{
            constants::SERIAL_IRQ,
            devices::{
                DeviceEvent, VmDevices, alloc::IrqAllocator, setup_devices,
                virtio::block::get_block_mount_source_by_index,
            },
            kernel::{create_cmdline, load_kernel},
            kvm::create_and_verify_kvm,
            memory::{create_memory, create_mmio_allocator},
            vcpu::{RunningVcpuHandle, Vcpu, VcpuEvent, VcpuEventType, VcpuRunResult},
        },
    },
    volume::Volume,
};

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
    Flash(SnapshotStrategy),
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
    pub mode: MachineMode,
    pub state_retention_mode: MachineStateRetentionMode,
    pub resources: MachineResources,
    pub image: Image,
    pub envs: HashMap<String, String>,
    pub volume_mounts: Vec<VolumeMountConfig>,
    pub network: NetworkConfig,
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
}

pub enum MachineStopReason {
    Stop,
    Suspend,
}

pub struct Machine {
    pub config: MachineConfig,
    state: Arc<RwLock<MachineState>>,
    state_change_rx: async_broadcast::Receiver<MachineState>,
    state_change_tx: async_broadcast::Sender<MachineState>,
    first_boot_duration: Arc<RwLock<Option<Duration>>>,
    last_start_time: Arc<RwLock<Option<Instant>>>,
    last_ready_time: Arc<RwLock<Option<Instant>>>,

    guest_memory: GuestMemoryMmap,
    mmio_allocator: AddressAllocator,
    kernel_start_address: GuestAddress,

    vm_fd: Arc<VmFd>,
    vcpu_event_rx: async_broadcast::Receiver<VcpuEvent>,
    vcpu_event_tx: async_broadcast::Sender<VcpuEvent>,
    vcpu_watcher_task: Mutex<Option<JoinHandle<()>>>,

    vcpu_start_barrier: Arc<Barrier>,
    idle_vcpus: Mutex<Vec<Vcpu>>,
    running_vcpus: Mutex<Vec<RunningVcpuHandle>>,

    device_event_rx: async_broadcast::Receiver<DeviceEvent>,
    device_event_tx: async_broadcast::Sender<DeviceEvent>,
    device_watcher_task: Mutex<Option<JoinHandle<()>>>,
    devices: VmDevices,
    event_manager_task: std::thread::JoinHandle<()>,
}

pub type MachineRef = Arc<Machine>;

impl Machine {
    pub async fn new(
        agent_config: &MachineAgentConfig,
        config: MachineConfig,
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
        };
        let takeoff_args_str = takeoff_args.encode()?;
        kernel_cmd.insert_str(format!("takeoff={}", takeoff_args_str))?;

        let mut io_manager = IoManager::new();
        let mut irq_allocator = IrqAllocator::new(SERIAL_IRQ)?;

        let mut event_manager =
            EventManager::<Arc<std::sync::Mutex<dyn MutEventSubscriber + Send>>>::new()?;

        let vm_fd = Arc::new(vm_fd);

        let (device_event_tx, device_event_rx) = async_broadcast::broadcast::<DeviceEvent>(128);

        // setup devices
        let devices = setup_devices(
            &config,
            &kvm,
            vm_fd.clone(),
            &guest_memory,
            &mut irq_allocator,
            &mut mmio_allocator,
            &mut io_manager,
            &mut event_manager,
            &mut kernel_cmd,
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
            &config,
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
        let (vcpu_event_tx, vcpu_event_rx) = async_broadcast::broadcast::<VcpuEvent>(128);

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

        let (state_change_tx, state_change_rx) = async_broadcast::broadcast::<MachineState>(128);

        let machine = Arc::new(Self {
            config,
            state: Arc::new(RwLock::new(MachineState::Idle)),
            state_change_rx,
            state_change_tx,

            last_start_time: Arc::new(RwLock::new(None)),
            last_ready_time: Arc::new(RwLock::new(None)),
            first_boot_duration: Arc::new(RwLock::new(None)),

            guest_memory,
            mmio_allocator,
            kernel_start_address,

            vm_fd,
            vcpu_event_rx,
            vcpu_event_tx,
            vcpu_watcher_task: Mutex::new(None),

            vcpu_start_barrier: barrier,
            idle_vcpus: Mutex::new(vcpus),
            running_vcpus: Mutex::new(vec![]),

            devices,
            event_manager_task,
            device_event_rx,
            device_event_tx,
            device_watcher_task: Mutex::new(None),
        });

        let watcher_machine = machine.clone();
        let vcpu_watcher_task = tokio::spawn(async move {
            let mut rx = watcher_machine.vcpu_event_rx.clone();
            while let Ok(event) = rx.recv().await {
                match event.event_type {
                    // stop the machine in case any vcpu errors
                    VcpuEventType::Errored => {
                        watcher_machine.stop().await.ok();
                    }
                    VcpuEventType::Stopped => {
                        watcher_machine.stop().await.ok();
                    }
                    VcpuEventType::Suspended => {
                        watcher_machine.suspend().await.ok();
                    }
                    VcpuEventType::Restarted => {
                        // net handler event won't be sent again. instead we'll use the vcpu event to set ready state
                        watcher_machine.set_state(MachineState::Ready).await.ok();
                    }
                }
            }
        });
        *machine.vcpu_watcher_task.lock().await = Some(vcpu_watcher_task);

        let watcher_machine = machine.clone();
        let device_watcher_task = tokio::spawn(async move {
            let mut rx = watcher_machine.device_event_rx.clone();
            while let Ok(event) = rx.recv().await {
                match event {
                    DeviceEvent::UserSpaceReady => {
                        watcher_machine.set_state(MachineState::Ready).await.ok();
                    }
                    DeviceEvent::StopRequested => {
                        if matches!(watcher_machine.config.mode, MachineMode::Flash(_)) {
                            watcher_machine.suspend().await.ok();
                        } else {
                            watcher_machine.stop().await.ok();
                        }
                    }
                }
            }
        });
        *machine.device_watcher_task.lock().await = Some(device_watcher_task);

        Ok(machine)
    }

    async fn set_state(&self, state: MachineState) -> Result<()> {
        let mut current_state = self.state.write().await;
        if *current_state == state {
            return Ok(());
        }

        'time_update_block: {
            if state == MachineState::Booting {
                *self.last_start_time.write().await = Some(Instant::now());
            } else if state == MachineState::Ready {
                let ready_time = Instant::now();
                *self.last_ready_time.write().await = Some(ready_time);

                let last_start_time = { self.last_start_time.read().await.clone() };
                let first_boot_duration = { self.first_boot_duration.read().await.clone() };

                let Some(last_start_time) = last_start_time else {
                    break 'time_update_block;
                };

                self.devices
                    .guest_manager
                    .lock()
                    .expect("Failed to lock guest manager")
                    .set_boot_duration(ready_time.duration_since(last_start_time));

                if first_boot_duration.is_some() {
                    break 'time_update_block;
                }

                let mut first_boot_duration = self.first_boot_duration.write().await;
                *first_boot_duration = Some(last_start_time.duration_since(Instant::now()));
            }
        }

        *current_state = state.clone();
        self.state_change_tx.broadcast(state).await.ok();
        Ok(())
    }

    pub async fn get_state(&self) -> MachineState {
        let state = self.state.read().await.clone();
        state
    }

    pub async fn get_last_boot_duration(&self) -> Result<Duration> {
        let last_start_time = self.last_start_time.read().await;
        let last_ready_time = self.last_ready_time.read().await;
        let Some(last_start_time) = *last_start_time else {
            return Ok(Duration::from_secs(0));
        };
        let Some(last_ready_time) = *last_ready_time else {
            return Ok(Duration::from_secs(0));
        };

        Ok(last_ready_time.duration_since(last_start_time))
    }

    pub async fn start(&self) -> Result<()> {
        let current_state = self.get_state().await;
        let is_first_start = current_state == MachineState::Idle;

        if matches!(current_state, MachineState::Booting | MachineState::Ready) {
            return Ok(());
        }

        if !matches!(
            current_state,
            MachineState::Idle | MachineState::Stopped | MachineState::Suspended
        ) {
            bail!("Machine can't be started from state: {:?}", current_state);
        }

        if !is_first_start {
            let mut guest_manager = self
                .devices
                .guest_manager
                .lock()
                .expect("Failed to lock guest manager");

            guest_manager.set_snapshot_strategy(None);
        }

        let mut idle_vcpus = self.idle_vcpus.lock().await;
        let mut running_vcpus = self.running_vcpus.lock().await;
        running_vcpus.clear();

        self.set_state(MachineState::Booting).await?;

        for vcpu in idle_vcpus.drain(..) {
            let handle = vcpu.start().await?;
            running_vcpus.push(handle);
        }

        Ok(())
    }

    pub async fn stop(&self) -> Result<()> {
        self.stop_with_reason(MachineStopReason::Stop).await
    }

    pub async fn suspend(&self) -> Result<()> {
        self.stop_with_reason(MachineStopReason::Suspend).await
    }

    pub async fn stop_with_reason(&self, reason: MachineStopReason) -> Result<()> {
        let current_state = self.get_state().await;
        if matches!(
            current_state,
            MachineState::Stopped | MachineState::Suspended
        ) {
            return Ok(());
        }

        if !matches!(current_state, MachineState::Ready | MachineState::Booting) {
            bail!("Machine can't be stopped from state: {:?}", current_state);
        }

        let next_state = match reason {
            MachineStopReason::Stop => MachineState::Stopping,
            MachineStopReason::Suspend => MachineState::Suspending,
        };
        self.set_state(next_state).await?;

        let mut running_vcpus = self.running_vcpus.lock().await;
        let mut idle_vcpus = self.idle_vcpus.lock().await;

        let handles = running_vcpus
            .drain(..)
            .map(|handle| handle.signal_stop_and_join());

        let results = join_all(handles).await;

        let mut failed_vcpu_index = vec![];
        for result in results {
            match result {
                VcpuRunResult::Ok(vcpu) => idle_vcpus.push(vcpu),
                VcpuRunResult::Error(e, vcpu) => {
                    failed_vcpu_index.push((vcpu.index, e));
                }
            }
        }

        if !failed_vcpu_index.is_empty() {
            let mut fail_message = String::from("Failed to stop vcpus: ");
            for (index, e) in failed_vcpu_index {
                fail_message.push_str(&format!("Vcpu {} failed to stop: {}", index, e));
                fail_message.push_str("\n");
            }

            self.set_state(MachineState::Error(fail_message.clone()))
                .await?;

            bail!("{}", fail_message);
        }

        let next_state = match reason {
            MachineStopReason::Stop => MachineState::Stopped,
            MachineStopReason::Suspend => MachineState::Suspended,
        };
        self.set_state(next_state).await?;

        Ok(())
    }

    pub async fn watch_state(&self) -> Result<async_broadcast::Receiver<MachineState>> {
        Ok(self.state_change_rx.clone())
    }

    pub async fn wait_for_state(&self, state: MachineState) -> Result<()> {
        let current_state = self.get_state().await;
        if current_state == state {
            return Ok(());
        }

        let mut rx = self.state_change_rx.clone();
        while let Ok(new_state) = rx.recv().await {
            if new_state == state {
                return Ok(());
            }
        }

        Ok(())
    }
}
