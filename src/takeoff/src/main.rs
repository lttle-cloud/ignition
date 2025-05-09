mod guest;
mod oci_config;
mod serial;

use clap::Parser;
use guest::GuestManager;
use nix::{
    mount::{self, MsFlags},
    unistd::{chdir, chroot},
};
use oci_config::OciConfig;
use serial::SerialWriter;
use std::{
    collections::HashMap,
    io::{self, BufReader},
    path::PathBuf,
    process::{Command, Stdio},
    str::FromStr,
    sync::Arc,
    thread::spawn,
    time::Duration,
};
use tracing::{error, info};
use util::result::{bail, Error};
use util::{
    async_runtime::{self, fs, time::sleep},
    result::Result,
};

async fn mount(device: &str, mount_point: &str, fs_type: Option<&str>) {
    // make sure mount point exists
    fs::create_dir_all(mount_point)
        .await
        .expect("create mount point");

    if let Err(e) = mount::mount(
        Some(&PathBuf::from(device)),
        mount_point,
        fs_type,
        MsFlags::empty(),
        Some(&PathBuf::from("")),
    ) {
        info!("mount {} failed: {:?}", mount_point, e);
    }
}

#[derive(Debug, Clone)]
struct EnvVar {
    key: String,
    value: String,
}

impl FromStr for EnvVar {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.splitn(2, '=').collect();
        if parts.len() != 2 {
            bail!("invalid env var: {}", s);
        }
        Ok(EnvVar {
            key: parts[0].to_string(),
            value: parts[1].to_string(),
        })
    }
}

#[derive(Parser, Debug, Clone)]
#[command(name = "takeoff", ignore_errors = true)]
struct TakeoffKernelArgs {
    #[arg(long = "takeoff-env")]
    pub envs: Vec<String>,
}

impl TakeoffKernelArgs {
    pub fn try_parse_from_kernel_cmdline(cmdline: &str) -> Result<Self> {
        let mut takeoff_args = vec!["takeoff"];
        for arg in cmdline.split(" ") {
            if arg.starts_with("--takeoff-") {
                takeoff_args.push(arg);
            }
        }

        let args = TakeoffKernelArgs::try_parse_from(takeoff_args)?;
        Ok(args)
    }
}

async fn takeoff() {
    let guest_manager = Arc::new(GuestManager::new().unwrap());
    guest_manager.mark_boot_ready();

    mount("devtmpfs", "/dev", Some("devtmpfs")).await;
    mount("proc", "/proc", Some("proc")).await;
    mount("/dev/vda", "/real-root", Some("ext4")).await;

    let cmdline = fs::read_to_string("/proc/cmdline").await.unwrap();
    let args = TakeoffKernelArgs::try_parse_from_kernel_cmdline(&cmdline).unwrap();

    info!("takeoff is ready");

    if let Err(e) = chroot("/real-root") {
        error!("failed to chroot: {}", e);
        return;
    }

    if let Err(e) = chdir("/") {
        error!("failed to chdir: {}", e);
        return;
    }

    let config = fs::read_to_string("/etc/lttle/oci-config.json")
        .await
        .unwrap();
    let config: OciConfig = serde_json::from_str(&config).unwrap();
    info!("config: {:#?}", config);

    let mut cmd = vec![];
    cmd.extend(config.entrypoint.clone().unwrap_or_default());
    cmd.extend(config.cmd.clone().unwrap_or_default());
    info!("cmd: {:?}", cmd);

    let mut env_vars = if let Some(env) = config.env {
        env.iter()
            .map(|s| EnvVar::from_str(s))
            .collect::<Result<Vec<_>>>()
            .unwrap()
    } else {
        vec![]
    };

    env_vars.extend(
        args.envs
            .iter()
            .map(|s| EnvVar::from_str(s))
            .collect::<Result<Vec<_>>>()
            .unwrap(),
    );

    info!("env_vars: {:#?}", env_vars);

    let mut env_vars_hash = HashMap::new();
    for env_var in env_vars {
        env_vars_hash.insert(env_var.key, env_var.value);
    }

    let cmd = Command::new(cmd[0].clone())
        .args(&cmd[1..])
        .envs(env_vars_hash)
        .current_dir(config.working_dir.unwrap_or("/".to_string()))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();

    let mut child = match cmd {
        Ok(child) => child,
        Err(e) => {
            error!("failed to spawn command: {}", e);
            return;
        }
    };

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    // pipe stdout and stderr to serial (SerialWriter implements Write)

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

    stdout_thread.join().unwrap();
    stderr_thread.join().unwrap();

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

    async_runtime::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(takeoff());

    Ok(())
}
