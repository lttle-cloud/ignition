use std::time::Duration;

use ansi_term::{Color, Style};
use anyhow::Result;
use ignition::{
    constants::{DEFAULT_NAMESPACE, DEFAULT_SUSPEND_TIMEOUT_SECS},
    resources::{
        app::{AppLatest, AppStatus},
        machine::{MachineMode, MachineSnapshotStrategy},
        service::ServiceBindExternalProtocol,
    },
};
use meta::{summary, table};
use ordinal::Ordinal;

use crate::{
    client::get_api_client,
    cmd::{DeleteNamespacedArgs, GetNamespacedArgs, ListNamespacedArgs},
    config::Config,
    ui::message::{message_info, message_warn},
};

#[table]
pub struct AppTable {
    #[field(name = "name")]
    name: String,

    #[field(name = "namespace")]
    namespace: Option<String>,

    #[field(name = "mode", cell_style = important)]
    mode: String,

    #[field(name = "image")]
    image: Option<String>,

    #[field(name = "cpus")]
    cpu: String,

    #[field(name = "memory")]
    memory: String,

    #[field(name = "services")]
    services: String,
}

#[summary]
pub struct AppSummary {
    #[field(name = "name")]
    name: String,

    #[field(name = "namespace")]
    namespace: Option<String>,

    #[field(name = "tags")]
    tags: Vec<String>,

    #[field(name = "mode", cell_style = important)]
    mode: String,

    #[field(name = "snapshot strategy", cell_style = important)]
    snapshot_strategy: Option<String>,

    #[field(name = "suspend timeout")]
    suspend_timeout: Option<String>,

    #[field(name = "restart policy")]
    restart_policy: Option<String>,

    #[field(name = "image")]
    image: Option<String>,

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

    #[field(name = "services", clip_value = false)]
    services: Vec<String>,
}

impl From<(AppLatest, AppStatus)> for AppTableRow {
    fn from((app, _status): (AppLatest, AppStatus)) -> Self {
        let mode = match app.mode {
            None | Some(MachineMode::Regular) => "regular".to_string(),
            _ => "flash".to_string(),
        };

        let services = app.expose.unwrap_or_default().keys().len();

        Self {
            name: app.name,
            namespace: app.namespace,
            mode,
            image: app.image,
            cpu: app.resources.cpu.to_string(),
            memory: format!("{} MiB", app.resources.memory),
            services: services.to_string(),
        }
    }
}

impl From<(AppLatest, AppStatus)> for AppSummary {
    fn from((app, status): (AppLatest, AppStatus)) -> Self {
        let namespace = app
            .namespace
            .clone()
            .unwrap_or(DEFAULT_NAMESPACE.to_string());

        let env = app
            .environment
            .unwrap_or_default()
            .into_iter()
            .map(|(k, v)| format!("{k} = {v}"))
            .collect();

        let volumes: Vec<_> = app
            .volumes
            .unwrap_or_default()
            .into_iter()
            .map(|v| {
                let namespace = v
                    .namespace
                    .or_else(|| app.namespace.clone())
                    .unwrap_or(DEFAULT_NAMESPACE.to_string());

                format!("{}/{} → {}", namespace, v.name, v.path)
            })
            .collect();

        let mode = match app.mode {
            None | Some(MachineMode::Regular) => "regular".to_string(),
            _ => "flash".to_string(),
        };

        let (snapshot_strategy, timeout) = match app.mode {
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

        let depends_on = app
            .depends_on
            .unwrap_or_default()
            .into_iter()
            .map(|d| {
                let namespace = d
                    .namespace
                    .or_else(|| app.namespace.clone())
                    .unwrap_or(DEFAULT_NAMESPACE.to_string());
                format!("{}/{}", namespace, d.name)
            })
            .collect();

        let service_name_sytle = Style::new().fg(Color::Blue);

        let mut services = vec![];
        for (service_name, service) in app.expose.unwrap_or_default().iter() {
            match (service.internal.clone(), service.external.clone()) {
                (_, Some(external)) => {
                    let allocated = status.allocated_services.get(service_name);

                    let output_port = external.port.or(match external.protocol {
                        ServiceBindExternalProtocol::Http => None,
                        ServiceBindExternalProtocol::Https => None,
                        ServiceBindExternalProtocol::Tls => Some(service.port),
                        ServiceBindExternalProtocol::Tcp => None, // TCP uses dynamic allocation
                    });

                    let protocol = match external.protocol {
                        ServiceBindExternalProtocol::Http => "http",
                        ServiceBindExternalProtocol::Https => "https",
                        ServiceBindExternalProtocol::Tls => "tls",
                        ServiceBindExternalProtocol::Tcp => "tcp",
                    };

                    // Handle TCP services specially
                    if external.protocol == ServiceBindExternalProtocol::Tcp {
                        // For TCP services, show dynamic port allocation info
                        if allocated.is_some() {
                            services.push(format!(
                                "{}: tcp://*:<dynamic> → :{}  (see 'ignition service list' for allocated port)",
                                service_name_sytle.paint(service_name),
                                service.port
                            ));
                        } else {
                            services.push(format!(
                                "{}: tcp://*:<pending> → :{}  (port allocation pending)",
                                service_name_sytle.paint(service_name),
                                service.port
                            ));
                        }
                    } else {
                        // Handle HTTP/HTTPS/TLS services as before
                        let host = external.host.or(allocated.and_then(|a| a.domain.clone()));
                        if let Some(host) = host {
                            if let Some(output_port) = output_port {
                                services.push(format!(
                                    "{}: {}://{}:{} → :{}",
                                    service_name_sytle.paint(service_name),
                                    protocol,
                                    host,
                                    output_port,
                                    service.port
                                ));
                            } else {
                                services.push(format!(
                                    "{}: {}://{} → :{}",
                                    service_name_sytle.paint(service_name),
                                    protocol,
                                    host,
                                    service.port
                                ));
                            }
                        }
                    }
                }
                (Some(internal), _) => {
                    let output_port = internal.port.unwrap_or(service.port);
                    let internal_domain = format!(
                        "{}-{}.{}.svc.lttle.local",
                        app.name, service_name, namespace
                    );
                    services.push(format!(
                        "{}: tcp://{}:{} → :{}",
                        service_name_sytle.paint(service_name),
                        internal_domain,
                        output_port,
                        service.port
                    ));
                }
                (None, None) => {
                    let internal_domain = format!(
                        "{}-{}.{}.svc.lttle.local",
                        app.name, service_name, namespace
                    );
                    services.push(format!(
                        "{}: tcp://{}:{} → :{}",
                        service_name_sytle.paint(service_name),
                        internal_domain,
                        service.port,
                        service.port
                    ));
                }
            };
        }

        Self {
            name: app.name,
            namespace: app.namespace,
            tags: app.tags.unwrap_or_default(),
            mode,
            snapshot_strategy,
            restart_policy: app.restart_policy.map(|r| r.to_string()),
            image: app.image,
            cpu: app.resources.cpu.to_string(),
            memory: format!("{} MiB", app.resources.memory),
            env,
            cmd: app.command.clone().map(|c| c.join(" ")),
            volumes,
            depends_on,
            suspend_timeout: timeout,
            services,
        }
    }
}

pub async fn run_app_list(config: &Config, args: ListNamespacedArgs) -> Result<()> {
    let api_client = get_api_client(config.try_into()?);
    let apps = api_client.app().list(args.into()).await?;

    let mut table = AppTable::new();

    for (app, status) in apps {
        table.add_row(AppTableRow::from((app, status)));
    }

    table.print();

    Ok(())
}

pub async fn run_app_get(config: &Config, args: GetNamespacedArgs) -> Result<()> {
    let api_client = get_api_client(config.try_into()?);
    let (app, status) = api_client.app().get(args.clone().into(), args.name).await?;

    let summary = AppSummary::from((app, status));
    summary.print();

    Ok(())
}

pub async fn run_app_delete(config: &Config, args: DeleteNamespacedArgs) -> Result<()> {
    let api_client = get_api_client(config.try_into()?);
    if !args.confirm {
        message_warn(format!(
            "You are about to delete the app '{}'. This action cannot be undone. To confirm, run the command with --yes (or -y).",
            args.name
        ));
        return Ok(());
    }

    api_client
        .app()
        .delete(args.clone().into(), args.name.clone())
        .await?;

    message_info(format!("App '{}' has been deleted.", args.name));

    Ok(())
}
