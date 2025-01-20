use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{extract::State, routing::get, Router};
use tracing::{error, info};
use tracing_subscriber::FmtSubscriber;
use util::{
    async_runtime::{
        self,
        sync::{mpsc, Mutex as AsyncMutex, Notify},
        task, time,
    },
    result::{Context, Result},
};

use vmm::{
    config::{Config, KernelConfig, MemoryConfig, NetConfig, VcpuConfig},
    memory::Memory,
    state::VmmState,
    vmm::Vmm,
};

#[derive(Debug, Clone)]
enum VmStatus {
    Running { deadline: Instant },
    WaitForNet,
    Stopping,
    Stopped,
}

enum VmMessage {
    VmStopped(VmmState),
    NetworkReady,
}

struct VmController {
    state: Arc<AsyncMutex<VmmState>>,
    memory: Arc<Memory>,
    status: Arc<AsyncMutex<VmStatus>>,
    stop_tx: Arc<AsyncMutex<Option<std::sync::mpsc::Sender<()>>>>,
    status_notify: Arc<Notify>,
    vm_message_tx: mpsc::Sender<VmMessage>,
}

impl VmController {
    pub fn new(config: Config) -> Result<Self> {
        let (state, memory) = {
            let start_time = std::time::Instant::now();
            let mut vm = Vmm::new(config.clone(), None).context("Failed to create Vmm")?;
            let elapsed_us = start_time.elapsed().as_micros();
            info!("Initial VM creation took {}µs", elapsed_us);

            let start_time = std::time::Instant::now();
            let (state, memory) = vm.run(None).context("Failed to run VM initially")?;
            let elapsed_ms = start_time.elapsed().as_millis();
            info!("Initial VM run took {}ms", elapsed_ms);

            (state, memory)
        };

        let (vm_message_tx, vm_message_rx) = mpsc::channel::<VmMessage>(100);

        let controller = Self {
            state: Arc::new(AsyncMutex::new(state)),
            memory,
            status: Arc::new(AsyncMutex::new(VmStatus::Stopped)),
            stop_tx: Arc::new(AsyncMutex::new(None)),
            status_notify: Arc::new(Notify::new()),
            vm_message_tx,
        };

        controller.start_vm_message_handler(vm_message_rx);

        Ok(controller)
    }

    fn start_vm_message_handler(&self, mut vm_message_rx: mpsc::Receiver<VmMessage>) {
        let state = self.state.clone();
        let status = self.status.clone();
        let status_notify = self.status_notify.clone();

        task::spawn(async move {
            while let Some(message) = vm_message_rx.recv().await {
                match message {
                    VmMessage::VmStopped(new_state) => {
                        {
                            let mut state_guard = state.lock().await;
                            *state_guard = new_state;
                        }

                        {
                            let mut status_guard = status.lock().await;
                            *status_guard = VmStatus::Stopped;
                        }

                        status_notify.notify_waiters();

                        info!("VM has stopped.");
                    }
                    VmMessage::NetworkReady => {
                        info!("Network is ready.");

                        {
                            let mut status_guard = status.lock().await;
                            *status_guard = VmStatus::Running {
                                deadline: Instant::now() + Duration::from_secs(10),
                            };
                        }

                        status_notify.notify_waiters();
                    }
                }
            }
        });
    }

    pub async fn prepare_for_request(&self) -> Result<()> {
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

            self.status_notify.notified().await;
        }

        self.start_vm_from_state().await?;

        Ok(())
    }

    async fn start_vm_from_state(&self) -> Result<()> {
        let state_clone = self.state.clone();
        let memory_clone = self.memory.clone();
        let vm_message_tx = self.vm_message_tx.clone();

        let old_state = {
            let state_guard = state_clone.lock().await;
            state_guard.clone()
        };

        let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
        let (net_ready_tx, net_ready_rx) = std::sync::mpsc::channel::<()>();

        let start_time = Instant::now();
        let mut vm = Vmm::from_state(old_state, memory_clone, net_ready_tx.into())
            .context("Failed to restore VM from state")?;
        let elapsed_us = start_time.elapsed().as_micros();
        info!("VM restoration took {}µs", elapsed_us);

        {
            let mut stop_tx_guard = self.stop_tx.lock().await;
            *stop_tx_guard = Some(stop_tx);
        }

        let run_vm_message_tx = vm_message_tx.clone();
        task::spawn_blocking(move || {
            let start_time = Instant::now();
            let run_result = vm.run(Some(stop_rx));
            let elapsed_ms = start_time.elapsed().as_millis();
            info!("VM run took {}ms", elapsed_ms);

            match run_result {
                Ok((new_state, _)) => {
                    let _ = run_vm_message_tx.blocking_send(VmMessage::VmStopped(new_state));
                }
                Err(e) => {
                    error!("VM run encountered an error: {:?}", e);
                }
            }
        });

        let network_message_tx = vm_message_tx.clone();
        task::spawn_blocking(move || {
            let timeout_duration = Duration::from_secs(5);
            match net_ready_rx.recv_timeout(timeout_duration) {
                Ok(_) => {
                    let _ = network_message_tx.blocking_send(VmMessage::NetworkReady);
                }
                Err(e) => {
                    error!(
                        "Failed to receive network readiness signal within timeout: {:?}",
                        e
                    );
                }
            }
        });

        {
            let mut status_guard = self.status.lock().await;
            *status_guard = VmStatus::WaitForNet;
            info!("VM status set to WaitingForNet.");
        }

        self.status_notify.notify_waiters();

        let stop_tx_clone = {
            let stop_tx_guard = self.stop_tx.lock().await;
            stop_tx_guard.clone()
        };

        let status_monitor = self.status.clone();
        let status_notify_monitor = self.status_notify.clone();
        task::spawn(async move {
            loop {
                time::sleep(Duration::from_secs(3)).await;

                let mut status_guard = status_monitor.lock().await;
                match &*status_guard {
                    VmStatus::Running { deadline } => {
                        if Instant::now() > *deadline {
                            info!("VM timeout reached. Initiating shutdown.");
                            *status_guard = VmStatus::Stopping;
                            if let Some(sender) = &stop_tx_clone {
                                if let Err(e) = sender.send(()) {
                                    error!("Failed to send stop signal: {:?}", e);
                                }
                            }
                            status_notify_monitor.notify_waiters();
                            break;
                        }
                    }
                    _ => break,
                }
            }
        });

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
                _ => {}
            };

            self.status_notify.notified().await;
        }

        Ok(())
    }
}

async fn handle_vm_request(State(controller): State<Arc<VmController>>) -> String {
    let start_time_total = Instant::now();

    if let Err(e) = controller.prepare_for_request().await {
        error!("Failed to prepare VM for request: {:?}", e);
        return format!("Internal Server Error: {:?}", e);
    }

    let elapsed_ms = start_time_total.elapsed().as_millis();
    info!("VM prepare took {}ms", elapsed_ms);

    let start_time = Instant::now();
    let res = reqwest::get("http://172.16.0.2:3000/").await;

    let elapsed_ms = start_time.elapsed().as_millis();
    info!("Proxy request took {}ms", elapsed_ms);

    let response_text = match res {
        Ok(resp) => match resp.text().await {
            Ok(text) => text,
            Err(e) => {
                error!("Failed to read response text: {:?}", e);
                format!("Error reading response: {:?}", e)
            }
        },
        Err(e) => {
            error!("Failed to perform proxy request: {:?}", e);
            format!("Error performing proxy request: {:?}", e)
        }
    };

    let total_elapsed = start_time_total.elapsed().as_millis();
    info!("Total time taken: {}ms", total_elapsed);

    response_text
}

async fn ignition() -> Result<()> {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(tracing::Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .context("Failed to set global default subscriber")?;

    let memory_config = MemoryConfig {
        size_mib: 128,
        // path: Some("./memory.bin".into()),
        path: None,
    };

    let config = Config {
        memory: memory_config.clone(),
        vcpu: VcpuConfig { num: 1 },
        kernel: KernelConfig::builder("../linux/vmlinux")?
            .with_initrd("./target/takeoff.cpio")
            .with_cmdline("i8042.nokbd reboot=t panic=1 noapic clocksource=kvm-clock tsc=reliable")?
            .build(),
        net: NetConfig {
            tap_name: "tap0".into(),
            ip_addr: "172.16.0.2".into(),
            netmask: "255.255.255.252".into(),
            gateway: "172.16.0.1".into(),
            mac_addr: "06:00:AC:10:00:02".into(),
        }
        .into(),
    };

    info!("Initializing VM Controller.");
    let controller = Arc::new(VmController::new(config).context("Failed to create VmController")?);

    let app = Router::new()
        .route("/vm", get(handle_vm_request))
        .with_state(controller.clone());

    let listener = async_runtime::net::TcpListener::bind("0.0.0.0:9898")
        .await
        .context("Failed to bind to address")?;

    let addr = listener
        .local_addr()
        .context("Failed to get local address")?;
    info!("Ignition server running on {}", addr);

    axum::serve(listener, app).await.unwrap();

    Ok(())
}

fn main() -> Result<()> {
    async_runtime::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(ignition())?;

    Ok(())
}
