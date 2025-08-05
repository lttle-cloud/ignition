use std::net::Ipv4Addr;

use hickory_proto::{
    op::{MessageType, OpCode, ResponseCode},
    rr::{RData, Record, RecordType, rdata::A},
};
use hickory_server::{
    authority::MessageResponseBuilder,
    server::{Request, RequestHandler, ResponseHandler, ResponseInfo},
};
use tracing::{debug, warn};

use crate::{
    controller::context::ControllerKey,
    resource_index::ResourceKind,
    resources::metadata::{Metadata, Namespace},
    utils::machine_name_from_key,
};

use super::DnsHandler;

#[derive(Debug, Clone, PartialEq, Eq)]
enum DnsSubdomain {
    Service { name: String, namespace: String },
    Machine { name: String, namespace: String },
}

impl DnsSubdomain {
    fn parse_address(address: &str) -> Option<Self> {
        let parts: Vec<&str> = address.trim_end_matches('.').split('.').collect();

        // Check if this is an Ignition domain query (expect exactly 5 parts)
        if parts.len() != 5 || parts[3] != "lttle" || parts[4] != "local" {
            return None;
        }

        let name = parts[0];
        let namespace = parts[1];
        let subdomain_type = parts[2];

        match subdomain_type {
            "svc" => Some(Self::Service {
                name: name.to_string(),
                namespace: namespace.to_string(),
            }),
            "machine" => Some(Self::Machine {
                name: name.to_string(),
                namespace: namespace.to_string(),
            }),
            _ => None,
        }
    }
}

impl DnsHandler {
    async fn resolve_service(&self, name: &str, namespace: &str, tenant: &str) -> Option<String> {
        // Look up service in repository
        let service_repo = self.repository.service(tenant.to_string());

        let metadata = Metadata::new(name.to_string(), Namespace::specified(namespace));

        if let Ok(Some((_, status))) = service_repo.get_with_status(metadata) {
            return status.service_ip;
        }

        None
    }

    async fn resolve_machine(&self, name: &str, namespace: &str, tenant: &str) -> Option<String> {
        // Look up machine by network tag
        let key = ControllerKey::new(tenant, ResourceKind::Machine, Some(namespace), name);
        let network_tag = machine_name_from_key(&key);

        if let Some(machine) = self
            .machine_agent
            .get_machine_by_network_tag(&network_tag)
            .await
        {
            return Some(machine.config.network.ip_address.clone());
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
        let tenant = match self.net_agent.ip_reservation_lookup(&src_ip).ok().flatten() {
            Some(t) => t.tenant.clone(),
            None => {
                debug!("No tenant found for source IP: {}", src_ip);
                return vec![];
            }
        };

        debug!(
            "DNS query from tenant {}: {} {:?}",
            tenant, name, record_type
        );

        // Only handle A record queries for now
        if record_type != RecordType::A {
            return vec![];
        }

        // Parse the query name
        let name_str = name.to_string();

        if let Some(subdomain) = DnsSubdomain::parse_address(&name_str) {
            match subdomain {
                DnsSubdomain::Service {
                    name: resource_name,
                    namespace,
                } => {
                    // Service query: <service>.<namespace>.svc.lttle.local
                    if let Some(service_ip) = self
                        .resolve_service(&resource_name, &namespace, &tenant)
                        .await
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
                DnsSubdomain::Machine {
                    name: resource_name,
                    namespace,
                } => {
                    // Machine query: <machine>.<namespace>.machine.lttle.local
                    if let Some(machine_ip) = self
                        .resolve_machine(&resource_name, &namespace, &tenant)
                        .await
                    {
                        if let Ok(ip) = machine_ip.parse::<Ipv4Addr>() {
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
