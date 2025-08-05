mod guest;
mod mount;
mod oci_config;
mod serial;

use std::{
    process::{Command, Stdio},
    sync::Arc,
    time::Duration,
};

use anyhow::{Result, bail};
use guest::GuestManager;
use mount::mount;
use nix::{
    libc,
    unistd::{chdir, chroot},
};
use oci_config::{EnvVar, OciConfig};
use serial::SerialWriter;
use takeoff_proto::proto::TakeoffInitArgs;
use tokio::{fs, time::sleep};
use tracing::{info, warn};

async fn takeoff() -> Result<()> {
    mount("devtmpfs", "/dev", Some("devtmpfs")).await;
    mount("proc", "/proc", Some("proc")).await;

    let guest_manager = Arc::new(GuestManager::new().expect("create guest manager"));

    let cmdline = fs::read_to_string("/proc/cmdline")
        .await
        .expect("read cmdline");
    let args = TakeoffInitArgs::try_parse_from_kernel_cmdline(&cmdline)?;

    info!("takeoff init args: {:#?}", args);

    let real_root = args.mount_points.first().expect("real root mount point");
    mount(&real_root.source, "/real_root", Some("ext4")).await;

    chroot("/real_root").expect("chroot");
    chdir("/").expect("chdir");

    mount("devtmpfs", "/dev", Some("devtmpfs")).await;
    mount("proc", "/proc", Some("proc")).await;
    mount("tmpfs", "/tmp", Some("tmpfs")).await;
    mount("tmpfs", "/run", Some("tmpfs")).await;

    for mount_point in args.mount_points.iter().skip(1) {
        mount(&mount_point.source, &mount_point.target, Some("ext4")).await;
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

    let mut envs = args.envs.clone();
    if let Some(config_envs) = config.env {
        for env in config_envs {
            let env_var: EnvVar = env.parse().expect("parse env var");
            envs.insert(env_var.key, env_var.value);
        }
    }

    info!("envs: {:#?}", envs);

    let result = unsafe { libc::unshare(libc::CLONE_NEWPID | libc::CLONE_NEWNS) };
    if result != 0 {
        let errno = std::io::Error::last_os_error();
        info!("failed to unshare PID namespace: {}", errno);
        bail!("failed to unshare PID namespace: {}", errno);
    }

    // remount proc
    mount("proc", "/proc", Some("proc")).await;

    let cmd = Command::new(cmd[0].clone())
        .args(&cmd[1..])
        .envs(envs)
        .current_dir(config.working_dir.unwrap_or("/".to_string()))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();

    let child = match cmd {
        Ok(child) => child,
        Err(e) => {
            bail!("failed to spawn command: {}", e);
        }
    };

    guest_manager.mark_user_space_ready();
    let output = child.wait_with_output().unwrap();

    info!("command exited with code {}", output.status.code().unwrap());
    warn!("stdout: {}", String::from_utf8_lossy(&output.stdout));
    warn!("stderr: {}", String::from_utf8_lossy(&output.stderr));

    if !output.status.success() {
        bail!("command failed: {}", output.status);
    }

    loop {
        sleep(Duration::from_secs(1)).await;
    }
}

fn main() -> Result<()> {
    SerialWriter::initialize_serial();

    tracing_subscriber::FmtSubscriber::builder()
        .with_max_level(tracing::Level::INFO)
        .with_writer(SerialWriter)
        .init();

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(takeoff())?;

    Ok(())
}
