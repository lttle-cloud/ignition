use anyhow::Result;
use meta::resource;

use crate::resources::{Convert, FromResource, ProvideMetadata};

#[resource(name = "Service", tag = "service")]
mod service {

    #[version(stored + served + latest)]
    struct V1 {
        target: ServiceTarget,
        bind: ServiceBind,
    }

    #[schema]
    struct ServiceTarget {
        #[serde(deserialize_with = "super::de_trim_non_empty_string")]
        name: String,
        #[serde(default, deserialize_with = "super::de_opt_trim_non_empty_string")]
        namespace: Option<String>,
        port: u16,
        protocol: ServiceTargetProtocol,
        #[serde(rename = "connection-tracking")]
        connection_tracking: Option<ServiceTargetConnectionTracking>,
    }

    #[schema]
    enum ServiceTargetProtocol {
        #[serde(rename = "http")]
        Http,
        #[serde(rename = "tcp")]
        Tcp,
    }

    #[schema]
    enum ServiceTargetConnectionTracking {
        #[serde(rename = "connection-aware")]
        ConnectionAware,
        #[serde(rename = "traffic-aware")]
        TrafficAware {
            #[serde(rename = "inactivity-timeout")]
            inactivity_timeout: Option<u64>,
        },
    }

    #[schema]
    enum ServiceBind {
        #[serde(rename = "internal")]
        Internal {
            /// If not provided, the port will be inferred from target port.
            port: Option<u16>,
        },
        #[serde(rename = "external")]
        External {
            #[serde(deserialize_with = "super::de_trim_non_empty_string")]
            host: String,
            /// If not provided, the port will be inferred from protocol or target port.
            port: Option<u16>,
            protocol: ServiceBindExternalProtocol,
        },
    }

    #[schema]
    enum ServiceBindExternalProtocol {
        #[serde(rename = "http")]
        Http,
        #[serde(rename = "https")]
        Https,
        #[serde(rename = "tls")]
        Tls,
    }

    #[status]
    struct Status {
        service_ip: Option<String>,
        internal_dns_hostname: Option<String>,
    }
}

impl FromResource<Service> for ServiceStatus {
    fn from_resource(_resource: Service) -> Result<Self> {
        Ok(ServiceStatus {
            service_ip: None,
            internal_dns_hostname: None,
        })
    }
}

impl ServiceBindExternalProtocol {
    pub fn default_port(&self, target: &ServiceTarget) -> u16 {
        match self {
            ServiceBindExternalProtocol::Http => 80,
            ServiceBindExternalProtocol::Https => 443,
            ServiceBindExternalProtocol::Tls => target.port,
        }
    }
}

impl ToString for ServiceBindExternalProtocol {
    fn to_string(&self) -> String {
        match self {
            ServiceBindExternalProtocol::Http => "http".to_string(),
            ServiceBindExternalProtocol::Https => "https".to_string(),
            ServiceBindExternalProtocol::Tls => "tls".to_string(),
        }
    }
}

impl ToString for ServiceTargetProtocol {
    fn to_string(&self) -> String {
        match self {
            ServiceTargetProtocol::Http => "http".to_string(),
            ServiceTargetProtocol::Tcp => "tcp".to_string(),
        }
    }
}

impl ToString for ServiceBind {
    fn to_string(&self) -> String {
        match self {
            ServiceBind::Internal { .. } => "internal".to_string(),
            ServiceBind::External { .. } => "external".to_string(),
        }
    }
}

impl Service {
    pub fn hash_with_updated_metadata(&self) -> u64 {
        use std::hash::{DefaultHasher, Hash, Hasher};

        let metadata = self.metadata();
        let mut service = self.stored();
        service.namespace = metadata.namespace;
        let service: Service = service.into();

        let mut hasher = DefaultHasher::new();
        service.hash(&mut hasher);
        hasher.finish()
    }
}
