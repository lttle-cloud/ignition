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
            vcpu::{RunningVcpuHandle, Vcpu, VcpuExitReason, VcpuRunResult},
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

    SystemExitCode { code: i32 },

    SystemSuspendTimeout { generation: u64 },

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
    command_rx: mpsc::UnboundedReceiver<StateCommand>,
    command_tx: mpsc::UnboundedSender<StateCommand>,
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
    last_exit_code: Arc<tokio::sync::RwLock<Option<i32>>>,
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

    pub async fn stop_all(&mut self, exit_reason: VcpuExitReason) -> Result<()> {
        let timeout = Duration::from_secs(30);
        let handles = self
            .running_vcpus
            .drain(..)
            .map(|handle| handle.signal_stop_and_join_with_timeout(exit_reason.clone(), timeout));

        let results = join_all(handles).await;

        let mut failed_vcpu_index = vec![];
        let mut timed_out_vcpus = vec![];

        for (i, result) in results.into_iter().enumerate() {
            match result {
                Ok(VcpuRunResult::Ok(vcpu)) => self.idle_vcpus.push(vcpu),
                Ok(VcpuRunResult::Error(e, vcpu)) => {
                    failed_vcpu_index.push((vcpu.index, e));
                    self.idle_vcpus.push(vcpu);
                }
                Err(()) => {
                    // Timeout - VCPU is stuck, can't recover it
                    timed_out_vcpus.push(i as u8);
                    warn!("VCPU {} timed out during stop, may be permanently stuck", i);
                }
            }
        }

        let mut has_errors = false;
        let mut fail_message = String::new();

        if !failed_vcpu_index.is_empty() {
            has_errors = true;
            fail_message.push_str("Failed to stop vcpus: ");
            for (index, e) in failed_vcpu_index {
                fail_message.push_str(&format!("Vcpu {} failed to stop: {}\n", index, e));
            }
        }

        if !timed_out_vcpus.is_empty() {
            has_errors = true;
            fail_message.push_str("Timed out vcpus: ");
            for index in timed_out_vcpus {
                fail_message.push_str(&format!("Vcpu {} timed out\n", index));
            }
            fail_message.push_str("VCPUs are stuck and machine needs force cleanup\n");
        }

        if has_errors {
            return Err(anyhow!("{}", fail_message));
        }

        Ok(())
    }
}

pub struct FlashLockTracker {
    active_count: u32,
    timeout_task: Option<JoinHandle<()>>,
    timeout_generation: u64,
    last_unlock_time: Option<Instant>,
}

impl FlashLockTracker {
    pub fn new() -> Self {
        Self {
            active_count: 0,
            timeout_task: None,
            timeout_generation: 0,
            last_unlock_time: None,
        }
    }

    pub fn add_flash_lock(&mut self) {
        self.active_count += 1;
        self.cancel_timeout();
        // Clear last unlock time since we have active connections again
        self.last_unlock_time = None;
    }

    pub fn remove_flash_lock(&mut self) -> bool {
        if self.active_count > 0 {
            self.active_count -= 1;
        }
        let is_last = self.active_count == 0;
        if is_last {
            self.last_unlock_time = Some(Instant::now());
        }
        is_last
    }

    pub fn has_active_locks(&self) -> bool {
        self.active_count > 0
    }

    pub fn cancel_timeout(&mut self) {
        if let Some(task) = self.timeout_task.take() {
            task.abort();
        }
    }

    pub fn start_timeout(
        &mut self,
        command_tx: mpsc::UnboundedSender<StateCommand>,
        timeout: Duration,
    ) {
        self.cancel_timeout();
        self.timeout_generation += 1;
        let generation = self.timeout_generation;

        info!(
            "Starting suspend timeout with generation {} for {}s",
            generation,
            timeout.as_secs()
        );

        let task = tokio::spawn(async move {
            sleep(timeout).await;
            let _ = command_tx.send(StateCommand::SystemSuspendTimeout { generation });
        });

        self.timeout_task = Some(task);
    }

    pub fn should_suspend(&self, timeout: Duration) -> bool {
        if self.has_active_locks() {
            return false;
        }

        if let Some(last_unlock) = self.last_unlock_time {
            last_unlock.elapsed() >= timeout
        } else {
            false
        }
    }

    pub fn current_generation(&self) -> u64 {
        self.timeout_generation
    }
}

impl MachineStateMachine {
    pub fn new(
        command_rx: mpsc::UnboundedReceiver<StateCommand>,
        command_tx: mpsc::UnboundedSender<StateCommand>,
        state_tx: broadcast::Sender<MachineState>,
        shared_state: Arc<tokio::sync::RwLock<MachineState>>,
        config: MachineConfig,
        vcpus: Vec<Vcpu>,
        devices: VmDevices,
        scheduler: std::sync::Weak<Scheduler>,
        first_boot_duration: Arc<tokio::sync::RwLock<Option<Duration>>>,
        last_start_time: Arc<tokio::sync::RwLock<Option<Instant>>>,
        last_ready_time: Arc<tokio::sync::RwLock<Option<Instant>>>,
        last_exit_code: Arc<tokio::sync::RwLock<Option<i32>>>,
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
            last_exit_code,
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
        let mut heartbeat_interval = tokio::time::interval(Duration::from_secs(1));

        loop {
            tokio::select! {
                Some(command) = self.command_rx.recv() => {
                    if let Err(e) = self.handle_command(command).await {
                        warn!("State machine error: {}", e);
                        if let Err(err) = self.transition_to_error(e.to_string()).await {
                            warn!("Failed to transition to error state: {}", err);
                        }
                    }
                }
                _ = heartbeat_interval.tick() => {
                    if let Err(e) = self.check_should_suspend().await {
                        warn!("Heartbeat check error: {}", e);
                    }
                }
                else => {
                    info!("Machine state machine stopped");
                    break;
                }
            }
        }
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

            StateCommand::SystemExitCode { code } => {
                self.handle_exit_code(code).await?;
            }

            StateCommand::SystemSuspendTimeout { generation } => {
                self.handle_suspend_timeout(generation).await?;
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
        let is_resume_from_suspend = self.current_state == MachineState::Suspended;

        // Reset guest manager for non-first starts
        if !is_first_start {
            self.resources
                .devices
                .guest_manager
                .lock()
                .expect("Failed to lock guest manager")
                .set_snapshot_strategy(None);
        }

        match self.current_state {
            MachineState::Idle | MachineState::Suspended => {
                // Set state to Booting and start VCPUs
                // For first start: SystemDeviceReady will transition to Ready
                // For resume from suspend: transition to Ready immediately since guest is already initialized
                info!(
                    "Starting machine: is_first_start={}, is_resume_from_suspend={}",
                    is_first_start, is_resume_from_suspend
                );
                self.set_state(MachineState::Booting).await?;
                self.resources.vcpu_manager.lock().await.start_all().await?;

                // When resuming from suspend, the guest is already in a ready state from the snapshot
                // Transition to Ready immediately without waiting for events
                // TODO: Temporarily disabled to observe failure logs
                if false && is_resume_from_suspend {
                    info!("Resuming from suspend, transitioning directly to Ready (bypassing event wait)");
                    self.set_state(MachineState::Ready).await?;
                } else {
                    if is_resume_from_suspend {
                        info!("Resuming from suspend, will wait for SystemVcpuRestarted event (FIX DISABLED FOR TESTING)");
                    } else {
                        info!("First start, will wait for SystemDeviceReady event");
                    }
                }

                Ok(())
            }
            MachineState::Booting | MachineState::Ready => {
                Ok(()) // Already started/starting
            }
            MachineState::Suspending => {
                // Wait for suspension to complete, then start
                info!(
                    "Machine '{}' is suspending, waiting for suspension to complete before starting",
                    self.resources.config.name
                );
                self.wait_for_suspension_complete().await?;
                // Once suspended, recursively call start to handle the Suspended state
                Box::pin(self.handle_user_start()).await
            }
            _ => Err(anyhow!("Can't start from {:?}", self.current_state)),
        }
    }

    async fn handle_user_stop(&mut self) -> Result<()> {
        let current_state = self.current_state.clone();
        match current_state {
            MachineState::Ready | MachineState::Booting => {
                self.set_state(MachineState::Stopping).await?;

                // Try to stop VCPUs, handle timeouts specially
                match self
                    .stop_vcpus_with_timeout_check(VcpuExitReason::Normal)
                    .await
                {
                    Ok(()) => {
                        self.set_state(MachineState::Stopped).await?;
                        Ok(())
                    }
                    Err(e) => {
                        let error_msg = e.to_string();
                        if error_msg.contains("timed out") || error_msg.contains("stuck") {
                            warn!(
                                "VCPU timeout detected during stop - transitioning to error state for cleanup"
                            );
                            let error_message = format!("VCPU timeout during stop: {}", error_msg);
                            self.transition_to_error(error_message).await
                        } else {
                            Err(e)
                        }
                    }
                }
            }
            MachineState::Suspended => {
                // Transition directly from Suspended to Stopped (VCPUs already stopped)
                self.set_state(MachineState::Stopped).await?;
                Ok(())
            }
            MachineState::Stopped => Ok(()),
            _ => Err(anyhow!("Can't stop from {:?}", current_state)),
        }
    }

    async fn stop_vcpus_with_timeout_check(&mut self, exit_reason: VcpuExitReason) -> Result<()> {
        self.resources
            .vcpu_manager
            .lock()
            .await
            .stop_all(exit_reason)
            .await
    }

    async fn wait_for_suspension_complete(&mut self) -> Result<()> {
        // Wait for the machine to transition from Suspending to Suspended or Error
        while self.current_state == MachineState::Suspending {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        // Check if suspension completed successfully
        match &self.current_state {
            MachineState::Suspended => Ok(()),
            MachineState::Error(msg) => Err(anyhow!("Suspension failed: {}", msg)),
            other => Err(anyhow!("Unexpected state after suspension: {:?}", other)),
        }
    }

    async fn handle_user_suspend(&mut self) -> Result<()> {
        let current_state = self.current_state.clone();
        match current_state {
            MachineState::Ready | MachineState::Booting => {
                self.set_state(MachineState::Suspending).await?;

                // For suspend, check if VCPUs timed out - this indicates serious issues
                match self
                    .stop_vcpus_with_timeout_check(VcpuExitReason::Suspend)
                    .await
                {
                    Ok(()) => {
                        self.set_state(MachineState::Suspended).await?;
                        Ok(())
                    }
                    Err(e) => {
                        let error_msg = e.to_string();
                        warn!("Failed to stop VCPUs during suspend: {}", error_msg);

                        // If VCPUs timed out, this is a serious issue requiring cleanup
                        if error_msg.contains("timed out") || error_msg.contains("stuck") {
                            warn!(
                                "VCPU timeout detected during suspend - transitioning to error state for cleanup"
                            );
                            self.transition_to_error(format!(
                                "VCPU timeout during suspend: {}",
                                error_msg
                            ))
                            .await
                        } else {
                            self.set_state(MachineState::Suspended).await?;
                            Ok(())
                        }
                    }
                }
            }
            MachineState::Suspended => Ok(()),
            _ => Err(anyhow!("Can't suspend from {:?}", current_state)),
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
        info!(
            "handle_vcpu_restarted called, current_state={:?}",
            self.current_state
        );
        if self.current_state == MachineState::Booting {
            info!("Transitioning from Booting to Ready due to VCPU restart");
            self.set_state(MachineState::Ready).await?;
        } else {
            info!(
                "Not transitioning to Ready, current state is {:?}",
                self.current_state
            );
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

        // Wake up suspended machine or cancel suspension in progress
        match self.current_state {
            MachineState::Suspended => {
                info!(
                    "Machine '{}' is suspended but has active flash locks, waking it up",
                    self.resources.config.name
                );
                self.handle_user_start().await?;
            }
            MachineState::Suspending => {
                info!(
                    "Machine '{}' is suspending but received flash lock, will start after suspension completes",
                    self.resources.config.name
                );
                // Start the machine - handle_user_start will wait for suspension to complete
                self.handle_user_start().await?;
            }
            _ => {
                // For other states, just track the flash lock
            }
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

    async fn handle_suspend_timeout(&mut self, generation: u64) -> Result<()> {
        // Validate generation to prevent stale timeout events
        let (current_generation, should_suspend) = {
            let tracker = self.resources.flash_lock_tracker.lock().await;
            (tracker.current_generation(), !tracker.has_active_locks())
        };

        if generation != current_generation {
            info!(
                "Ignoring stale suspend timeout (generation {} != current {})",
                generation, current_generation
            );
            return Ok(());
        }

        if should_suspend {
            info!(
                "Suspend timeout expired for machine '{}' (generation {}), suspending",
                self.resources.config.name, generation
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

    async fn handle_exit_code(&mut self, code: i32) -> Result<()> {
        *self.resources.last_exit_code.write().await = Some(code);
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

            // Spawn a non-blocking task to avoid circular dependency deadlock
            // State machine -> scheduler -> controller -> machine -> state machine
            let tenant = self.resources.config.controller_key.tenant.clone();
            let key = self.resources.config.controller_key.clone();
            let machine_id = self.resources.config.name.clone();
            let state_clone = state.clone();

            let notify_machine_id = machine_id.clone();
            tokio::spawn(async move {
                let result = scheduler
                    .push(
                        tenant,
                        ControllerEvent::AsyncWorkChange(
                            key,
                            AsyncWork::MachineStateChange {
                                machine_id,
                                state: state_clone,
                                first_boot_duration,
                                last_boot_duration,
                            },
                        ),
                    )
                    .await;

                if let Err(e) = result {
                    warn!(
                        "Failed to notify scheduler for machine '{}': {}",
                        notify_machine_id, e
                    );
                }
            });
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
    async fn get_command_sender(&self) -> Result<mpsc::UnboundedSender<StateCommand>> {
        Ok(self.command_tx.clone())
    }

    // Periodic heartbeat check to ensure machine suspends even if timeout events are lost
    async fn check_should_suspend(&mut self) -> Result<()> {
        // Only check in Ready state for Flash mode
        if self.current_state != MachineState::Ready {
            return Ok(());
        }

        let suspend_timeout = match &self.resources.config.mode {
            MachineMode::Flash { suspend_timeout, .. } => *suspend_timeout,
            MachineMode::Regular => return Ok(()),
        };

        let should_suspend = {
            let tracker = self.resources.flash_lock_tracker.lock().await;
            tracker.should_suspend(suspend_timeout)
        };

        if should_suspend {
            info!(
                "Heartbeat detected machine '{}' should be suspended (no locks, timeout expired)",
                self.resources.config.name
            );
            let _ = self.handle_user_suspend().await;
        }

        Ok(())
    }

    // Method to get current state
    pub fn get_current_state(&self) -> MachineState {
        self.current_state.clone()
    }
}
