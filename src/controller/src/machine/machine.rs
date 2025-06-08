use std::{net::IpAddr, str::FromStr, sync::Arc};

use util::{
    async_runtime::{
        sync::{RwLock, broadcast},
        task::{self, JoinHandle},
    },
    encoding::codec,
    result::{Result, bail},
    tracing::{info, warn},
};
use vmm::{
    config::{BlockConfig, Config, KernelConfig, MemoryConfig, NetConfig, VcpuConfig},
    memory::Memory,
    state::VmmState,
    vmm::{Vmm, VmmStateController, VmmStateControllerMessage},
};

#[derive(Clone)]
pub enum MachineStopReason {
    Suspend,
    Shutdown,
}

#[codec]
#[derive(Clone, Debug)]
pub enum SparkSnapshotPolicy {
    OnNthListenSyscall(u32),
    OnUserspaceReady,
    Manual,
}

#[derive(Clone)]
pub enum MachineState {
    New,
    Running {
        vmm_controller: VmmStateController,
        run_task: Arc<JoinHandle<()>>,
        msg_handler_task: Arc<JoinHandle<()>>,
    },
    Ready {
        vmm_controller: VmmStateController,
        run_task: Arc<JoinHandle<()>>,
        msg_handler_task: Arc<JoinHandle<()>>,
    },
    Stopping {
        stop_reason: MachineStopReason,
    },
    Suspended {
        vmm_state: VmmState,
    },
    Stopped,
    Error(String),
}

#[derive(Clone, Debug)]
pub enum MachineStatus {
    New,
    Running,
    Ready,
    Stopping,
    Suspended,
    Stopped,
    Error(String),
}

impl MachineState {
    pub fn to_status(&self) -> MachineStatus {
        match self {
            MachineState::New => MachineStatus::New,
            MachineState::Running { .. } => MachineStatus::Running,
            MachineState::Ready { .. } => MachineStatus::Ready,
            MachineState::Stopping { .. } => MachineStatus::Stopping,
            MachineState::Suspended { .. } => MachineStatus::Suspended,
            MachineState::Stopped => MachineStatus::Stopped,
            MachineState::Error(e) => MachineStatus::Error(e.clone()),
        }
    }
}

impl std::fmt::Debug for MachineState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MachineState::New => write!(f, "New"),
            MachineState::Running { .. } => write!(f, "Running"),
            MachineState::Ready { .. } => write!(f, "Ready"),
            MachineState::Stopping { .. } => write!(f, "Stopping"),
            MachineState::Suspended { .. } => write!(f, "Suspended"),
            MachineState::Stopped => write!(f, "Stopped"),
            MachineState::Error(e) => write!(f, "Error: {}", e),
        }
    }
}

#[derive(Clone)]
pub struct MachineConfig {
    pub memory_size_mib: usize,
    pub vcpu_count: u8,
    pub rootfs_path: String,
    pub tap_name: String,
    pub ip_addr: String,
    pub gateway: String,
    pub netmask: String,
    pub envs: Vec<String>,
    pub log_file_path: String,
    pub spark_snapshot_policy: Option<SparkSnapshotPolicy>,
}

impl TryFrom<&MachineConfig> for Config {
    type Error = util::result::Error;

    fn try_from(config: &MachineConfig) -> Result<Self> {
        let IpAddr::V4(ip_addr) = IpAddr::from_str(&config.ip_addr)? else {
            bail!("Invalid IP address: {}", config.ip_addr);
        };

        let ip_addr_bytes = ip_addr.octets();
        let mac_addr = format!(
            "06:00:{}",
            ip_addr_bytes
                .iter()
                .map(|b| format!("{:02X}", b))
                .collect::<Vec<String>>()
                .join(":")
        );

        let config = Config::new()
            .memory(MemoryConfig::new(config.memory_size_mib))
            .vcpu(VcpuConfig::new(config.vcpu_count))
            .kernel(
                // TODO: take these values from some sort of shared config, not hardcoded here
                KernelConfig::new("../linux/vmlinux")?
                    .with_initrd("./target/takeoff.cpio")
                    .with_cmdline(
                        "i8042.nokbd reboot=t panic=1 noapic clocksource=kvm-clock tsc=reliable",
                    )?
                    .with_init_envs(config.envs.clone())?,
            )
            .with_net(
                NetConfig::new(
                    config.tap_name.clone(),
                    config.ip_addr.clone(),
                    config.netmask.clone(),
                    config.gateway.clone(),
                    mac_addr,
                )
                .with_listen_trigger_count(300),
            )
            .with_block(BlockConfig::new(config.rootfs_path.clone()).writeable())
            .with_log_file_path(config.log_file_path.clone())
            .into();

        Ok(config)
    }
}

#[derive(Clone)]
pub struct Machine {
    config: MachineConfig,
    evt: Arc<broadcast::Sender<MachineStatus>>,
    memory: Arc<Memory>,
    state: Arc<RwLock<MachineState>>,
}

impl Machine {
    pub fn new(config: MachineConfig) -> Result<Self> {
        let vmm_config = Config::try_from(&config)?;
        let memory = Memory::new(vmm_config.memory.clone())?;

        let (tx, _) = broadcast::channel(100);

        Ok(Self {
            config,
            evt: tx.into(),
            memory: memory.into(),
            state: RwLock::new(MachineState::New).into(),
        })
    }

    pub async fn status(&self) -> Result<MachineStatus> {
        let state = self.state.read().await;
        Ok(state.to_status())
    }

    pub async fn status_rx(&self) -> broadcast::Receiver<MachineStatus> {
        self.evt.subscribe()
    }

    pub async fn start(&self) -> Result<()> {
        let state_guard = self.state.read().await;
        let state = state_guard.clone();
        drop(state_guard);

        match state {
            MachineState::New => self.start_from_scratch().await,
            MachineState::Suspended { vmm_state } => {
                self.start_from_suspended(vmm_state, self.memory.clone())
                    .await
            }
            _ => {
                warn!(
                    "Attempted to start machine in state {:?}",
                    self.state.read().await
                );
                Ok(())
            }
        }
    }

    pub async fn stop(&self, reason: MachineStopReason) -> Result<()> {
        let state_guard = self.state.read().await;
        let state = state_guard.clone();
        drop(state_guard);

        match state {
            MachineState::Running { vmm_controller, .. }
            | MachineState::Ready { vmm_controller, .. } => {
                self.state
                    .update_state(
                        self.evt.clone(),
                        MachineState::Stopping {
                            stop_reason: reason,
                        },
                    )
                    .await;

                vmm_controller.request_stop();
                Ok(())
            }
            _ => {
                warn!(
                    "Attempted to stop machine in state {:?}",
                    self.state.read().await
                );
                Ok(())
            }
        }
    }

    pub async fn start_if_needed(&self) -> Result<()> {
        let state_guard = self.state.read().await;
        let state = state_guard.clone();
        drop(state_guard);

        match state {
            MachineState::Running { .. } | MachineState::Ready { .. } => Ok(()),
            _ => self.start().await,
        }
    }

    pub async fn wait_for_ready(&self) -> Result<()> {
        let state_guard = self.state.read().await;
        let state = state_guard.clone();
        drop(state_guard);

        if let MachineState::Ready { .. } = state {
            return Ok(());
        }

        let mut rx = self.status_rx().await;

        loop {
            let status = rx.recv().await;
            if let Ok(MachineStatus::Ready) = status {
                return Ok(());
            }
        }
    }

    async fn start_from_scratch(&self) -> Result<()> {
        let vmm_config = Config::try_from(&self.config)?;
        let vmm = Vmm::new(vmm_config, self.memory.clone())?;
        self.start_vmm(vmm).await
    }

    async fn start_from_suspended(&self, vmm_state: VmmState, memory: Arc<Memory>) -> Result<()> {
        info!("Starting machine from suspended state");
        let vmm = Vmm::from_state(vmm_state, memory)?;
        self.start_vmm(vmm).await
    }

    async fn start_vmm(&self, mut vmm: Vmm) -> Result<()> {
        let msg_handler_controller = vmm.controller();
        let msg_handler_state_guard = self.state.clone();

        let evt = self.evt.clone();
        let msg_handler_task = task::spawn(async move {
            let mut rx = msg_handler_controller.rx();
            while let Ok(message) = rx.recv().await {
                match message {
                    VmmStateControllerMessage::Error(e) => {
                        msg_handler_state_guard
                            .update_state_to_error(evt.clone(), e)
                            .await;
                    }
                    VmmStateControllerMessage::Stopped(new_state) => {
                        let stop_reason = {
                            let state = msg_handler_state_guard.read().await;
                            if let MachineState::Stopping { stop_reason } = &*state {
                                Some(stop_reason.clone())
                            } else {
                                None
                            }
                        };

                        match stop_reason {
                            Some(MachineStopReason::Shutdown) => {
                                msg_handler_state_guard
                                    .update_state(evt.clone(), MachineState::Stopped)
                                    .await;
                            }
                            Some(MachineStopReason::Suspend) | None => {
                                msg_handler_state_guard
                                    .update_state(
                                        evt.clone(),
                                        MachineState::Suspended {
                                            vmm_state: new_state,
                                        },
                                    )
                                    .await;
                            }
                        }
                    }
                    VmmStateControllerMessage::NetworkReady => {
                        let Some((controller, run_task, msg_handler_task)) = ({
                            let state = msg_handler_state_guard.read().await;
                            if let MachineState::Running {
                                run_task,
                                msg_handler_task,
                                vmm_controller,
                            } = &*state
                            {
                                Some((
                                    vmm_controller.clone(),
                                    run_task.clone(),
                                    msg_handler_task.clone(),
                                ))
                            } else {
                                None
                            }
                        }) else {
                            msg_handler_state_guard
                                .update_state_to_error(
                                evt.clone(),
                                    format!(
                                    "Received NetworkReady message while in invalid state",
                                ))
                                .await;

                            return;
                        };

                        msg_handler_state_guard
                            .update_state(
                                evt.clone(),
                                MachineState::Ready {
                                    vmm_controller: controller,
                                    run_task,
                                    msg_handler_task,
                                },
                            )
                            .await;
                    }
                    _ => {}
                }
            }
        });

        let state_controller = vmm.controller();

        let run_controller = vmm.controller();
        let run_task = task::spawn_blocking(move || match vmm.run() {
            Ok(_) => {}
            Err(e) => {
                run_controller.send(VmmStateControllerMessage::Error(e.to_string()));
            }
        });

        self.state
            .update_state(
                self.evt.clone(),
                MachineState::Running {
                    vmm_controller: state_controller,
                    run_task: Arc::new(run_task),
                    msg_handler_task: Arc::new(msg_handler_task),
                },
            )
            .await;

        Ok(())
    }
}

trait UpdateState {
    async fn update_state(
        &self,
        notifier: Arc<broadcast::Sender<MachineStatus>>,
        new_state: MachineState,
    );
    async fn update_state_to_error(
        &self,
        notifier: Arc<broadcast::Sender<MachineStatus>>,
        error: String,
    );
}

impl UpdateState for Arc<RwLock<MachineState>> {
    async fn update_state(
        &self,
        notifier: Arc<broadcast::Sender<MachineStatus>>,
        new_state: MachineState,
    ) {
        let status = new_state.to_status();

        let mut state = self.write().await;
        *state = new_state;

        info!("Machine state updated to {:?}", status);

        let _ = notifier.send(status);
    }

    async fn update_state_to_error(
        &self,
        notifier: Arc<broadcast::Sender<MachineStatus>>,
        error: String,
    ) {
        self.update_state(notifier, MachineState::Error(error))
            .await;
    }
}
