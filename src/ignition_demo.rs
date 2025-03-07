use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    extract::{OriginalUri, Path, State},
    http::HeaderValue,
    response::IntoResponse,
    routing::get,
    Router,
};
use reqwest::StatusCode;
use tracing::{error, info};
use tracing_subscriber::FmtSubscriber;
use util::{
    async_runtime::{
        self,
        sync::{Mutex as AsyncMutex, Notify},
        task, time,
    },
    result::{Context, Result},
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
}

impl VmController {
    pub fn new(config: Config) -> Result<Self> {
        let (state, memory) = {
            let start_time = std::time::Instant::now();
            let mut vm = Vmm::new(config.clone()).context("Failed to create Vmm")?;
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
                            *status_guard = VmStatus::Stopped;
                        }

                        status_notify.notify_waiters();

                        info!("VM has stopped.");
                    }
                    VmmStateControllerMessage::NetworkReady => {
                        info!("Network is ready.");

                        {
                            let mut status_guard = status.lock().await;
                            *status_guard = VmStatus::Running {
                                deadline: Instant::now() + Duration::from_secs(10),
                            };
                        }

                        status_notify.notify_waiters();
                    }
                    _ => {}
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

        let old_state = {
            let state_guard = state_clone.lock().await;
            state_guard.clone()
        };

        let start_time = Instant::now();
        let mut vm =
            Vmm::from_state(old_state, memory_clone).context("Failed to restore VM from state")?;
        let state_controller = vm.controller();
        self.start_vm_message_handler(state_controller.clone());

        let elapsed_us = start_time.elapsed().as_micros();
        info!("VM restoration took {}µs", elapsed_us);

        task::spawn_blocking(move || {
            let start_time = Instant::now();
            let run_result = vm.run();
            let elapsed_ms = start_time.elapsed().as_millis();
            info!("VM run took {}ms", elapsed_ms);

            if let Err(e) = run_result {
                error!("VM run encountered an error: {:?}", e);
            }
        });

        {
            let mut status_guard = self.status.lock().await;
            *status_guard = VmStatus::WaitForNet;
            info!("VM status set to WaitingForNet.");
        }

        self.status_notify.notify_waiters();

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
                            state_controller.request_stop();
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

async fn handle_vm_request(
    OriginalUri(original_uri): OriginalUri,
    State(controller): State<Arc<VmController>>,
) -> impl IntoResponse {
    let start_time_total = Instant::now();

    if let Err(e) = controller.prepare_for_request().await {
        error!("Failed to prepare VM for request: {:?}", e);
        return format!("Internal Server Error: {:?}", e).into_response();
    }

    let elapsed_ms = start_time_total.elapsed().as_millis();
    info!("VM prepare took {}ms", elapsed_ms);

    let path = original_uri.path();
    info!("Path: {}", path);

    let start_time = Instant::now();
    let res = reqwest::get(format!("http://172.16.0.2:3000/{}", path)).await;

    let elapsed_ms = start_time.elapsed().as_millis();
    info!("Proxy request took {}ms", elapsed_ms);

    let (response_text, status_code, content_type) = match res {
        Ok(resp) => {
            let status_code = resp.status();
            let content_type = resp
                .headers()
                .get("content-type")
                .unwrap_or(&HeaderValue::from_static("text/html"))
                .clone();

            match resp.text().await {
                Ok(text) => (text, status_code, content_type),
                Err(e) => {
                    error!("Failed to read response text: {:?}", e);
                    (
                        format!("Error reading response: {:?}", e),
                        status_code,
                        content_type,
                    )
                }
            }
        }
        Err(e) => {
            error!("Failed to perform proxy request: {:?}", e);
            (
                format!("Error performing proxy request: {:?}", e),
                StatusCode::INTERNAL_SERVER_ERROR,
                HeaderValue::from_static("text/html"),
            )
        }
    };

    let total_elapsed = start_time_total.elapsed().as_millis();
    info!("Total time taken: {}ms", total_elapsed);

    let mut res = response_text.into_response();
    res.headers_mut().insert("content-type", content_type);
    *res.status_mut() = status_code;
    res.headers_mut()
        .insert("x-powered-by", HeaderValue::from_static("ignition"));

    res
}

async fn ignition() -> Result<()> {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(tracing::Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .context("Failed to set global default subscriber")?;

    let config = Config::new()
        .memory(MemoryConfig::new(128))
        .vcpu(VcpuConfig::new(1))
        .kernel(
            KernelConfig::new("../linux/vmlinux")?
                .with_initrd("./target/takeoff.cpio")
                .with_cmdline(
                    "i8042.nokbd reboot=t panic=1 noapic clocksource=kvm-clock tsc=reliable",
                )?,
        )
        .with_net(NetConfig::new(
            "tap0",
            "172.16.0.2",
            "255.255.255.252",
            "172.16.0.1",
            "06:00:AC:10:00:02",
        ))
        .with_block(BlockConfig::new("./target/hello-page.ext4").writeable())
        .into();

    info!("Initializing VM Controller.");
    let controller = Arc::new(VmController::new(config).context("Failed to create VmController")?);

    let app = Router::new()
        .route("/", get(handle_vm_request))
        .route("/{*path}", get(handle_vm_request))
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
