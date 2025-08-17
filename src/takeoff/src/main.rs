mod guest;
mod mount;
mod oci_config;
mod serial;

use std::{collections::HashMap, process::Stdio, sync::Arc, time::Duration};

use anyhow::{Result, bail};
use guest::GuestManager;
use mount::mount;
use nix::{
    libc,
    unistd::{chdir, chroot},
};
use oci_config::{EnvVar, OciConfig};
use serial::SerialWriter;
use takeoff_proto::proto::{LogsTelemetryConfig, TakeoffInitArgs};

use tokio::{
    fs,
    io::AsyncWriteExt,
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    time::sleep,
};

use tracing::{error, info};

use opentelemetry::KeyValue;
use opentelemetry::logs::{AnyValue, LogRecord, Logger, LoggerProvider, Severity};
use opentelemetry_otlp::{Protocol, WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::logs::{BatchConfigBuilder, BatchLogProcessor, SdkLoggerProvider};

async fn takeoff() -> Result<()> {
    mount("proc", "/proc", Some("proc")).await;
    mount("devtmpfs", "/dev", Some("devtmpfs")).await;

    let guest_manager = Arc::new(GuestManager::new().expect("create guest manager"));

    let cmdline = fs::read_to_string("/proc/cmdline")
        .await
        .expect("read cmdline");
    let args = TakeoffInitArgs::try_parse_from_kernel_cmdline(&cmdline)?;

    configure_dns(&cmdline).await?;

    info!("takeoff init args: {:#?}", args);

    let real_root = args.mount_points.first().expect("real root mount point");
    mount(&real_root.source, "/real_root", Some("ext4")).await;

    chroot("/real_root").expect("chroot");
    chdir("/").expect("chdir");

    mount("proc", "/proc", Some("proc")).await;
    mount("devtmpfs", "/dev", Some("devtmpfs")).await;
    mount("tmpfs", "/tmp", Some("tmpfs")).await;
    mount("tmpfs", "/run", Some("tmpfs")).await;

    configure_dns(&cmdline).await?;

    for mount_point in args.mount_points.iter().skip(1) {
        info!(
            "mounting {} to {} (read-only: {})",
            mount_point.source, mount_point.target, mount_point.read_only
        );
        mount(&mount_point.source, &mount_point.target, Some("ext4")).await;
        if !mount_point.read_only {
            let _ = fs::remove_dir_all(format!("{}/lost+found", mount_point.target)).await;
        }
    }

    let config = fs::read_to_string("/etc/lttle/oci-config.json")
        .await
        .unwrap();
    let config: OciConfig = serde_json::from_str(&config).unwrap();
    info!("oci_config: {:#?}", config);

    let mut cmd = vec![];
    cmd.extend(config.entrypoint.clone().unwrap_or_default());
    cmd.extend(config.cmd.clone().unwrap_or_default());
    info!("cmd: {:?}", cmd);

    let mut envs = HashMap::new();
    if let Some(config_envs) = config.env {
        for env in config_envs {
            let env_var: EnvVar = env.parse().expect("parse env var");
            envs.insert(env_var.key, env_var.value);
        }
    }
    envs.extend(args.envs);

    info!("envs: {:#?}", envs);

    let result = unsafe { libc::unshare(libc::CLONE_NEWPID | libc::CLONE_NEWNS) };
    if result != 0 {
        let errno = std::io::Error::last_os_error();
        info!("failed to unshare PID namespace: {}", errno);
        bail!("failed to unshare PID namespace: {}", errno);
    }

    mount("proc", "/proc", Some("proc")).await;

    for (link, target) in [
        ("/dev/fd", "/proc/self/fd"),
        ("/dev/stdin", "/proc/self/fd/0"),
        ("/dev/stdout", "/proc/self/fd/1"),
        ("/dev/stderr", "/proc/self/fd/2"),
    ] {
        let _ = fs::remove_file(link).await;
        let _ = fs::symlink(target, link).await;
    }
    let telemetry_config = args.logs_telemetry_config.clone();
    let otel_provider = tokio::task::spawn_blocking(move || {
        tokio::runtime::Handle::current().block_on(init_otel_logger(telemetry_config))
    })
    .await??;

    let stdout_logger = otel_provider.logger(format!(
        "{}/stdout",
        args.logs_telemetry_config.service_name
    ));
    let stderr_logger = otel_provider.logger(format!(
        "{}/stderr",
        args.logs_telemetry_config.service_name
    ));
    let cmd_logger =
        otel_provider.logger(format!("{}/cmd", args.logs_telemetry_config.service_name));

    let mut child = Command::new(cmd[0].clone())
        .args(&cmd[1..])
        .envs(envs)
        .current_dir(config.working_dir.clone().unwrap_or("/".to_string()))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            info!("failed to spawn command: {}", e);
            e
        })?;

    guest_manager.mark_user_space_ready();

    let pid = child.id();

    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");

    let out_task = {
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.is_empty() {
                    continue;
                }

                let mut rec = stdout_logger.create_log_record();
                rec.set_severity_number(Severity::Info);
                rec.set_severity_text("INFO");
                rec.set_body(AnyValue::String(line.into()));
                rec.add_attribute("log.stream", "stdout");
                if let Some(pid) = pid {
                    rec.add_attribute("process.pid", pid as i64);
                }
                stdout_logger.emit(rec);
            }
        })
    };

    let err_task = {
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.is_empty() {
                    continue;
                }

                let mut rec = stderr_logger.create_log_record();
                rec.set_severity_number(Severity::Error);
                rec.set_severity_text("ERROR");
                rec.set_body(AnyValue::String(line.into()));
                rec.add_attribute("log.stream", "stderr");
                if let Some(pid) = pid {
                    rec.add_attribute("process.pid", pid as i64);
                }
                stderr_logger.emit(rec);
            }
        })
    };

    let status = child.wait().await?;
    let _ = out_task.await;
    let _ = err_task.await;

    info!("command exited with code {:?}", status.code());

    {
        let mut rec = cmd_logger.create_log_record();
        if status.success() {
            rec.set_severity_number(Severity::Info);
            rec.set_severity_text("INFO");
            rec.add_attribute("log.stream", "stdout");
        } else {
            rec.set_severity_number(Severity::Error);
            rec.set_severity_text("ERROR");
            rec.add_attribute("log.stream", "stderr");
        }
        rec.set_body(AnyValue::String(
            format!("process exited: {}", status).into(),
        ));
        if let Some(pid) = pid {
            rec.add_attribute("process.pid", pid as i64);
        }
        rec.add_attribute("process.status.success", status.success());
        if let Some(code) = status.code() {
            rec.add_attribute("process.exit_code", code as i64);
        }
        cmd_logger.emit(rec);
    }

    otel_provider.force_flush()?;

    if !status.success() {
        info!("command failed: {}", status);
        bail!("command failed: {}", status);
    }

    loop {
        sleep(Duration::from_secs(1)).await;
    }
}

async fn configure_dns(cmdline: &str) -> Result<()> {
    // Parse nameserver parameters from kernel cmdline
    let mut nameservers = Vec::new();

    for param in cmdline.split_whitespace() {
        if param.starts_with("nameserver") {
            if let Some(dns) = param.split('=').nth(1) {
                nameservers.push(dns);
            }
        }
    }

    if !nameservers.is_empty() {
        fs::create_dir_all("/etc").await.ok();

        let mut resolv_conf = String::new();
        resolv_conf.push_str("# Generated by Ignition takeoff init\n");

        for ns in &nameservers {
            resolv_conf.push_str(&format!("nameserver {}\n", ns));
        }

        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open("/etc/resolv.conf")
            .await?;

        file.write_all(resolv_conf.as_bytes()).await?;
        file.flush().await?;

        info!(
            "Created /etc/resolv.conf with DNS servers: {:?}",
            nameservers
        );
    }

    Ok(())
}

async fn init_otel_logger(cfg: LogsTelemetryConfig) -> Result<SdkLoggerProvider> {
    let exporter = opentelemetry_otlp::LogExporter::builder()
        .with_http()
        .with_protocol(Protocol::HttpBinary)
        .with_endpoint(&cfg.endpoint)
        .with_headers(HashMap::from([(
            "X-Scope-OrgID".to_string(),
            cfg.tenant_id.clone(),
        )]))
        .build();

    if exporter.is_err() {
        info!("failed to build exporter: {:?}", exporter.err());
        loop {
            sleep(Duration::from_secs(1)).await;
        }
    }
    info!("exporter built");
    let exporter = exporter.unwrap();

    let resource = Resource::builder()
        .with_attributes(vec![
            KeyValue::new("service.name", cfg.service_name.clone()),
            KeyValue::new("service.namespace", cfg.service_namespace.clone()),
            KeyValue::new("service.group", cfg.service_group.clone()),
            KeyValue::new("service.tenant", cfg.tenant_id.clone()),
        ])
        .build();

    let provider = SdkLoggerProvider::builder()
        .with_log_processor(
            BatchLogProcessor::builder(exporter)
                .with_batch_config(
                    BatchConfigBuilder::default()
                        .with_scheduled_delay(Duration::from_millis(200))
                        .build(),
                )
                .build(),
        )
        .with_resource(resource)
        .build();

    Ok(provider)
}

fn main() -> Result<()> {
    SerialWriter::initialize_serial();

    tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(tracing::Level::INFO)
        .with_writer(SerialWriter)
        .init();

    std::panic::set_hook(Box::new(|panic_info| {
        let message = if let Some(s) = panic_info.payload().downcast_ref::<&str>() {
            s
        } else if let Some(s) = panic_info.payload().downcast_ref::<String>() {
            s.as_str()
        } else {
            "Box<dyn Any>"
        };

        let location = if let Some(location) = panic_info.location() {
            format!(
                " at {}:{}:{}",
                location.file(),
                location.line(),
                location.column()
            )
        } else {
            String::new()
        };

        error!("panic: {}{}", message, location);
    }));

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(takeoff())?;

    Ok(())
}
