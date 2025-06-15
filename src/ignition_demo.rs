use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use tracing_subscriber::FmtSubscriber;
use util::{
    async_runtime::{
        self,
        io::{AsyncReadExt, AsyncWriteExt},
        sync::{Mutex as AsyncMutex, Notify},
        task, time,
    },
    result::{bail, Context, Result},
    tracing::{self, error, info},
};

use vmm::{
    config::{BlockConfig, Config, KernelConfig, MemoryConfig, NetConfig, VcpuConfig},
    memory::Memory,
    state::VmmState,
    vmm::{Vmm, VmmStateController, VmmStateControllerMessage},
};

#[derive(Debug, Clone)]
enum VmStatus {
    Running { deadline: Instant },
    WaitForNet,
    Stopping,
    Stopped,
}

struct VmController {
    state: Arc<AsyncMutex<VmmState>>,
    memory: Arc<Memory>,
    status: Arc<AsyncMutex<VmStatus>>,
    status_notify: Arc<Notify>,
    startup_lock: Arc<AsyncMutex<()>>,
}

impl VmController {
    pub fn new(config: Config) -> Result<Self> {
        let (state, memory) = {
            let start_time = std::time::Instant::now();
            let memory = Vmm::create_memory_from_config(&config)?;
            let mut vm = Vmm::new(config.clone(), memory)?;
            let elapsed_us = start_time.elapsed().as_micros();
            info!("Initial VM creation took {}µs", elapsed_us);

            let start_time = std::time::Instant::now();
            let (state, memory) = vm.run().context("Failed to run VM initially")?;
            let elapsed_ms = start_time.elapsed().as_millis();
            info!("Initial VM run took {}ms", elapsed_ms);

            (state, memory)
        };

        let controller = Self {
            state: Arc::new(AsyncMutex::new(state)),
            memory,
            status: Arc::new(AsyncMutex::new(VmStatus::Stopped)),
            status_notify: Arc::new(Notify::new()),
            startup_lock: Arc::new(AsyncMutex::new(())),
        };

        Ok(controller)
    }

    fn start_vm_message_handler(&self, state_controller: VmmStateController) {
        let state = self.state.clone();
        let status = self.status.clone();
        let status_notify = self.status_notify.clone();

        let mut rx = state_controller.rx();
        task::spawn(async move {
            while let Ok(message) = rx.recv().await {
                match message {
                    VmmStateControllerMessage::Stopped(new_state) => {
                        {
                            let mut state_guard = state.lock().await;
                            *state_guard = new_state;
                        }

                        {
                            let mut status_guard = status.lock().await;
                            match *status_guard {
                                VmStatus::Stopping => {
                                    *status_guard = VmStatus::Stopped;
                                    info!("VM has stopped.");
                                }
                                VmStatus::Running { .. } => {
                                    *status_guard = VmStatus::Stopped;
                                    info!("VM has stopped unexpectedly.");
                                }
                                _ => {
                                    error!(
                                        "Received unexpected Stopped message while in state: {:?}",
                                        *status_guard
                                    );
                                }
                            }
                        }

                        status_notify.notify_waiters();
                    }
                    VmmStateControllerMessage::NetworkReady => {
                        info!("Network is ready.");

                        {
                            let mut status_guard = status.lock().await;
                            if matches!(*status_guard, VmStatus::WaitForNet) {
                                *status_guard = VmStatus::Running {
                                    deadline: Instant::now() + Duration::from_secs(10),
                                };
                            } else {
                                error!(
                                    "Received NetworkReady message while in state: {:?}",
                                    *status_guard
                                );
                            }
                        }

                        status_notify.notify_waiters();
                    }
                    _ => {}
                }
            }
        });
    }

    pub async fn prepare_for_request(&self) -> Result<()> {
        // First check if VM is already running without acquiring the startup lock
        {
            let status_guard = self.status.lock().await;
            if let VmStatus::Running { .. } = &*status_guard {
                // No need to update status here, we'll do it after releasing the lock
                return Ok(());
            }
        }

        // VM is not running, try to acquire startup lock
        // Only one thread can proceed past this point at a time
        let _startup_guard = self.startup_lock.lock().await;

        // Check status again after acquiring the lock
        // VM might have been started by another thread while we were waiting
        loop {
            {
                let mut status_guard = self.status.lock().await;

                match &*status_guard {
                    VmStatus::Running { .. } => {
                        *status_guard = VmStatus::Running {
                            deadline: Instant::now() + Duration::from_secs(10),
                        };
                        info!("VM is already running. Deadline updated.");
                        return Ok(());
                    }
                    VmStatus::Stopping => {
                        info!("VM is stopping. Awaiting current operation.");
                    }
                    VmStatus::WaitForNet => {
                        info!("VM is waiting for network. Awaiting network readiness.");
                    }
                    VmStatus::Stopped => {
                        info!("VM is stopped. Preparing to start.");
                        break;
                    }
                }
            }

            let timeout = time::timeout(Duration::from_secs(3), self.status_notify.notified());
            match timeout.await {
                Ok(_) => {}
                Err(_) => {
                    error!("Timeout waiting for VM state transition");

                    {
                        let mut status_guard = self.status.lock().await;
                        if matches!(*status_guard, VmStatus::Stopping) {
                            *status_guard = VmStatus::Stopped;
                            info!("Forced VM state to Stopped after timeout");
                        }
                    }
                }
            }
        }

        // Only one thread will ever reach this point at a time due to the startup_lock
        self.start_vm_from_state().await
    }

    async fn start_vm_from_state(&self) -> Result<()> {
        let state_clone = self.state.clone();
        let memory_clone = self.memory.clone();

        // Double-check current state to avoid starting VM that's not stopped
        {
            let status_guard = self.status.lock().await;
            if !matches!(*status_guard, VmStatus::Stopped) {
                error!(
                    "Attempting to start VM when it's not in Stopped state: {:?}",
                    *status_guard
                );
                bail!("Cannot start VM: invalid state transition");
            }
        }

        let old_state = {
            let state_guard = state_clone.lock().await;
            state_guard.clone()
        };

        let start_time = Instant::now();
        let vm_result = Vmm::from_state(old_state, memory_clone);

        let mut vm = match vm_result {
            Ok(vm) => vm,
            Err(e) => {
                error!("Failed to restore VM from state: {:?}", e);

                // Return to Stopped state when VM creation fails
                {
                    let mut status_guard = self.status.lock().await;
                    *status_guard = VmStatus::Stopped;
                }
                self.status_notify.notify_waiters();

                return Err(e.into());
            }
        };

        let state_controller = vm.controller();
        self.start_vm_message_handler(state_controller.clone());

        let elapsed_us = start_time.elapsed().as_micros();
        info!("VM restoration took {}µs", elapsed_us);

        // Update state before spawning the VM thread
        {
            let mut status_guard = self.status.lock().await;
            *status_guard = VmStatus::WaitForNet;
            info!("VM status set to WaitingForNet.");
        }
        self.status_notify.notify_waiters();

        task::spawn_blocking(move || {
            let start_time = Instant::now();
            let run_result = vm.run();
            let elapsed_ms = start_time.elapsed().as_millis();
            info!("VM run took {}ms", elapsed_ms);

            if let Err(e) = run_result {
                error!("VM run encountered an error: {:?}", e);
            }
        });

        let status_monitor = self.status.clone();
        let status_notify_monitor = self.status_notify.clone();
        let state_controller_monitor = state_controller.clone();

        task::spawn(async move {
            loop {
                time::sleep(Duration::from_secs(3)).await;

                let mut status_guard = status_monitor.lock().await;
                match &*status_guard {
                    VmStatus::Running { deadline } => {
                        if Instant::now() > *deadline {
                            info!("VM timeout reached. Initiating shutdown.");
                            *status_guard = VmStatus::Stopping;

                            // Drop the lock before calling request_stop to avoid deadlocks
                            drop(status_guard);

                            // Use timeout to avoid hanging if the request_stop call itself hangs
                            let stop_timeout = time::timeout(Duration::from_secs(2), async {
                                state_controller_monitor.request_stop();
                            });

                            if let Err(_) = stop_timeout.await {
                                error!("Timeout when requesting VM to stop");
                            }

                            status_notify_monitor.notify_waiters();
                            break;
                        }
                    }
                    VmStatus::Stopping => {
                        // VM is already in process of stopping
                        break;
                    }
                    VmStatus::Stopped => {
                        // VM is already stopped
                        break;
                    }
                    _ => {
                        // For any other state, continue monitoring
                    }
                }
            }
        });

        // Wait for VM to start up completely
        let mut retries = 0;
        let max_retries = 5;
        // Clone controller for potential use in timeout handling
        let state_controller_timeout = state_controller.clone();

        loop {
            let status = {
                let status = self.status.lock().await;
                status.clone()
            };

            match status {
                VmStatus::Running { .. } => {
                    info!("VM is running. Proceeding with request.");
                    break;
                }
                VmStatus::WaitForNet => {
                    // Continue waiting
                }
                VmStatus::Stopping | VmStatus::Stopped => {
                    // If we enter stopping or stopped state during startup, there was an error
                    error!("VM failed to start properly. Current status: {:?}", status);
                    bail!("VM failed to start properly");
                }
            };

            // Use a timeout to avoid infinite wait
            let timeout = time::timeout(Duration::from_secs(2), self.status_notify.notified());
            match timeout.await {
                Ok(_) => {
                    // Got notification, continue loop
                    retries = 0;
                }
                Err(_) => {
                    // Timeout occurred, increment retry counter
                    retries += 1;

                    if retries >= max_retries {
                        error!("Maximum retries exceeded waiting for VM to start");

                        // Clean up if we give up waiting
                        {
                            let mut status_guard = self.status.lock().await;
                            if matches!(*status_guard, VmStatus::WaitForNet) {
                                *status_guard = VmStatus::Stopping;
                            }
                        }

                        // Request stop to clean up resources
                        state_controller_timeout.request_stop();
                        self.status_notify.notify_waiters();

                        bail!("VM startup timed out");
                    }

                    // Check VM status again
                    let status = {
                        let status = self.status.lock().await;
                        status.clone()
                    };

                    match status {
                        VmStatus::WaitForNet => {
                            // Still waiting for network after timeout, continue waiting
                            info!(
                                "Still waiting for VM network after timeout ({}/{})",
                                retries, max_retries
                            );
                        }
                        _ => {}
                    }
                }
            }
        }

        Ok(())
    }
}

async fn ignition() -> Result<()> {
    let rootfs_path = std::env::args()
        .nth(1)
        .context("Rootfs path not provided")?;

    let subscriber = FmtSubscriber::builder()
        .with_max_level(tracing::Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .context("Failed to set global default subscriber")?;

    let config = Config::new()
        .memory(MemoryConfig::new(1024))
        .vcpu(VcpuConfig::new(2))
        .kernel(
            KernelConfig::new("../linux/vmlinux")?
                .with_initrd("./target/takeoff.cpio")
                .with_cmdline(
                    "i8042.nokbd reboot=t panic=1 noapic clocksource=kvm-clock tsc=reliable",
                )?
                .with_init_envs(vec!["IGNITION_DEMO=true"])?,
        )
        .with_net(NetConfig::new(
            "tap0",
            "172.16.0.2",
            "255.255.255.252",
            "172.16.0.1",
            "06:00:AC:10:00:02",
        ))
        .with_block(BlockConfig::new(rootfs_path).writeable())
        .with_snapshot_policy(vmm::config::SnapshotPolicy::OnNthListenSyscall(1))
        .into();

    info!("Initializing VM Controller.");
    let controller = Arc::new(VmController::new(config).context("Failed to create VmController")?);

    let listener = async_runtime::net::TcpListener::bind("0.0.0.0:9898")
        .await
        .context("Failed to bind to address")?;

    let addr = listener
        .local_addr()
        .context("Failed to get local address")?;
    info!("TCP proxy running on {}", addr);

    // Track active connections
    let active_connections = Arc::new(AsyncMutex::new(Vec::new()));

    loop {
        info!("Waiting for client connection...");
        let (client_stream, client_addr) = match listener.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                error!("Failed to accept client connection: {:?}", e);
                continue;
            }
        };
        info!("Accepted connection from {}", client_addr);

        // Set TCP nodelay on the client connection
        if let Err(e) = client_stream.set_nodelay(true) {
            error!("Failed to set nodelay on client connection: {:?}", e);
        }

        let controller = controller.clone();
        let active_connections = active_connections.clone();

        // Monitor VM status and close connections when VM stops
        let active_connections_monitor = active_connections.clone();
        let controller_monitor = controller.clone();
        task::spawn(async move {
            loop {
                time::sleep(Duration::from_secs(1)).await;
                let status = {
                    let status = controller_monitor.status.lock().await;
                    status.clone()
                };

                if matches!(status, VmStatus::Stopping | VmStatus::Stopped) {
                    info!("VM is stopping/stopped, closing all active connections");
                    let mut connections = active_connections_monitor.lock().await;
                    for addr in connections.drain(..) {
                        info!("Connection for {} will be closed when VM stops", addr);
                    }
                    break;
                }
            }
        });

        task::spawn(async move {
            info!("Handling connection from {}", client_addr);
            // Try to prepare VM with retries
            let mut retries = 0;
            let max_retries = 3;
            let mut vm_ready = false;

            while retries < max_retries {
                match controller.prepare_for_request().await {
                    Ok(_) => {
                        vm_ready = true;
                        break;
                    }
                    Err(e) => {
                        error!(
                            "Failed to prepare VM (attempt {}/{}): {:?}",
                            retries + 1,
                            max_retries,
                            e
                        );
                        retries += 1;
                        if retries < max_retries {
                            time::sleep(Duration::from_millis(500)).await;
                        }
                    }
                }
            }

            if !vm_ready {
                error!("Failed to prepare VM after {} attempts", max_retries);
                return;
            }

            // Wait for VM to be ready to accept connections
            let mut retries = 0;
            let max_retries = 10; // More retries for connection readiness
            let mut vm_stream = None;

            while retries < max_retries {
                info!(
                    "Attempting to connect to VM (attempt {}/{})",
                    retries + 1,
                    max_retries
                );
                // Try to connect to VM
                match async_runtime::net::TcpStream::connect("172.16.0.2:80").await {
                    Ok(stream) => {
                        // Set TCP nodelay on the VM connection
                        if let Err(e) = stream.set_nodelay(true) {
                            error!("Failed to set nodelay on VM connection: {:?}", e);
                        }

                        // Test if connection is actually ready by trying to write
                        match stream.writable().await {
                            Ok(_) => {
                                info!("Successfully connected to VM");
                                vm_stream = Some(stream);
                                break;
                            }
                            Err(e) => {
                                error!(
                                    "VM connection not ready (attempt {}/{}): {:?}",
                                    retries + 1,
                                    max_retries,
                                    e
                                );
                            }
                        }
                    }
                    Err(e) => {
                        error!(
                            "Failed to connect to VM (attempt {}/{}): {:?}",
                            retries + 1,
                            max_retries,
                            e
                        );
                    }
                }
                retries += 1;
                time::sleep(Duration::from_millis(500)).await;
            }

            let vm_stream = match vm_stream {
                Some(stream) => stream,
                None => {
                    error!("Failed to connect to VM after {} attempts", max_retries);
                    return;
                }
            };

            info!("Starting bidirectional copy for {}", client_addr);
            // Bidirectional copy between client and VM
            let (mut client_read, mut client_write) = client_stream.into_split();
            let (mut vm_read, mut vm_write) = vm_stream.into_split();

            // Add connection to active connections
            {
                let mut connections = active_connections.lock().await;
                connections.push(client_addr);
            }

            let client_to_vm = async {
                let mut buf = vec![0; 8192];
                loop {
                    match client_read.read(&mut buf).await {
                        Ok(0) => {
                            info!("Client {} closed connection", client_addr);
                            break;
                        }
                        Ok(n) => {
                            if let Err(e) = vm_write.write_all(&buf[..n]).await {
                                error!("Failed to write to VM: {:?}", e);
                                break;
                            }
                            // Ensure data is flushed
                            if let Err(e) = vm_write.flush().await {
                                error!("Failed to flush VM write: {:?}", e);
                                break;
                            }
                        }
                        Err(e) => {
                            error!("Failed to read from client: {:?}", e);
                            break;
                        }
                    }
                }
            };

            let vm_to_client = async {
                let mut buf = vec![0; 8192];
                loop {
                    match vm_read.read(&mut buf).await {
                        Ok(0) => {
                            info!("VM closed connection for client {}", client_addr);
                            break;
                        }
                        Ok(n) => {
                            if let Err(e) = client_write.write_all(&buf[..n]).await {
                                error!("Failed to write to client: {:?}", e);
                                break;
                            }
                            // Ensure data is flushed
                            if let Err(e) = client_write.flush().await {
                                error!("Failed to flush client write: {:?}", e);
                                break;
                            }
                        }
                        Err(e) => {
                            error!("Failed to read from VM: {:?}", e);
                            break;
                        }
                    }
                }
            };

            // Wait for either direction to complete
            async_runtime::select! {
                _ = client_to_vm => info!("Client to VM copy completed for {}", client_addr),
                _ = vm_to_client => info!("VM to client copy completed for {}", client_addr),
            }

            // Remove connection from active connections
            {
                let mut connections = active_connections.lock().await;
                connections.retain(|addr| *addr != client_addr);
            }
        });
    }
}

fn main() -> Result<()> {
    async_runtime::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(ignition())?;

    Ok(())
}
