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

use crate::resources::{Convert, metadata::Metadata};

use super::{DnsHandler, ServiceDnsEntry};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DnsSubdomain {
    Service,
    Machine,
}

impl DnsSubdomain {
    fn from_str(s: &str) -> Option<Self> {
        match s {
            "svc" => Some(Self::Service),
            "machine" => Some(Self::Machine),
            _ => None,
        }
    }
}

impl DnsHandler {
    async fn ip_reservation_lookup(&self, ip: &str) -> Option<String> {
        self.net_agent.ip_reservation_lookup(ip).ok().flatten()
    }

    async fn resolve_service(
        &self,
        name: &str,
        namespace: &str,
        tenant: &str,
    ) -> Option<ServiceDnsEntry> {
        // Look up service in repository
        let service_repo = self.repository.service(tenant.to_string());

        let metadata = Metadata {
            name: name.to_string(),
            namespace: Some(namespace.to_string()),
        };

        if let Ok(Some((service, status))) = service_repo.get_with_status(metadata) {
            let service = service.latest();
            let entry = ServiceDnsEntry {
                service_ip: status.service_ip,
                target_machine: service.target.name.clone(),
                target_namespace: service
                    .target
                    .namespace
                    .or(service.namespace)
                    .unwrap_or_default(),
                port: service.target.port,
            };

            return Some(entry);
        }

        None
    }

    async fn resolve_machine(&self, name: &str, namespace: &str, tenant: &str) -> Option<String> {
        // Look up machine by network tag
        let network_tag = format!("{}-{}-{}", tenant, namespace, name);

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
        let tenant = match self.ip_reservation_lookup(&src_ip).await {
            Some(t) => t,
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
        let parts: Vec<&str> = name_str.trim_end_matches('.').split('.').collect();

        // Check if this is an Ignition domain query
        if parts.len() >= 4
            && parts[parts.len() - 2] == "lttle"
            && parts[parts.len() - 1] == "local"
        {
            let subdomain_str = parts[parts.len() - 3];

            if let Some(subdomain) = DnsSubdomain::from_str(subdomain_str) {
                if parts.len() >= 5 {
                    let resource_name = parts[0];
                    let namespace = parts[1];

                    match subdomain {
                        DnsSubdomain::Service => {
                            // Service query: <service>.<namespace>.svc.lttle.local
                            if let Some(entry) = self
                                .resolve_service(resource_name, namespace, &tenant)
                                .await
                            {
                                if let Some(service_ip) = entry.service_ip {
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
                        DnsSubdomain::Machine => {
                            // Machine query: <machine>.<namespace>.machine.lttle.local
                            if let Some(machine_ip) = self
                                .resolve_machine(resource_name, namespace, &tenant)
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
