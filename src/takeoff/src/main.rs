mod guest;
mod mount;
mod oci_config;
mod serial;

use std::{
    io::{self, BufReader},
    process::{Command, Stdio},
    sync::Arc,
    thread::spawn,
    time::Duration,
};

use anyhow::{Result, bail};
use guest::GuestManager;
use mount::mount;
use nix::unistd::{chdir, chroot};
use oci_config::{EnvVar, OciConfig};
use serial::SerialWriter;
use takeoff_proto::proto::TakeoffInitArgs;
use tokio::{fs, time::sleep};
use tracing::info;

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

    let cmd = Command::new(cmd[0].clone())
        .args(&cmd[1..])
        .envs(envs)
        .current_dir(config.working_dir.unwrap_or("/".to_string()))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();

    let mut child = match cmd {
        Ok(child) => child,
        Err(e) => {
            bail!("failed to spawn command: {}", e);
        }
    };

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    let stdout_thread = spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut writer = SerialWriter;
        io::copy(&mut reader, &mut writer).unwrap();
    });

    let stderr_thread = spawn(move || {
        let mut reader = BufReader::new(stderr);
        let mut writer = SerialWriter;
        io::copy(&mut reader, &mut writer).unwrap();
    });

    guest_manager.mark_user_space_ready();

    stdout_thread.join().expect("stdout thread join");
    stderr_thread.join().expect("stderr thread join");

    info!(
        "command exited with code {}",
        child.wait().unwrap().code().unwrap()
    );

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
