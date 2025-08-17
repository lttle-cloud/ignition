use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use chrono;
use clap::Args;
use ignition::{
    constants::{DEFAULT_NAMESPACE, DEFAULT_SUSPEND_TIMEOUT_SECS},
    resources::{
        core::{LogStreamParams, LogStreamTarget},
        machine::{MachineLatest, MachineMode, MachineSnapshotStrategy, MachineStatus},
        metadata::Namespace,
    },
};
use meta::{summary, table};
use ordinal::Ordinal;

use crate::{
    client::get_api_client,
    cmd::{DeleteNamespacedArgs, GetNamespacedArgs, ListNamespacedArgs},
    config::Config,
    ui::message::{message_info, message_log_stderr, message_log_stdout, message_warn},
};

#[derive(Clone, Debug, Args)]
pub struct MachineLogsArgs {
    /// Namespace of the machine (short: --ns)
    #[arg(long = "namespace", alias = "ns")]
    namespace: Option<String>,

    /// Since when to fetch logs [default: 1d] (eg. 1d, 1h, 1m, 10s)
    #[arg(long = "since", short = 's')]
    since: Option<String>,

    /// Show timestamps (always in UTC)
    #[arg(long = "timestamps", short = 't')]
    show_timestamps: bool,

    #[arg(long = "elapsed", short = 'e')]
    show_elapsed: bool,

    /// Follow the logs
    #[arg(long = "follow", short = 'f')]
    follow: bool,

    /// Name of the machine to fetch logs for
    name: String,
}

#[table]
pub struct MachineTable {
    #[field(name = "name")]
    name: String,

    #[field(name = "namespace")]
    namespace: Option<String>,

    #[field(name = "mode", cell_style = important)]
    mode: String,

    #[field(name = "status", cell_style = important)]
    status: String,

    #[field(name = "image", max_width = 50)]
    image: String,

    #[field(name = "cpus")]
    cpu: String,

    #[field(name = "memory")]
    memory: String,

    #[field(name = "last boot time")]
    last_boot_time: Option<String>,
}

#[summary]
pub struct MachineSummary {
    #[field(name = "name")]
    name: String,

    #[field(name = "namespace")]
    namespace: Option<String>,

    #[field(name = "status", cell_style = important)]
    status: String,

    #[field(name = "mode", cell_style = important)]
    mode: String,

    #[field(name = "snapshot strategy", cell_style = important)]
    snapshot_strategy: Option<String>,

    #[field(name = "suspend timeout")]
    suspend_timeout: Option<String>,

    #[field(name = "internal ip")]
    internal_ip: Option<String>,

    #[field(name = "image")]
    image: String,

    #[field(name = "cpus")]
    cpu: String,

    #[field(name = "memory")]
    memory: String,

    #[field(name = "environment")]
    env: Vec<String>,

    #[field(name = "volumes")]
    volumes: Vec<String>,

    #[field(name = "last boot time")]
    last_boot_time: Option<String>,

    #[field(name = "first boot time")]
    first_boot_time: Option<String>,

    #[field(name = "machine id (internal)")]
    hypervisor_machine_id: Option<String>,

    #[field(name = "root volume id (internal)")]
    hypervisor_root_volume_id: Option<String>,

    #[field(name = "tap device (internal)")]
    hypervisor_tap_device: Option<String>,
}

impl From<(MachineLatest, MachineStatus)> for MachineSummary {
    fn from((machine, status): (MachineLatest, MachineStatus)) -> Self {
        let env = machine
            .env
            .unwrap_or_default()
            .into_iter()
            .map(|(k, v)| format!("{k} = {v}"))
            .collect();

        let volumes: Vec<_> = machine
            .volumes
            .unwrap_or_default()
            .into_iter()
            .map(|v| {
                let namespace = v
                    .namespace
                    .or_else(|| machine.namespace.clone())
                    .unwrap_or(DEFAULT_NAMESPACE.to_string());

                format!("{}/{} → {}", namespace, v.name, v.path)
            })
            .collect();

        let mode = match machine.mode {
            None | Some(MachineMode::Regular) => "regular".to_string(),
            _ => "flash".to_string(),
        };

        let (snapshot_strategy, timeout) = match machine.mode {
            Some(MachineMode::Flash {
                strategy: MachineSnapshotStrategy::Manual,
                timeout,
            }) => (
                Some("manual".to_string()),
                Some(timeout.unwrap_or(DEFAULT_SUSPEND_TIMEOUT_SECS)),
            ),
            Some(MachineMode::Flash {
                strategy: MachineSnapshotStrategy::WaitForUserSpaceReady,
                timeout,
            }) => (
                Some("user-space ready".to_string()),
                Some(timeout.unwrap_or(DEFAULT_SUSPEND_TIMEOUT_SECS)),
            ),
            Some(MachineMode::Flash {
                strategy: MachineSnapshotStrategy::WaitForFirstListen,
                timeout,
            }) => (
                Some("first listen".to_string()),
                Some(timeout.unwrap_or(DEFAULT_SUSPEND_TIMEOUT_SECS)),
            ),
            Some(MachineMode::Flash {
                strategy: MachineSnapshotStrategy::WaitForNthListen(n),
                timeout,
            }) => (
                Some(format!("{} listen", Ordinal(n))),
                Some(timeout.unwrap_or(DEFAULT_SUSPEND_TIMEOUT_SECS)),
            ),
            Some(MachineMode::Flash {
                strategy: MachineSnapshotStrategy::WaitForListenOnPort(port),
                timeout,
            }) => (
                Some(format!("listen on port {port}")),
                Some(timeout.unwrap_or(DEFAULT_SUSPEND_TIMEOUT_SECS)),
            ),
            _ => (None, None),
        };

        let timeout = timeout.map(|t| {
            let duration = Duration::from_secs(t);
            let duration = humantime::format_duration(duration);
            duration.to_string()
        });

        Self {
            name: machine.name,
            namespace: machine.namespace,
            mode,
            snapshot_strategy,
            internal_ip: status.machine_ip.clone(),
            status: status.phase.to_string(),
            image: status.image_resolved_reference.unwrap_or(machine.image),
            cpu: machine.resources.cpu.to_string(),
            memory: format!("{} MiB", machine.resources.memory),
            env,
            volumes,
            suspend_timeout: timeout,
            hypervisor_machine_id: status.machine_id.clone(),
            hypervisor_root_volume_id: status.machine_image_volume_id.clone(),
            hypervisor_tap_device: status.machine_tap.clone(),
            first_boot_time: status.first_boot_time_us.map(|t| {
                let duration = Duration::from_micros(t as u64);
                let duration = humantime::format_duration(duration);
                duration.to_string()
            }),
            last_boot_time: status.last_boot_time_us.map(|t| {
                let duration = Duration::from_micros(t as u64);
                let duration = humantime::format_duration(duration);
                duration.to_string()
            }),
        }
    }
}

impl From<(MachineLatest, MachineStatus)> for MachineTableRow {
    fn from((machine, status): (MachineLatest, MachineStatus)) -> Self {
        let mode = match machine.mode {
            None | Some(MachineMode::Regular) => "regular".to_string(),
            _ => "flash".to_string(),
        };

        Self {
            name: machine.name,
            namespace: machine.namespace,
            mode,
            status: status.phase.to_string(),
            image: status.image_resolved_reference.unwrap_or(machine.image),
            cpu: machine.resources.cpu.to_string(),
            memory: format!("{} MiB", machine.resources.memory),
            last_boot_time: status.last_boot_time_us.map(|t| {
                let duration = Duration::from_micros(t as u64);
                let duration = humantime::format_duration(duration);
                duration.to_string()
            }),
        }
    }
}

pub async fn run_machine_list(config: &Config, args: ListNamespacedArgs) -> Result<()> {
    let api_client = get_api_client(config).await?;
    let machines = api_client.machine().list(args.into()).await?;

    let mut table = MachineTable::new();

    for (machine, status) in machines {
        table.add_row(MachineTableRow::from((machine, status)));
    }

    table.print();

    Ok(())
}

pub async fn run_machine_get(config: &Config, args: GetNamespacedArgs) -> Result<()> {
    let api_client = get_api_client(config).await?;
    let (machine, status) = api_client
        .machine()
        .get(args.clone().into(), args.name)
        .await?;

    let summary = MachineSummary::from((machine, status));
    summary.print();

    Ok(())
}

pub async fn run_machine_get_logs(config: &Config, args: MachineLogsArgs) -> Result<()> {
    let api_client = get_api_client(config).await?;

    let namespace = Namespace::from_value_or_default(args.namespace);

    if args.follow && args.since.is_some() {
        message_warn("Cannot use --follow and --since together");
        return Ok(());
    }

    if args.follow && args.show_elapsed {
        message_warn("Cannot use --follow and --elapsed together");
        return Ok(());
    }

    let now_ns = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();

    let since = args.since.unwrap_or("1d".to_string());
    let since = humantime::parse_duration(&since)?;
    let since_ns = since.as_nanos();

    let start_ts = if args.follow {
        None
    } else {
        Some((now_ns - since_ns).to_string())
    };

    let end_ts = if args.follow {
        None
    } else {
        Some(now_ns.to_string())
    };

    let mut stream = api_client
        .core()
        .stream_logs(
            namespace,
            LogStreamParams::Machine {
                machine_name: args.name,
                start_ts_ns: start_ts,
                end_ts_ns: end_ts,
            },
        )
        .await?;

    while let Some(result) = stream.next().await {
        let timestamp = if args.show_timestamps {
            let secs = result.timestamp / 1_000_000_000;
            let nanos = result.timestamp % 1_000_000_000;

            let dt =
                chrono::DateTime::from_timestamp(secs as i64, nanos as u32).unwrap_or_default();

            Some(dt.format("%Y-%m-%d %H:%M:%S%.3f").to_string())
        } else if args.show_elapsed {
            let duration = Duration::from_secs((now_ns - result.timestamp) as u64 / 1_000_000_000);
            let duration = humantime::format_duration(duration);
            Some(format!("{} ago", duration))
        } else {
            None
        };

        match result.target_stream {
            LogStreamTarget::Stdout => message_log_stdout(&result.message, timestamp),
            LogStreamTarget::Stderr => message_log_stderr(&result.message, timestamp),
        }
    }

    Ok(())
}

pub async fn run_machine_delete(config: &Config, args: DeleteNamespacedArgs) -> Result<()> {
    let api_client = get_api_client(config).await?;
    if !args.confirm {
        message_warn(format!(
            "You are about to delete the machine '{}'. This action cannot be undone. To confirm, run the command with --yes (or -y).",
            args.name
        ));
        return Ok(());
    }

    api_client
        .machine()
        .delete(args.clone().into(), args.name.clone())
        .await?;

    message_info(format!("Machine '{}' has been deleted.", args.name));

    Ok(())
}
