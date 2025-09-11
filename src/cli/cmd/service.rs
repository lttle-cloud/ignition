use anyhow::Result;
use ignition::{
    constants::DEFAULT_TRAFFIC_AWARE_INACTIVITY_TIMEOUT_SECS,
    resources::{
        metadata::Namespace,
        service::{ServiceBind, ServiceLatest, ServiceStatus, ServiceTargetConnectionTracking},
    },
};
use meta::{summary, table};

use crate::{
    client::get_api_client,
    cmd::{DeleteNamespacedArgs, GetNamespacedArgs, ListNamespacedArgs},
    config::Config,
    ui::message::{message_info, message_warn},
};

#[table]
pub struct ServiceTable {
    #[field(name = "name")]
    name: String,

    #[field(name = "namespace")]
    namespace: Option<String>,

    #[field(name = "mode", cell_style = important)]
    mode: String,

    #[field(name = "host", cell_style = important)]
    host: Option<String>,

    #[field(name = "target")]
    target: String,

    #[field(name = "route")]
    route: String,
}

#[summary]
pub struct ServiceSummary {
    #[field(name = "name")]
    name: String,

    #[field(name = "namespace")]
    namespace: Option<String>,

    #[field(name = "tags")]
    tags: Vec<String>,

    #[field(name = "mode", cell_style = important)]
    mode: String,

    #[field(name = "target", cell_style = important)]
    target: String,

    #[field(name = "target port")]
    target_port: String,

    #[field(name = "host", cell_style = important)]
    host: Option<String>,

    #[field(name = "service ip")]
    ip: Option<String>,

    #[field(name = "route")]
    route: String,

    #[field(name = "connection tracking")]
    connection_tracking: String,
}

impl From<(ServiceLatest, ServiceStatus)> for ServiceTableRow {
    fn from((service, status): (ServiceLatest, ServiceStatus)) -> Self {
        let host = match &service.bind {
            ServiceBind::External { host, .. } => Some(host.clone()),
            ServiceBind::Internal { .. } => status
                .internal_dns_hostname
                .clone()
                .or(status.service_ip.clone()),
        };

        let target_namespace = service
            .target
            .namespace
            .clone()
            .or(service.namespace.clone());
        let target_namespace = Namespace::from_value_or_default(target_namespace)
            .as_value()
            .unwrap_or_default();

        let target = format!("{}/{}", target_namespace, service.target.name);

        let route = match &service.bind {
            ServiceBind::Internal { port } => format!(
                ":{} ({}) → :{} ({})",
                port.unwrap_or(service.target.port),
                service.target.protocol.to_string(),
                service.target.port,
                service.target.protocol.to_string()
            ),
            ServiceBind::External { port, protocol, .. } => {
                let port = port.unwrap_or(protocol.default_port(&service.target));
                format!(
                    ":{} ({}) → :{} ({})",
                    port,
                    protocol.to_string(),
                    service.target.port,
                    service.target.protocol.to_string()
                )
            }
        };

        Self {
            name: service.name,
            namespace: service.namespace,
            mode: service.bind.to_string(),
            target,
            host,
            route,
        }
    }
}

impl From<(ServiceLatest, ServiceStatus)> for ServiceSummary {
    fn from((service, status): (ServiceLatest, ServiceStatus)) -> Self {
        let host = match &service.bind {
            ServiceBind::External { host, .. } => Some(host.clone()),
            ServiceBind::Internal { .. } => status
                .internal_dns_hostname
                .clone()
                .or(status.service_ip.clone()),
        };

        let target_namespace = service
            .target
            .namespace
            .clone()
            .or(service.namespace.clone());
        let target_namespace = Namespace::from_value_or_default(target_namespace)
            .as_value()
            .unwrap_or_default();

        let target = format!("{}/{}", target_namespace, service.target.name);

        let route = match &service.bind {
            ServiceBind::Internal { port } => format!(
                ":{} ({}) → :{} ({})",
                port.unwrap_or(service.target.port),
                service.target.protocol.to_string(),
                service.target.port,
                service.target.protocol.to_string()
            ),
            ServiceBind::External { port, protocol, .. } => {
                let port = port.unwrap_or(protocol.default_port(&service.target));
                format!(
                    ":{} ({}) → :{} ({})",
                    port,
                    protocol.to_string(),
                    service.target.port,
                    service.target.protocol.to_string()
                )
            }
        };

        let connection_tracking = match &service.target.connection_tracking {
            Some(ServiceTargetConnectionTracking::TrafficAware { inactivity_timeout }) => {
                format!(
                    "traffic aware (inactivity timeout: {}s)",
                    inactivity_timeout.unwrap_or(DEFAULT_TRAFFIC_AWARE_INACTIVITY_TIMEOUT_SECS)
                )
            }
            _ => "connection aware".to_string(),
        };

        Self {
            name: service.name,
            namespace: service.namespace,
            tags: service.tags.clone().unwrap_or_default(),
            target,
            target_port: service.target.port.to_string(),
            host,
            ip: status.service_ip.clone(),
            mode: service.bind.to_string(),
            route,
            connection_tracking,
        }
    }
}

pub async fn run_service_list(config: &Config, args: ListNamespacedArgs) -> Result<()> {
    let api_client = get_api_client(config.try_into()?);
    let services = api_client.service().list(args.into()).await?;

    let mut table = ServiceTable::new();

    for (service, status) in services {
        table.add_row(ServiceTableRow::from((service, status)));
    }

    table.print();

    Ok(())
}

pub async fn run_service_get(config: &Config, args: GetNamespacedArgs) -> Result<()> {
    let api_client = get_api_client(config.try_into()?);
    let (service, status) = api_client
        .service()
        .get(args.clone().into(), args.name)
        .await?;

    let summary = ServiceSummary::from((service, status));
    summary.print();

    Ok(())
}

pub async fn run_service_delete(config: &Config, args: DeleteNamespacedArgs) -> Result<()> {
    let api_client = get_api_client(config.try_into()?);
    if !args.confirm {
        message_warn(format!(
            "You are about to delete the service '{}'. This action cannot be undone. To confirm, run the command with --yes (or -y).",
            args.name
        ));
        return Ok(());
    }

    api_client
        .service()
        .delete(args.clone().into(), args.name.clone())
        .await?;

    message_info(format!("Service '{}' has been deleted.", args.name));

    Ok(())
}
