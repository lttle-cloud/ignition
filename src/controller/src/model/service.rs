use util::encoding::codec;

#[codec]
#[derive(Debug, Clone)]
pub enum ServiceProtocol {
    Tcp { port: u16 },
    Tls { port: u16 },
    Http,
}

#[codec]
#[derive(Debug, Clone, PartialEq)]
pub enum ServiceMode {
    Internal,
    External { host: String },
}

#[codec]
#[derive(Debug, Clone)]
pub struct ServiceTarget {
    pub name: String,
    pub port: u16,
}

#[codec]
#[derive(Debug, Clone)]
pub struct Service {
    pub name: String,
    pub target: ServiceTarget,
    pub protocol: ServiceProtocol,
    pub mode: ServiceMode,
    pub internal_ip: Option<String>,
}
