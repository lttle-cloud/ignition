use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use chrono;
use clap::Args;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use ignition::{
    constants::{DEFAULT_NAMESPACE, DEFAULT_SUSPEND_TIMEOUT_SECS},
    resources::{
        core::{ExecParams, LogStreamParams, LogStreamTarget},
        machine::{
            MachineLatest, MachineMode, MachinePhase, MachineSnapshotStrategy, MachineStatus,
        },
        metadata::Namespace,
    },
};
use meta::{summary, table};
use ordinal::Ordinal;

use crate::{
    client::{MachineClientExt, get_api_client},
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

    /// Show elapsed time since log entry
    #[arg(long = "elapsed", short = 'e')]
    show_elapsed: bool,

    /// Follow the logs
    #[arg(long = "follow", short = 'f')]
    follow: bool,

    /// Name of the machine to fetch logs for
    name: String,
}

#[derive(Clone, Debug, Args)]
pub struct MachineExecArgs {
    /// Namespace of the machine (short: --ns)
    #[arg(long = "namespace", alias = "ns")]
    namespace: Option<String>,

    /// Name of the machine to fetch logs for
    name: String,

    /// Pass stdin to the container (interactive)
    #[arg(short = 'i', long = "stdin")]
    stdin: bool,

    /// Stdin is a TTY (allocate pseudo-terminal)
    #[arg(short = 't', long = "tty")]
    tty: bool,

    /// Command to execute
    #[arg(trailing_var_arg = true)]
    command: Vec<String>,
}

#[derive(Clone, Debug, Args)]
pub struct RestartNamespacedArgs {
    /// Namespace of the machine (short: --ns)
    #[arg(long = "namespace", alias = "ns")]
    namespace: Option<String>,

    /// Name of the machine to restart
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

    #[field(name = "tags")]
    tags: Vec<String>,

    #[field(name = "status", cell_style = important)]
    status: String,

    #[field(name = "mode", cell_style = important)]
    mode: String,

    #[field(name = "snapshot strategy", cell_style = important)]
    snapshot_strategy: Option<String>,

    #[field(name = "suspend timeout")]
    suspend_timeout: Option<String>,

    #[field(name = "restart policy")]
    restart_policy: Option<String>,

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

    #[field(name = "command")]
    cmd: Option<String>,

    #[field(name = "volumes")]
    volumes: Vec<String>,

    #[field(name = "dependencies")]
    depends_on: Vec<String>,

    #[field(name = "last boot time")]
    last_boot_time: Option<String>,

    #[field(name = "first boot time")]
    first_boot_time: Option<String>,

    #[field(name = "last exit code")]
    last_exit_code: Option<String>,

    #[field(name = "last restarting time")]
    last_restarting_time: Option<String>,

    #[field(name = "restart count")]
    restart_count: Option<String>,

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
            .environment
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

        let depends_on = machine
            .depends_on
            .unwrap_or_default()
            .into_iter()
            .map(|d| {
                let namespace = d
                    .namespace
                    .or_else(|| machine.namespace.clone())
                    .unwrap_or(DEFAULT_NAMESPACE.to_string());
                format!("{}/{}", namespace, d.name)
            })
            .collect();

        let last_restarting_time = status.last_restarting_time_us.map(|t| {
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64;

            let duration = (now_ms - t) / 1_000;
            let duration = Duration::from_secs(duration as u64);
            let duration = humantime::format_duration(duration);

            format!("{} ago", duration)
        });

        Self {
            name: machine.name,
            namespace: machine.namespace,
            tags: machine.tags.unwrap_or_default(),
            mode,
            snapshot_strategy,
            restart_policy: machine.restart_policy.map(|r| r.to_string()),
            internal_ip: status.machine_ip.clone(),
            status: status.phase.to_string(),
            image: status
                .image_resolved_reference
                .or(machine.image)
                .unwrap_or_default(),
            cpu: machine.resources.cpu.to_string(),
            memory: format!("{} MiB", machine.resources.memory),
            env,
            cmd: machine.command.clone().map(|c| c.join(" ")),
            volumes,
            depends_on,
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
            last_restarting_time,
            restart_count: status.restart_count.map(|c| c.to_string()),
            last_exit_code: status.last_exit_code.map(|c| c.to_string()),
        }
    }
}

impl From<(MachineLatest, MachineStatus)> for MachineTableRow {
    fn from((machine, status): (MachineLatest, MachineStatus)) -> Self {
        let mode = match machine.mode {
            None | Some(MachineMode::Regular) => "regular".to_string(),
            _ => "flash".to_string(),
        };

        let status_str = match (status.phase, status.last_exit_code) {
            (MachinePhase::Stopped, Some(code)) => format!("stopped (exit: {})", code),
            (MachinePhase::Error { message }, _) => format!("error ({})", message),
            (phase, _) => phase.to_string(),
        };

        Self {
            name: machine.name,
            namespace: machine.namespace,
            mode,
            status: status_str,
            image: status
                .image_resolved_reference
                .or(machine.image)
                .unwrap_or_default(),
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
    let api_client = get_api_client(config.try_into()?);
    let machines = api_client.machine().list(args.into()).await?;

    let mut table = MachineTable::new();

    for (machine, status) in machines {
        table.add_row(MachineTableRow::from((machine, status)));
    }

    table.print();

    Ok(())
}

pub async fn run_machine_get(config: &Config, args: GetNamespacedArgs) -> Result<()> {
    let api_client = get_api_client(config.try_into()?);
    let (machine, status) = api_client
        .machine()
        .get(args.clone().into(), args.name)
        .await?;

    let summary = MachineSummary::from((machine, status));
    summary.print();

    Ok(())
}

pub async fn run_machine_get_logs(config: &Config, args: MachineLogsArgs) -> Result<()> {
    let api_client = get_api_client(config.try_into()?);

    let namespace = Namespace::from_value_or_default(args.namespace);

    if args.follow && args.since.is_some() {
        message_warn("Cannot use --follow and --since together");
        return Ok(());
    }

    if args.follow && args.show_elapsed {
        message_warn("Cannot use --follow and --elapsed together");
        return Ok(());
    }

    let now_ns = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos() as u64;

    let since = args.since.unwrap_or("1d".to_string());
    let since = humantime::parse_duration(&since)?;
    let since_ns = since.as_nanos() as u64;

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

pub async fn run_machine_exec(config: &Config, args: MachineExecArgs) -> Result<()> {
    let cmd = args.command.join(" ");
    let stdin_enabled = args.stdin;
    let tty_mode = args.tty;

    let api_client = get_api_client(config.try_into()?);
    let ws_stream = api_client
        .core()
        .exec(
            Namespace::from_value_or_default(args.namespace),
            ExecParams {
                machine_name: args.name,
                command: cmd,
                stdin: if stdin_enabled { Some(true) } else { None },
                tty: if tty_mode { Some(true) } else { None },
            },
        )
        .await?;

    // Split the WebSocket stream for bidirectional communication
    use futures_util::stream::StreamExt;
    let (mut ws_write, mut ws_read) = ws_stream.split();

    // Enable raw mode only if TTY mode is requested (-t flag)
    let _guard = if tty_mode {
        enable_raw_mode()?;
        Some(scopeguard::guard((), |_| {
            let _ = disable_raw_mode();
        }))
    } else {
        None
    };

    // Handle bidirectional data flow
    let tty_mode_for_input = tty_mode;
    let mut stdin_handle = if stdin_enabled {
        tokio::spawn(async move {
            if tty_mode_for_input {
                // TTY mode: character-by-character input with raw terminal events
                loop {
                    if let Ok(true) = event::poll(Duration::from_millis(100)) {
                        if let Ok(event) = event::read() {
                            match event {
                                Event::Key(KeyEvent {
                                    code, modifiers, ..
                                }) => {
                                    let bytes = match code {
                                        KeyCode::Char('c')
                                            if modifiers.contains(
                                                crossterm::event::KeyModifiers::CONTROL,
                                            ) =>
                                        {
                                            b"\x03".to_vec()
                                        }
                                        KeyCode::Char('d')
                                            if modifiers.contains(
                                                crossterm::event::KeyModifiers::CONTROL,
                                            ) =>
                                        {
                                            b"\x04".to_vec()
                                        }
                                        KeyCode::Char(c) => c.to_string().into_bytes(),
                                        KeyCode::Enter => b"\r".to_vec(),
                                        KeyCode::Backspace => b"\x08".to_vec(),
                                        KeyCode::Tab => b"\t".to_vec(),
                                        KeyCode::Esc => b"\x1b".to_vec(),
                                        KeyCode::Delete => b"\x7f".to_vec(),
                                        KeyCode::Up => b"\x1b[A".to_vec(),
                                        KeyCode::Down => b"\x1b[B".to_vec(),
                                        KeyCode::Right => b"\x1b[C".to_vec(),
                                        KeyCode::Left => b"\x1b[D".to_vec(),
                                        KeyCode::F(n) => format!("\x1b[{};2~", n + 10).into_bytes(),
                                        _ => continue,
                                    };

                                    use futures_util::SinkExt;
                                    use tungstenite::Message;

                                    if ws_write.send(Message::Binary(bytes.into())).await.is_err() {
                                        break;
                                    }
                                }
                                Event::Resize(_width, _height) => {
                                    // Terminal resize events - could be handled via WebSocket protocol
                                    // but for now we'll ignore them to avoid TTY issues
                                }
                                _ => {}
                            }
                        }
                    }
                }
            } else {
                // Non-TTY mode: line-buffered input (like regular shell pipe)
                use tokio::io::{AsyncBufReadExt, BufReader, stdin};
                let mut lines = BufReader::new(stdin()).lines();

                while let Ok(Some(line)) = lines.next_line().await {
                    use futures_util::SinkExt;
                    use tungstenite::Message;

                    let mut line_with_newline = line;
                    line_with_newline.push('\n');

                    if ws_write
                        .send(Message::Binary(line_with_newline.into_bytes().into()))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }
        })
    } else {
        // No stdin - just create a dummy task that does nothing
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(u64::MAX)).await;
        })
    };

    let tty_mode_for_output = tty_mode;
    let mut stdout_handle = tokio::spawn(async move {
        use futures_util::StreamExt;

        while let Some(msg) = ws_read.next().await {
            match msg {
                Ok(tungstenite::Message::Binary(data)) => {
                    if tty_mode_for_output {
                        // TTY mode: output raw bytes directly to preserve terminal control sequences
                        use crossterm::{queue, style::Print};
                        use std::io::{self, Write};

                        if let Ok(text) = String::from_utf8(data.to_vec()) {
                            // If it's valid UTF-8, use crossterm
                            if queue!(io::stdout(), Print(text)).is_err() {
                                break;
                            }
                        } else {
                            // If it's raw bytes, write directly
                            if io::stdout().write_all(&data).is_err() {
                                break;
                            }
                        }

                        if io::stdout().flush().is_err() {
                            break;
                        }
                    } else {
                        // Non-TTY mode: convert to string and fix line endings
                        if let Ok(mut text) = String::from_utf8(data.to_vec()) {
                            text = text.replace("\r\n", "\n").replace('\n', "\r\n");
                            print!("{}", text);
                            use std::io::{self, Write};
                            if io::stdout().flush().is_err() {
                                break;
                            }
                        }
                    }
                }
                Ok(tungstenite::Message::Text(text)) => {
                    if tty_mode_for_output {
                        // TTY mode: output text directly to preserve terminal behavior
                        use crossterm::{queue, style::Print};
                        use std::io::{self, Write};

                        if queue!(io::stdout(), Print(&text)).is_err() {
                            break;
                        }
                        if io::stdout().flush().is_err() {
                            break;
                        }
                    } else {
                        // Non-TTY mode: fix line endings for proper display
                        let fixed_text = text.replace("\r\n", "\n").replace('\n', "\r\n");
                        print!("{}", fixed_text);
                        use std::io::{self, Write};
                        if io::stdout().flush().is_err() {
                            break;
                        }
                    }
                }
                Ok(tungstenite::Message::Close(_)) => {
                    // WebSocket closed, command finished
                    break;
                }
                Err(_) => break,
                _ => continue,
            }
        }
    });

    // Wait for either task to complete, then abort the other
    tokio::select! {
        _ = &mut stdout_handle => {
            stdin_handle.abort();
        },
        _ = &mut stdin_handle => {
            stdout_handle.abort();
        },
    }

    let _ = disable_raw_mode();
    std::process::exit(0);
}

pub async fn run_machine_delete(config: &Config, args: DeleteNamespacedArgs) -> Result<()> {
    let api_client = get_api_client(config.try_into()?);
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

pub async fn run_machine_restart(config: &Config, args: RestartNamespacedArgs) -> Result<()> {
    let api_client = get_api_client(config.try_into()?);

    let namespace = Namespace::from_value_or_default(args.namespace);

    api_client
        .machine()
        .add_tag(
            namespace,
            args.name.clone(),
            "ignitiond.restart".to_string(),
        )
        .await?;

    Ok(())
}
