use anyhow::{Result, anyhow};
use futures_util::future::join_all;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    sync::{Mutex, broadcast, mpsc, oneshot},
    task::JoinHandle,
    time::sleep,
};
use tracing::{info, warn};

use crate::{
    agent::machine::{
        MachineConfig,
        vm::{
            devices::VmDevices,
            vcpu::{RunningVcpuHandle, Vcpu, VcpuRunResult},
        },
    },
    controller::{
        context::{AsyncWork, ControllerEvent},
        scheduler::Scheduler,
    },
};

use super::machine::{MachineMode, MachineState};

#[derive(Debug)]
pub enum StateCommand {
    // User commands
    UserStart { reply: oneshot::Sender<Result<()>> },
    UserStop { reply: oneshot::Sender<Result<()>> },
    UserSuspend { reply: oneshot::Sender<Result<()>> },

    // System events
    SystemDeviceReady,
    SystemStopRequested,
    SystemVcpuError { message: String },
    SystemVcpuStopped,
    SystemVcpuSuspended,
    SystemVcpuRestarted,

    SystemSuspendTimeout,

    // Flash events
    SystemFlashLock,
    SystemFlashUnlock,
}

pub struct MachineStateMachine {
    // Core state
    current_state: MachineState,
    // Shared state reference for Machine to query
    shared_state: Arc<tokio::sync::RwLock<MachineState>>,

    // Communication
    command_rx: mpsc::Receiver<StateCommand>,
    command_tx: mpsc::Sender<StateCommand>,
    state_tx: broadcast::Sender<MachineState>,

    // Resources for executing transitions
    resources: MachineResources,
}

struct MachineResources {
    config: MachineConfig,
    vcpu_manager: Arc<Mutex<VcpuManager>>,
    devices: VmDevices,
    flash_lock_tracker: Arc<Mutex<FlashLockTracker>>,
    scheduler: std::sync::Weak<Scheduler>,
    first_boot_duration: Arc<tokio::sync::RwLock<Option<Duration>>>,
    last_start_time: Arc<tokio::sync::RwLock<Option<Instant>>>,
    last_ready_time: Arc<tokio::sync::RwLock<Option<Instant>>>,
}

pub struct VcpuManager {
    idle_vcpus: Vec<Vcpu>,
    running_vcpus: Vec<RunningVcpuHandle>,
}

impl VcpuManager {
    pub fn new(vcpus: Vec<Vcpu>) -> Self {
        Self {
            idle_vcpus: vcpus,
            running_vcpus: Vec::new(),
        }
    }

    pub async fn start_all(&mut self) -> Result<()> {
        self.running_vcpus.clear();
        for vcpu in self.idle_vcpus.drain(..) {
            let handle = vcpu.start().await?;
            self.running_vcpus.push(handle);
        }
        Ok(())
    }

    pub async fn stop_all(&mut self) -> Result<()> {
        let handles = self
            .running_vcpus
            .drain(..)
            .map(|handle| handle.signal_stop_and_join());

        let results = join_all(handles).await;

        let mut failed_vcpu_index = vec![];
        for result in results {
            match result {
                VcpuRunResult::Ok(vcpu) => self.idle_vcpus.push(vcpu),
                VcpuRunResult::Error(e, vcpu) => {
                    failed_vcpu_index.push((vcpu.index, e));
                    self.idle_vcpus.push(vcpu);
                }
            }
        }

        if !failed_vcpu_index.is_empty() {
            let mut fail_message = String::from("Failed to stop vcpus: ");
            for (index, e) in failed_vcpu_index {
                fail_message.push_str(&format!("Vcpu {} failed to stop: {}", index, e));
                fail_message.push_str("\n");
            }
            return Err(anyhow!("{}", fail_message));
        }

        Ok(())
    }
}

pub struct FlashLockTracker {
    active_count: u32,
    timeout_task: Option<JoinHandle<()>>,
}

impl FlashLockTracker {
    pub fn new() -> Self {
        Self {
            active_count: 0,
            timeout_task: None,
        }
    }

    pub fn add_flash_lock(&mut self) {
        self.active_count += 1;
        self.cancel_timeout();
    }

    pub fn remove_flash_lock(&mut self) -> bool {
        if self.active_count > 0 {
            self.active_count -= 1;
        }
        self.active_count == 0
    }

    pub fn has_active_locks(&self) -> bool {
        self.active_count > 0
    }

    pub fn cancel_timeout(&mut self) {
        if let Some(task) = self.timeout_task.take() {
            task.abort();
        }
    }

    pub fn start_timeout(&mut self, command_tx: mpsc::Sender<StateCommand>, timeout: Duration) {
        self.cancel_timeout();

        let task = tokio::spawn(async move {
            sleep(timeout).await;
            let _ = command_tx.send(StateCommand::SystemSuspendTimeout).await;
        });

        self.timeout_task = Some(task);
    }
}

impl MachineStateMachine {
    pub fn new(
        command_rx: mpsc::Receiver<StateCommand>,
        command_tx: mpsc::Sender<StateCommand>,
        state_tx: broadcast::Sender<MachineState>,
        shared_state: Arc<tokio::sync::RwLock<MachineState>>,
        config: MachineConfig,
        vcpus: Vec<Vcpu>,
        devices: VmDevices,
        scheduler: std::sync::Weak<Scheduler>,
        first_boot_duration: Arc<tokio::sync::RwLock<Option<Duration>>>,
        last_start_time: Arc<tokio::sync::RwLock<Option<Instant>>>,
        last_ready_time: Arc<tokio::sync::RwLock<Option<Instant>>>,
    ) -> Self {
        let resources = MachineResources {
            config,
            vcpu_manager: Arc::new(Mutex::new(VcpuManager::new(vcpus))),
            devices,
            flash_lock_tracker: Arc::new(Mutex::new(FlashLockTracker::new())),
            scheduler,
            first_boot_duration,
            last_start_time,
            last_ready_time,
        };

        Self {
            current_state: MachineState::Idle,
            shared_state,
            command_rx,
            command_tx,
            state_tx,
            resources,
        }
    }

    pub async fn run(mut self) {
        info!("Machine state machine started");
        while let Some(command) = self.command_rx.recv().await {
            if let Err(e) = self.handle_command(command).await {
                warn!("State machine error: {}", e);
                if let Err(err) = self.transition_to_error(e.to_string()).await {
                    warn!("Failed to transition to error state: {}", err);
                }
            }
        }
        info!("Machine state machine stopped");
    }

    async fn handle_command(&mut self, command: StateCommand) -> Result<()> {
        match command {
            StateCommand::UserStart { reply } => {
                let result = self.handle_user_start().await;
                let _ = reply.send(result);
            }

            StateCommand::UserStop { reply } => {
                let result = self.handle_user_stop().await;
                let _ = reply.send(result);
            }

            StateCommand::UserSuspend { reply } => {
                let result = self.handle_user_suspend().await;
                let _ = reply.send(result);
            }

            StateCommand::SystemDeviceReady => {
                self.handle_device_ready().await?;
            }

            StateCommand::SystemStopRequested => {
                self.handle_stop_requested().await?;
            }

            StateCommand::SystemVcpuError { message } => {
                self.handle_vcpu_error(message).await?;
            }

            StateCommand::SystemVcpuStopped => {
                self.handle_vcpu_stopped().await?;
            }

            StateCommand::SystemVcpuSuspended => {
                self.handle_vcpu_suspended().await?;
            }

            StateCommand::SystemVcpuRestarted => {
                self.handle_vcpu_restarted().await?;
            }

            StateCommand::SystemSuspendTimeout => {
                self.handle_suspend_timeout().await?;
            }

            StateCommand::SystemFlashLock => {
                self.handle_flash_lock().await?;
            }

            StateCommand::SystemFlashUnlock => {
                self.handle_flash_unlock().await?;
            }
        }
        Ok(())
    }

    // User transitions
    async fn handle_user_start(&mut self) -> Result<()> {
        let is_first_start = self.current_state == MachineState::Idle;

        match self.current_state {
            MachineState::Idle | MachineState::Stopped | MachineState::Suspended => {
                // Reset guest manager for non-first starts
                if !is_first_start {
                    self.resources
                        .devices
                        .guest_manager
                        .lock()
                        .expect("Failed to lock guest manager")
                        .set_snapshot_strategy(None);
                }

                // Set state to Booting and start VCPUs
                // For first start: SystemDeviceReady will transition to Ready
                // For resume from suspend: SystemVcpuRestarted will transition to Ready
                self.set_state(MachineState::Booting).await?;
                self.resources.vcpu_manager.lock().await.start_all().await?;
                Ok(())
            }
            MachineState::Booting | MachineState::Ready => {
                Ok(()) // Already started/starting
            }
            _ => Err(anyhow!("Can't start from {:?}", self.current_state)),
        }
    }

    async fn handle_user_stop(&mut self) -> Result<()> {
        match self.current_state {
            MachineState::Ready | MachineState::Booting => {
                self.set_state(MachineState::Stopping).await?;
                self.resources.vcpu_manager.lock().await.stop_all().await?;
                self.set_state(MachineState::Stopped).await?;
                Ok(())
            }
            MachineState::Suspended => {
                // Transition directly from Suspended to Stopped (VCPUs already stopped)
                self.set_state(MachineState::Stopped).await?;
                Ok(())
            }
            MachineState::Stopped => Ok(()),
            _ => Err(anyhow!("Can't stop from {:?}", self.current_state)),
        }
    }

    async fn handle_user_suspend(&mut self) -> Result<()> {
        match self.current_state {
            MachineState::Ready | MachineState::Booting => {
                self.set_state(MachineState::Suspending).await?;
                self.resources.vcpu_manager.lock().await.stop_all().await?;
                self.set_state(MachineState::Suspended).await?;
                Ok(())
            }
            MachineState::Suspended => Ok(()),
            _ => Err(anyhow!("Can't suspend from {:?}", self.current_state)),
        }
    }

    // System transitions
    async fn handle_device_ready(&mut self) -> Result<()> {
        if self.current_state == MachineState::Booting {
            self.set_state(MachineState::Ready).await?;
        }
        Ok(())
    }

    async fn handle_stop_requested(&mut self) -> Result<()> {
        match self.resources.config.mode {
            MachineMode::Flash { .. } => self.handle_user_suspend().await,
            MachineMode::Regular => self.handle_user_stop().await,
        }
    }

    async fn handle_vcpu_error(&mut self, message: String) -> Result<()> {
        self.transition_to_error(message).await
    }

    async fn handle_vcpu_stopped(&mut self) -> Result<()> {
        // Only trigger stop if we're not already in a suspend-related state
        match self.current_state {
            MachineState::Suspending | MachineState::Suspended => {
                // VCPUs stopping during suspend is expected, don't change state
                Ok(())
            }
            _ => {
                // Unexpected VCPU stop, trigger stop sequence
                let _ = self.handle_user_stop().await;
                Ok(())
            }
        }
    }

    async fn handle_vcpu_suspended(&mut self) -> Result<()> {
        // Only trigger suspend if we're not already in a suspend-related state
        match self.current_state {
            MachineState::Suspending | MachineState::Suspended => {
                // Already suspending/suspended, don't change state
                Ok(())
            }
            _ => {
                // Unexpected VCPU suspend, trigger suspend sequence
                let _ = self.handle_user_suspend().await;
                Ok(())
            }
        }
    }

    async fn handle_vcpu_restarted(&mut self) -> Result<()> {
        // VCPU restarted - transition to Ready
        if self.current_state == MachineState::Booting {
            self.set_state(MachineState::Ready).await?;
        }
        Ok(())
    }

    // Flash lock transitions
    async fn handle_flash_lock_added(&mut self) -> Result<()> {
        // Cancel any pending timeouts
        self.resources
            .flash_lock_tracker
            .lock()
            .await
            .cancel_timeout();

        // Wake up suspended machine
        if self.current_state == MachineState::Suspended {
            info!(
                "Machine '{}' is suspended but has active flash locks, waking it up",
                self.resources.config.name
            );
            self.handle_user_start().await?;
        }
        Ok(())
    }

    async fn handle_last_flash_lock_removed(&mut self) -> Result<()> {
        if let MachineMode::Flash {
            suspend_timeout, ..
        } = &self.resources.config.mode
        {
            info!(
                "Last flash lock removed for machine '{}', starting suspend timeout",
                self.resources.config.name
            );
            // Get command sender for timeout
            let command_tx = self.get_command_sender().await?;
            self.resources
                .flash_lock_tracker
                .lock()
                .await
                .start_timeout(command_tx, *suspend_timeout);
        }
        Ok(())
    }

    async fn handle_suspend_timeout(&mut self) -> Result<()> {
        // Only suspend if no active flash locks
        let should_suspend = {
            let tracker = self.resources.flash_lock_tracker.lock().await;
            !tracker.has_active_locks()
        };

        if should_suspend {
            info!(
                "Suspend timeout expired for machine '{}', suspending",
                self.resources.config.name
            );
            let _ = self.handle_user_suspend().await;
        } else {
            info!(
                "Suspend timeout expired for machine '{}' but has active flash locks, not suspending",
                self.resources.config.name
            );
        }
        Ok(())
    }

    async fn handle_flash_lock(&mut self) -> Result<()> {
        self.resources
            .flash_lock_tracker
            .lock()
            .await
            .add_flash_lock();
        self.handle_flash_lock_added().await
    }

    async fn handle_flash_unlock(&mut self) -> Result<()> {
        let is_last_flash_lock_removed = {
            let mut tracker = self.resources.flash_lock_tracker.lock().await;
            tracker.remove_flash_lock()
        };

        if is_last_flash_lock_removed {
            self.handle_last_flash_lock_removed().await?;
        }
        Ok(())
    }

    async fn set_state(&mut self, new_state: MachineState) -> Result<()> {
        if self.current_state == new_state {
            return Ok(());
        }

        // Handle timing updates
        self.update_timing_metrics(&new_state).await?;

        // Update state
        let old_state = self.current_state.clone();
        self.current_state = new_state.clone();

        // Update shared state for Machine to query
        *self.shared_state.write().await = new_state.clone();

        // Broadcast change
        let _ = self.state_tx.send(new_state.clone());

        // Notify scheduler
        self.notify_scheduler(&new_state).await?;

        info!("State transition: {:?} -> {:?}", old_state, new_state);
        Ok(())
    }

    async fn transition_to_error(&mut self, message: String) -> Result<()> {
        self.set_state(MachineState::Error(message)).await
    }

    async fn update_timing_metrics(&mut self, state: &MachineState) -> Result<()> {
        match state {
            MachineState::Booting => {
                *self.resources.last_start_time.write().await = Some(Instant::now());
            }
            MachineState::Ready => {
                let ready_time = Instant::now();
                *self.resources.last_ready_time.write().await = Some(ready_time);

                let last_start_time = { self.resources.last_start_time.read().await.clone() };
                let first_boot_duration =
                    { self.resources.first_boot_duration.read().await.clone() };

                if let Some(last_start_time) = last_start_time {
                    let boot_duration = ready_time.duration_since(last_start_time);

                    self.resources
                        .devices
                        .guest_manager
                        .lock()
                        .expect("Failed to lock guest manager")
                        .set_boot_duration(boot_duration);

                    if first_boot_duration.is_none() {
                        *self.resources.first_boot_duration.write().await = Some(boot_duration);
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn notify_scheduler(&self, state: &MachineState) -> Result<()> {
        if let Some(scheduler) = self.resources.scheduler.upgrade() {
            let (first_boot_duration, last_boot_duration) = {
                let first_boot_duration = self.resources.first_boot_duration.read().await.clone();
                let last_boot_duration = self.get_last_boot_duration().await;
                (first_boot_duration, last_boot_duration)
            };

            scheduler
                .push(
                    self.resources.config.controller_key.tenant.clone(),
                    ControllerEvent::AsyncWorkChange(
                        self.resources.config.controller_key.clone(),
                        AsyncWork::MachineStateChange {
                            machine_id: self.resources.config.name.clone(),
                            state: state.clone(),
                            first_boot_duration,
                            last_boot_duration,
                        },
                    ),
                )
                .await
                .ok();
        }
        Ok(())
    }

    async fn get_last_boot_duration(&self) -> Option<Duration> {
        let last_start_time = self.resources.last_start_time.read().await;
        let last_ready_time = self.resources.last_ready_time.read().await;

        if let (Some(start), Some(ready)) = (*last_start_time, *last_ready_time) {
            Some(ready.duration_since(start))
        } else {
            None
        }
    }

    // Helper method to get command sender
    async fn get_command_sender(&self) -> Result<mpsc::Sender<StateCommand>> {
        Ok(self.command_tx.clone())
    }

    // Method to get current state
    pub fn get_current_state(&self) -> MachineState {
        self.current_state.clone()
    }
}
