use crate::{client::get_client, config::Config};
use comfy_table::Table;
use ignition_client::ignition_proto::{
    service::{service_mode::Mode, service_protocol::Protocol, External, Tcp, Tls},
    util::Empty,
};
use util::result::Result;

pub async fn run_service_list(config: Config) -> Result<()> {
    let client = get_client(config).await?;

    let services = client.service().list(Empty {}).await?.into_inner();

    let mut table = Table::new();
    table.set_header(vec!["Name", "Protocol", "Mode", "Target"]);

    for service in services.services {
        let Some(target) = service.target else {
            continue;
        };

        let Some(mode) = service.mode.and_then(|m| m.mode) else {
            continue;
        };

        let Some(protocol) = service.protocol.and_then(|p| p.protocol) else {
            continue;
        };

        let target = format!("{} @ {}", target.name, target.port);
        let mode = match mode {
            Mode::Internal(_) => match service.internal_ip {
                Some(ip) => format!("internal ({})", ip),
                None => "internal".to_string(),
            },
            Mode::External(External { host }) => format!("external ({})", host),
        };

        let protocol = match protocol {
            Protocol::Http(_) => "http".to_string(),
            Protocol::Tcp(Tcp { port }) => format!("tcp ({})", port),
            Protocol::Tls(Tls { port }) => format!("tls ({})", port),
        };

        table.add_row(vec![service.name, protocol, mode, target]);
    }

    println!("{}", table);

    Ok(())
}
