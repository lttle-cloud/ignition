mod guest;
mod mount;
mod oci_config;
mod serial;

use std::{
    collections::HashMap, os::unix::process::ExitStatusExt, process::Stdio, sync::Arc,
    time::Duration,
};

use anyhow::{Result, bail};
use guest::GuestManager;
use mount::mount;
use nix::{
    libc::{self, c_int},
    unistd::{Group, User, chdir, chroot},
};
use oci_config::{EnvVar, OciConfig};
use serial::SerialWriter;
use takeoff_proto::proto::LogsTelemetryConfig;

use tokio::{
    fs,
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    process::Command,
    task::JoinHandle,
    time::sleep,
};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

use caps::{CapSet, Capability, CapsHashSet};
use tracing::{error, info, warn};

use opentelemetry::KeyValue;
use opentelemetry::logs::{AnyValue, LogRecord, Logger, LoggerProvider, Severity};
use opentelemetry_otlp::{Protocol, WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::logs::{BatchConfigBuilder, BatchLogProcessor, SdkLoggerProvider};

async fn takeoff() -> Result<()> {
    mount("proc", "/proc", Some("proc")).await;
    mount("devtmpfs", "/dev", Some("devtmpfs")).await;

    let guest_manager = Arc::new(GuestManager::new().expect("create guest manager"));

    let Ok(args) = guest_manager.read_takeoff_args() else {
        bail!("failed to read takeoff init args");
    };

    let cmdline = fs::read_to_string("/proc/cmdline")
        .await
        .expect("read cmdline");

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

    use nix::mount::MsFlags;
    mount::mount_with_options(
        "tmpfs",
        "/dev/shm",
        Some("tmpfs"),
        MsFlags::MS_NOSUID | MsFlags::MS_NODEV | MsFlags::MS_NOEXEC,
        Some("mode=1777,size=64m"),
    )
    .await;

    mount::mount_with_options(
        "mqueue",
        "/dev/mqueue",
        Some("mqueue"),
        MsFlags::MS_NOSUID | MsFlags::MS_NODEV | MsFlags::MS_NOEXEC,
        None,
    )
    .await;

    setup_pty_devices().await?;

    setup_additional_devices().await?;

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

    let mut cmd = config.entrypoint.clone().unwrap_or_default();
    if let Some(override_cmd) = args.cmd.clone() {
        cmd.extend(override_cmd);
    } else {
        cmd.extend(config.cmd.clone().unwrap_or_default());
    };
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

    let working_dir = config.working_dir.clone().unwrap_or("/".to_string());

    if cmd.is_empty() {
        let mut rec = cmd_logger.create_log_record();
        rec.set_severity_number(Severity::Error);
        rec.set_severity_text("ERROR");
        rec.add_attribute("log.stream", "stderr");
        rec.set_body(AnyValue::String("no entrypoint/cmd provided".into()));
        rec.add_attribute("process.status.success", false);
        rec.add_attribute("process.exit_code", 1i64);
        cmd_logger.emit(rec);
        guest_manager.set_exit_code(1);
        return Ok(());
    }

    // config.user is of the format "user:group", where either user or group can be string or number; group can be omitted
    let user = config.user.unwrap_or("root".to_string());
    let (specified_user, specified_group) = if let Some(pos) = user.find(':') {
        (user[..pos].to_string(), Some(user[pos + 1..].to_string()))
    } else {
        (user.to_string(), None)
    };
    info!(
        "specified_user: {:?}, specified_group: {:?}",
        specified_user, specified_group
    );

    let (uid, primary_gid) = if let Some(uid) = specified_user.parse::<u32>().ok() {
        // User specified as numeric UID - try to get primary group from passwd
        if let Ok(Some(user)) = User::from_uid(nix::unistd::Uid::from_raw(uid)) {
            (Some(uid), Some(user.gid.as_raw()))
        } else {
            (Some(uid), None)
        }
    } else {
        if let Ok(Some(user)) = User::from_name(&specified_user) {
            // User specified as name - get both UID and primary GID
            (Some(user.uid.as_raw()), Some(user.gid.as_raw()))
        } else {
            (None, None)
        }
    };

    let gid = if let Some(specified_group) = specified_group {
        // Group explicitly specified
        if let Some(gid) = specified_group.parse::<u32>().ok() {
            Some(gid)
        } else {
            if let Ok(Some(group)) = Group::from_name(&specified_group) {
                Some(group.gid.as_raw())
            } else {
                // Group name not found - this should typically be an error like Docker
                warn!(
                    "Group '{}' not found, falling back to primary group",
                    specified_group
                );
                primary_gid
            }
        }
    } else {
        // No group specified - use primary group from user lookup, or default
        info!("No group specified, using primary_gid: {:?}", primary_gid);
        primary_gid
    };

    let uid = uid.unwrap_or(0);
    let gid = gid.unwrap_or(0);

    info!("uid: {:?}; gid: {:?}", uid, gid);

    // Set HOME environment variable when running as non-root user (like Docker does)
    let mut envs = envs.clone();
    if uid != 0 && !envs.contains_key("HOME") {
        // Try to get home directory from passwd entry, fallback to working_dir
        let home_dir = if let Ok(Some(user)) = User::from_name(&specified_user) {
            user.dir.to_string_lossy().to_string()
        } else {
            working_dir.clone()
        };

        info!("Setting HOME environment variable to: {}", home_dir);
        envs.insert("HOME".to_string(), home_dir);
    }

    // Capabilities were already set after namespace creation
    if uid != 0 {
        info!("Non-root user - capabilities should have been set after unshare");
    }

    let mut command = Command::new(cmd[0].clone());
    command
        .args(&cmd[1..])
        .envs(envs.clone())
        .current_dir(working_dir.clone())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    command.envs(envs.clone());

    if uid != 0 {
        unsafe {
            command.pre_exec(move || {
                use nix::unistd::{Gid, Uid, setgroups, setresgid, setresuid};

                // 1. Set PR_SET_KEEPCAPS so capabilities survive UID change
                let result = libc::prctl(libc::PR_SET_KEEPCAPS, 1, 0, 0, 0);
                if result != 0 {
                    eprintln!(
                        "Failed to set PR_SET_KEEPCAPS: {}",
                        std::io::Error::last_os_error()
                    );
                    return Err(std::io::Error::last_os_error());
                }

                // 2. Drop groups and switch UID/GID
                if let Err(e) = setgroups(&[]) {
                    eprintln!("Failed to setgroups: {}", e);
                    return Err(e.into());
                }
                if let Err(e) =
                    setresgid(Gid::from_raw(gid), Gid::from_raw(gid), Gid::from_raw(gid))
                {
                    eprintln!("Failed to setresgid: {}", e);
                    return Err(e.into());
                }
                if let Err(e) =
                    setresuid(Uid::from_raw(uid), Uid::from_raw(uid), Uid::from_raw(uid))
                {
                    eprintln!("Failed to setresuid: {}", e);
                    return Err(e.into());
                }

                // 3. Set Docker default capability set
                let mut caps = CapsHashSet::new();
                caps.insert(Capability::CAP_CHOWN);
                caps.insert(Capability::CAP_DAC_OVERRIDE);
                caps.insert(Capability::CAP_FOWNER);
                caps.insert(Capability::CAP_FSETID);
                caps.insert(Capability::CAP_KILL);
                caps.insert(Capability::CAP_SETGID);
                caps.insert(Capability::CAP_SETUID);
                caps.insert(Capability::CAP_SETPCAP);
                caps.insert(Capability::CAP_NET_BIND_SERVICE);
                caps.insert(Capability::CAP_NET_RAW);
                caps.insert(Capability::CAP_SYS_CHROOT);
                caps.insert(Capability::CAP_MKNOD);
                caps.insert(Capability::CAP_AUDIT_WRITE);
                caps.insert(Capability::CAP_SETFCAP);

                // Set permitted and effective
                if let Err(e) = caps::set(None, CapSet::Permitted, &caps) {
                    eprintln!("Failed to set permitted caps: {}", e);
                    return Err(std::io::Error::from_raw_os_error(1));
                }
                if let Err(e) = caps::set(None, CapSet::Effective, &caps) {
                    eprintln!("Failed to set effective caps: {}", e);
                    return Err(std::io::Error::from_raw_os_error(1));
                }

                // Set inheritable for execve
                if let Err(e) = caps::set(None, CapSet::Inheritable, &caps) {
                    eprintln!("Failed to set inheritable caps: {}", e);
                    return Err(std::io::Error::from_raw_os_error(1));
                }

                // 4. Raise ambient capabilities for execve without file caps (Docker default set)
                for &cap in &caps {
                    let result = libc::prctl(
                        libc::PR_CAP_AMBIENT,
                        libc::PR_CAP_AMBIENT_RAISE,
                        cap.index() as c_int,
                        0,
                        0,
                    );
                    if result != 0 {
                        eprintln!(
                            "Failed to raise ambient cap {}: {}",
                            cap,
                            std::io::Error::last_os_error()
                        );
                        return Err(std::io::Error::last_os_error());
                    }
                }

                // 5. Clear PR_SET_KEEPCAPS
                let result = libc::prctl(libc::PR_SET_KEEPCAPS, 0, 0, 0, 0);
                if result != 0 {
                    eprintln!(
                        "Failed to clear PR_SET_KEEPCAPS: {}",
                        std::io::Error::last_os_error()
                    );
                    return Err(std::io::Error::last_os_error());
                }

                // 6. Set PR_SET_NO_NEW_PRIVS for security
                let result = libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0);
                if result != 0 {
                    eprintln!(
                        "Failed to set PR_SET_NO_NEW_PRIVS: {}",
                        std::io::Error::last_os_error()
                    );
                    return Err(std::io::Error::last_os_error());
                }

                // 7. Set PR_SET_DUMPABLE for debugging
                let result = libc::prctl(libc::PR_SET_DUMPABLE, 1, 0, 0, 0);
                if result != 0 {
                    eprintln!(
                        "Failed to set PR_SET_DUMPABLE: {}",
                        std::io::Error::last_os_error()
                    );
                    // Non-fatal, continue
                }

                eprintln!(
                    "Successfully dropped privileges to uid={}, gid={} with Docker default capabilities",
                    uid, gid
                );
                Ok(())
            });
        }
    }

    info!("About to spawn command: {:?}", cmd);
    info!("Working directory: {:?}", working_dir);
    info!("Environment variables: {:?}", envs);

    let mut child = command.spawn().map_err(|e| {
        info!("failed to spawn command: {}", e);
        e
    })?;

    tokio::spawn(run_exec_server(envs, working_dir));

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

                // Also log stderr to console for debugging
                error!("STDERR: {}", line);

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
    guest_manager.set_exit_code(status.code().unwrap_or(1));

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

    Ok(())
}

async fn setup_additional_devices() -> Result<()> {
    info!("Setting up additional devices for application compatibility");

    // Create essential device files that Chrome and other applications need
    let devices = [
        ("/dev/random", 0o666, 1, 8),
        ("/dev/urandom", 0o666, 1, 9),
        ("/dev/zero", 0o666, 1, 5),
        ("/dev/full", 0o666, 1, 7),
    ];

    for (device_path, mode, major, minor) in devices {
        if !std::path::Path::new(device_path).exists() {
            // Use libc mknod directly
            let path_cstring = std::ffi::CString::new(device_path)
                .map_err(|e| anyhow::anyhow!("Invalid path: {}", e))?;

            let dev_t = libc::makedev(major, minor);
            let result = unsafe { libc::mknod(path_cstring.as_ptr(), libc::S_IFCHR | mode, dev_t) };

            if result == 0 {
                info!("Created device file: {}", device_path);
            } else {
                let error = std::io::Error::last_os_error();
                info!("Could not create {}: {}", device_path, error);
            }
        }
    }

    Ok(())
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

async fn setup_pty_devices() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    info!("Setting up PTY devices for terminal support");

    // Create /dev/pts directory if it doesn't exist
    if let Err(e) = fs::create_dir_all("/dev/pts").await {
        info!("Warning: Could not create /dev/pts: {}", e);
    }

    // Mount devpts filesystem for PTY support with proper options
    // Use mount syscall directly for better control over mount options
    let result = unsafe {
        libc::mount(
            b"devpts\0".as_ptr() as *const libc::c_char,
            b"/dev/pts\0".as_ptr() as *const libc::c_char,
            b"devpts\0".as_ptr() as *const libc::c_char,
            0,
            b"newinstance,ptmxmode=0666,mode=0620,gid=5\0".as_ptr() as *const libc::c_void,
        )
    };

    if result == 0 {
        info!("Mounted devpts filesystem at /dev/pts with PTY support");
    } else {
        // Fallback to simple mount
        mount("devpts", "/dev/pts", Some("devpts")).await;
        info!("Mounted devpts filesystem at /dev/pts (fallback)");
    }

    // Create /dev/ptmx if it doesn't exist (some systems need this)
    if !std::path::Path::new("/dev/ptmx").exists() {
        // Try to create a symlink to /dev/pts/ptmx first (modern approach)
        if let Err(_) = fs::symlink("/dev/pts/ptmx", "/dev/ptmx").await {
            info!("Could not create /dev/ptmx symlink, PTY support may be limited");
        } else {
            info!("Created /dev/ptmx symlink to /dev/pts/ptmx");
        }
    }

    // Ensure proper permissions on /dev/pts
    if let Ok(metadata) = fs::metadata("/dev/pts").await {
        let mut perms = metadata.permissions();
        perms.set_mode(0o755);
        if let Err(e) = fs::set_permissions("/dev/pts", perms).await {
            info!("Warning: Could not set /dev/pts permissions: {}", e);
        }
    }

    info!("PTY device setup completed");
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

async fn handle_exec_request(
    stream: TcpStream,
    envs: HashMap<String, String>,
    working_dir: String,
) -> Result<()> {
    info!("Starting exec request handler");
    let (mut read_half, write_half) = stream.into_split();

    // Read exec request: [cmd_len: u32][cmd: string][stdin_flag: u8][tty_flag: u8]
    // Note: Terminal size is not sent by current client, so we use defaults
    let mut buf = [0; 4];
    read_half.read_exact(&mut buf).await?;
    let cmd_len = u32::from_le_bytes(buf) as usize;
    let mut cmd = vec![0; cmd_len];
    read_half.read_exact(&mut cmd).await?;
    let cmd = String::from_utf8(cmd)?;

    let mut flags = [0; 2];
    read_half.read_exact(&mut flags).await?;
    let stdin_enabled = flags[0] != 0;
    let tty_enabled = flags[1] != 0;

    info!(
        "Raw flags received: stdin_flag={}, tty_flag={}",
        flags[0], flags[1]
    );
    info!(
        "Parsed flags: stdin_enabled={}, tty_enabled={}",
        stdin_enabled, tty_enabled
    );

    // Use default terminal size since client doesn't send it yet
    let pty_size = PtySize {
        rows: 24,
        cols: 80,
        pixel_width: 0,
        pixel_height: 0,
    };

    info!(
        "exec request: {} (stdin: {}, tty: {}, size: {}x{})",
        cmd, stdin_enabled, tty_enabled, pty_size.rows, pty_size.cols
    );

    if tty_enabled {
        info!(
            "Using PTY mode with size {}x{}",
            pty_size.rows, pty_size.cols
        );
    } else {
        info!("Using pipe mode (no TTY)");
    }

    // Use command as-is - users should add -i flag if they want interactive shells
    let cmd_parts = vec!["/bin/sh".to_string(), "-c".to_string(), cmd];

    // Environment setup for terminal sessions
    let mut additional_envs = HashMap::new();
    if tty_enabled {
        // PTY sessions get full terminal support
        additional_envs.insert("TERM".to_string(), "xterm-256color".to_string());
        additional_envs.insert("COLORTERM".to_string(), "truecolor".to_string());
        additional_envs.insert("COLUMNS".to_string(), pty_size.cols.to_string());
        additional_envs.insert("LINES".to_string(), pty_size.rows.to_string());
    } else {
        // Non-PTY sessions get basic terminal support
        additional_envs.insert("TERM".to_string(), "dumb".to_string());
    }

    // Merge additional environment variables
    additional_envs.extend(envs);

    // Execute with PTY if tty_enabled, otherwise use pipes
    if tty_enabled {
        // Use PTY for true terminal-like behavior
        info!("Creating PTY system");
        let pty_system = native_pty_system();

        info!("Opening PTY with size {}x{}", pty_size.rows, pty_size.cols);
        let pty_pair = match pty_system.openpty(pty_size) {
            Ok(pair) => pair,
            Err(e) => {
                error!("Failed to create PTY: {}. Falling back to pipe mode.", e);
                info!("Falling back to pipe execution due to PTY unavailability");

                // Fallback to pipe mode
                let child = Command::new(&cmd_parts[0])
                    .args(&cmd_parts[1..])
                    .envs(additional_envs)
                    .current_dir(working_dir)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()?;

                let result =
                    handle_pipe_execution(child, read_half, write_half, stdin_enabled).await;
                info!("Pipe execution (fallback) completed: {:?}", result);
                return result;
            }
        };

        info!("Building command: {:?}", cmd_parts);
        let mut cmd = CommandBuilder::new(&cmd_parts[0]);
        cmd.args(&cmd_parts[1..]);
        for (key, value) in &additional_envs {
            info!("Setting env: {}={}", key, value);
            cmd.env(key, value);
        }
        cmd.cwd(&working_dir);

        info!("Spawning command in PTY: {:?}", cmd_parts);
        let child = pty_pair.slave.spawn_command(cmd)?;
        drop(pty_pair.slave);

        info!("PTY child process spawned, starting I/O handling");

        let result =
            handle_pty_execution(child, pty_pair.master, read_half, write_half, stdin_enabled)
                .await;
        info!("PTY execution completed: {:?}", result);
        result
    } else {
        // Use regular pipes for non-TTY execution
        let child = Command::new(&cmd_parts[0])
            .args(&cmd_parts[1..])
            .envs(additional_envs)
            .current_dir(working_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let result = handle_pipe_execution(child, read_half, write_half, stdin_enabled).await;
        info!("Pipe execution completed: {:?}", result);
        result
    }
}

/// Handle PTY execution with full I/O forwarding
///
/// Features:
/// - Bidirectional I/O between TCP socket and PTY master
/// - Signals (Ctrl+C, Ctrl+Z) are naturally handled by the PTY
/// - Job control works automatically due to PTY acting as controlling terminal
/// - Process groups are properly managed
/// - Uses blocking I/O in separate threads to avoid blocking the async runtime
async fn handle_pty_execution(
    mut pty: Box<dyn portable_pty::Child + Send + Sync>,
    pty_master: Box<dyn portable_pty::MasterPty + Send>,
    mut read_half: tokio::net::tcp::OwnedReadHalf,
    write_half: tokio::net::tcp::OwnedWriteHalf,
    stdin_enabled: bool,
) -> Result<()> {
    use std::io::Write;

    let write_half_clone = Arc::new(tokio::sync::Mutex::new(write_half));

    // Get reader and writer from PTY master
    let pty_reader = pty_master.try_clone_reader()?;
    let pty_writer = pty_master.take_writer()?;

    // Handle stdin: TCP -> PTY
    let stdin_task: JoinHandle<Result<()>> = if stdin_enabled {
        let pty_writer = Arc::new(std::sync::Mutex::new(pty_writer));
        tokio::spawn(async move {
            let mut buf = [0; 1024];
            while let Ok(n) = read_half.read(&mut buf).await {
                if n == 0 {
                    break;
                }
                // Write data to PTY using spawn_blocking for the blocking write
                let data = buf[..n].to_vec();
                let pty_writer = pty_writer.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let mut writer = pty_writer.lock().unwrap();
                    let write_result = writer.write_all(&data);
                    writer.flush().unwrap_or_default();
                    write_result
                })
                .await;

                if result.is_err() || result.unwrap().is_err() {
                    break;
                }
            }

            Ok(())
        })
    } else {
        tokio::spawn(async move { Ok(()) })
    };

    // Handle stdout: PTY -> TCP
    let output_task: JoinHandle<Result<()>> = {
        let write_half = write_half_clone.clone();
        let pty_reader = Arc::new(std::sync::Mutex::new(pty_reader));
        tokio::spawn(async move {
            use std::io::Read;

            // Handle PTY output: PTY -> TCP client
            loop {
                let pty_reader = pty_reader.clone();
                let read_result = tokio::task::spawn_blocking(move || {
                    let mut reader = pty_reader.lock().unwrap();
                    let mut buf = [0; 1024];
                    reader.read(&mut buf).map(|n| buf[..n].to_vec())
                })
                .await;

                match read_result {
                    Ok(Ok(data)) if !data.is_empty() => {
                        // Send PTY output to TCP client
                        if write_half.lock().await.write_all(&data).await.is_err() {
                            break;
                        }
                        if write_half.lock().await.flush().await.is_err() {
                            break;
                        }
                    }
                    Ok(Ok(_)) => {
                        // PTY EOF
                        break;
                    }
                    Ok(Err(_)) | Err(_) => {
                        // PTY read error or task error
                        break;
                    }
                }

                // Small delay to prevent busy loop
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }

            Ok(())
        })
    };

    // Monitor PTY process and handle completion

    // Wait for either PTY completion or I/O tasks to finish
    tokio::select! {
        _ = stdin_task => {},
        _ = output_task => {},
        _ = async {
            loop {
                if let Ok(Some(_)) = pty.try_wait() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        } => {},
    }

    // Kill the pty process if it's still running
    let _ = pty.kill();

    // Flush the write half to ensure all data is sent
    let _ = write_half_clone.lock().await.flush().await;

    let status = pty
        .wait()
        .unwrap_or_else(|_| portable_pty::ExitStatus::with_exit_code(1));

    if !status.success() {
        bail!("command failed: {}", status);
    }
    info!("command exited with code {:?}", status.exit_code());
    Ok(())
}

async fn handle_pipe_execution(
    mut child: tokio::process::Child,
    mut read_half: tokio::net::tcp::OwnedReadHalf,
    write_half: tokio::net::tcp::OwnedWriteHalf,
    stdin_enabled: bool,
) -> Result<()> {
    let mut stdin = child.stdin.take().expect("piped stdin");
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");

    let stdin_task: JoinHandle<Result<()>> = if stdin_enabled {
        tokio::spawn(async move {
            let mut buf = [0; 1024];
            while let Ok(n) = read_half.read(&mut buf).await {
                if n == 0 {
                    break;
                }
                if stdin.write_all(&buf[..n]).await.is_err() {
                    break;
                }
            }
            Ok(())
        })
    } else {
        tokio::spawn(async move { Ok(()) })
    };

    let write_half_clone = Arc::new(tokio::sync::Mutex::new(write_half));

    let stdout_task: JoinHandle<Result<()>> = {
        let write_half = write_half_clone.clone();
        tokio::spawn(async move {
            let mut buf = [0; 1024];
            while let Ok(n) = stdout.read(&mut buf).await {
                if n == 0 {
                    break;
                }
                if write_half.lock().await.write_all(&buf[..n]).await.is_err() {
                    break;
                }
            }
            Ok(())
        })
    };

    let stderr_task: JoinHandle<Result<()>> = {
        let write_half = write_half_clone.clone();
        tokio::spawn(async move {
            let mut buf = [0; 1024];
            while let Ok(n) = stderr.read(&mut buf).await {
                if n == 0 {
                    break;
                }
                if write_half.lock().await.write_all(&buf[..n]).await.is_err() {
                    break;
                }
            }
            Ok(())
        })
    };

    // Use select to stop all tasks when any one completes/fails
    let child_exit = tokio::select! {
        _ = stdin_task => {
            info!("Stdin task completed first");
            None
        },
        _ = stdout_task => {
            info!("Stdout task completed first");
            None
        },
        _ = stderr_task => {
            info!("Stderr task completed first");
            None
        },
        result = child.wait() => {
            info!("Child process completed first");
            Some(result)
        },
    };

    // Only kill the child if it hasn't already exited
    let status = if let Some(result) = child_exit {
        result.unwrap_or_else(|_| std::process::ExitStatus::from_raw(1))
    } else {
        info!("Waiting for child process to complete gracefully");
        child
            .wait()
            .await
            .unwrap_or_else(|_| std::process::ExitStatus::from_raw(1))
    };

    // Flush the write half to ensure all data is sent
    let _ = write_half_clone.lock().await.flush().await;

    if !status.success() {
        bail!("command failed: {}", status);
    }
    info!("command exited with code {:?}", status);
    Ok(())
}

async fn run_exec_server(envs: HashMap<String, String>, working_dir: String) -> Result<()> {
    let listener = TcpListener::bind("0.0.0.0:50051").await?;
    while let Ok((stream, _)) = listener.accept().await {
        let envs = envs.clone();
        let working_dir = working_dir.clone();
        tokio::spawn(async move {
            let result = handle_exec_request(stream, envs, working_dir).await;
            if let Err(e) = result {
                error!("Exec request failed: {}", e);
            } else {
                info!("Exec request completed successfully");
            }
        });
    }
    Ok(())
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
