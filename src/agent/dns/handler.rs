use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use hickory_proto::{
    op::{MessageType, OpCode, ResponseCode},
    rr::{RData, Record, RecordType, rdata::A},
};
use hickory_resolver::{
    TokioAsyncResolver,
    config::{NameServerConfig, Protocol, ResolverConfig, ResolverOpts},
};
use hickory_server::{
    authority::MessageResponseBuilder,
    server::{Request, RequestHandler, ResponseHandler, ResponseInfo},
};
use tracing::{debug, warn};

use crate::resources::metadata::{Metadata, Namespace};

use super::DnsHandler;

#[derive(Debug, Clone, PartialEq, Eq)]
enum DnsSubdomain {
    Service { name: String, namespace: String },
}

impl DnsHandler {
    fn parse_subdomain(&self, address: &str) -> Option<DnsSubdomain> {
        let parts: Vec<&str> = address.trim_end_matches('.').split('.').collect();

        if parts.len() != 5 {
            return None;
        }

        // Check if the address ends with our zone suffix
        let expected_suffix = format!(".svc.{}", self.zone_suffix);
        if !address.ends_with(&expected_suffix) {
            return None;
        }

        let name = parts[0];
        let namespace = parts[1];
        let subdomain_type = parts[2];

        match subdomain_type {
            "svc" => Some(DnsSubdomain::Service {
                name: name.to_string(),
                namespace: namespace.to_string(),
            }),
            _ => None,
        }
    }

    pub(super) fn create_upstream_resolver(
        upstream_dns_servers: &[String],
    ) -> Option<TokioAsyncResolver> {
        if upstream_dns_servers.is_empty() {
            return None;
        }

        let mut resolver_config = ResolverConfig::new();

        for server in upstream_dns_servers {
            if let Ok(addr) = server.parse::<SocketAddr>() {
                resolver_config.add_name_server(NameServerConfig {
                    socket_addr: addr,
                    protocol: Protocol::Udp,
                    tls_dns_name: None,
                    trust_negative_responses: true,
                    bind_addr: None,
                });
            }
        }

        let mut opts = ResolverOpts::default();
        opts.timeout = Duration::from_secs(2);
        opts.attempts = 2;

        Some(TokioAsyncResolver::tokio(resolver_config, opts))
    }

    async fn resolve_service(&self, name: &str, namespace: &str, tenant: &str) -> Option<String> {
        // Look up service in repository
        let service_repo = self.repository.service(tenant.to_string());

        let metadata = Metadata::new(name.to_string(), Namespace::specified(namespace));

        if let Ok(Some((_, status))) = service_repo.get_with_status(metadata) {
            return status.service_ip;
        }

        None
    }

    async fn handle_query(&self, request: &Request) -> Vec<Record> {
        let query = request.query();
        let name = query.name();
        let record_type = query.query_type();

        // Get source IP to determine tenant
        let src_ip = match request.src() {
            std::net::SocketAddr::V4(addr) => addr.ip().to_string(),
            std::net::SocketAddr::V6(addr) => {
                debug!("IPv6 source address not supported: {}", addr);
                return vec![];
            }
        };

        // Find tenant from source IP
        let tenant = self
            .net_agent
            .ip_reservation_lookup(&src_ip)
            .ok()
            .flatten()
            .map(|t| t.tenant.clone());

        if let Some(ref t) = tenant {
            debug!("DNS query from tenant {}: {} {:?}", t, name, record_type);
        } else {
            debug!(
                "DNS query from unknown source {}: {} {:?}",
                src_ip, name, record_type
            );
        }

        // Only handle A record queries for now
        if record_type != RecordType::A {
            return vec![];
        }

        // Parse the query name
        let name_str = name.to_string();

        // First check if this is an internal service query
        if let Some(ref t) = tenant {
            if let Some(subdomain) = self.parse_subdomain(&name_str) {
                match subdomain {
                    DnsSubdomain::Service {
                        name: resource_name,
                        namespace,
                    } => {
                        // Service query: <service>.<namespace>.svc.<zone_suffix>
                        if let Some(service_ip) =
                            self.resolve_service(&resource_name, &namespace, t).await
                        {
                            if let Ok(ip) = service_ip.parse::<Ipv4Addr>() {
                                return vec![Record::from_rdata(
                                    name.clone().into(),
                                    self.default_ttl,
                                    RData::A(A(ip)),
                                )];
                            }
                        }
                    }
                }
            }
        }

        // If not an internal service, try upstream DNS resolution
        if let Some(ref resolver) = self.upstream_resolver {
            debug!("Forwarding query to upstream DNS: {}", name_str);

            match resolver.lookup_ip(name_str.trim_end_matches('.')).await {
                Ok(lookup) => {
                    let records: Vec<Record> = lookup
                        .iter()
                        .filter_map(|ip| match ip {
                            std::net::IpAddr::V4(ipv4) => Some(Record::from_rdata(
                                name.clone().into(),
                                self.default_ttl,
                                RData::A(A(ipv4)),
                            )),
                            std::net::IpAddr::V6(_) => None, // Skip IPv6 for now
                        })
                        .collect();

                    if !records.is_empty() {
                        debug!("Upstream DNS returned {} records", records.len());
                        return records;
                    }
                }
                Err(e) => {
                    debug!("Upstream DNS lookup failed: {}", e);
                }
            }
        }

        vec![]
    }
}

#[async_trait::async_trait]
impl RequestHandler for DnsHandler {
    async fn handle_request<R: ResponseHandler>(
        &self,
        request: &Request,
        mut response_handle: R,
    ) -> ResponseInfo {
        let response = MessageResponseBuilder::from_message_request(request);

        if request.message_type() == MessageType::Query && request.op_code() == OpCode::Query {
            let answers = self.handle_query(request).await;

            if answers.is_empty() {
                let response_message = response.error_msg(request.header(), ResponseCode::NXDomain);
                response_handle
                    .send_response(response_message)
                    .await
                    .map_err(|e| warn!("Error sending DNS response: {}", e))
                    .ok()
                    .expect("DNS response handler should return ResponseInfo")
            } else {
                let mut header = *request.header();
                header.set_response_code(ResponseCode::NoError);
                header.set_answer_count(answers.len() as u16);
                header.set_recursion_available(true);
                header.set_message_type(MessageType::Response);

                let response_message = response.build(header, answers.iter(), &[], &[], &[]);
                response_handle
                    .send_response(response_message)
                    .await
                    .map_err(|e| warn!("Error sending DNS response: {}", e))
                    .ok()
                    .expect("DNS response handler should return ResponseInfo")
            }
        } else {
            let response_message = response.error_msg(request.header(), ResponseCode::NotImp);
            response_handle
                .send_response(response_message)
                .await
                .map_err(|e| warn!("Error sending DNS response: {}", e))
                .ok()
                .expect("DNS response handler should return ResponseInfo")
        }
    }
}
