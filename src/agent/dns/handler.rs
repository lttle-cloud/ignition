use std::{net::Ipv4Addr, sync::Arc};

use hickory_proto::{
    op::{Header, MessageType, OpCode, ResponseCode},
    rr::{DNSClass, Name, RData, Record, RecordType, rdata::A},
};
use hickory_server::{
    authority::MessageResponseBuilder,
    server::{Request, RequestHandler, ResponseHandler, ResponseInfo},
};
use tracing::{debug, warn};

use crate::{
    agent::machine::MachineAgent, constants::DEFAULT_AGENT_TENANT, machinery::store::Store,
    repository::Repository, resources::metadata::Metadata,
};

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
    async fn resolve_service(&self, name: &str, namespace: &str) -> Option<ServiceDnsEntry> {
        let tenant = DEFAULT_AGENT_TENANT;
        let cache_key = format!("{}.{}.{}", name, namespace, tenant);

        // Check cache first
        let guard = self.service_cache.guard();
        if let Some(entry) = self.service_cache.get(&cache_key, &guard) {
            return Some(entry.clone());
        }

        // Look up service in repository
        let repository = Repository::new(self.store.clone(), std::sync::Weak::new());
        let service_repo = repository.service(tenant.to_string());

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

            // Update cache
            let guard = self.service_cache.guard();
            self.service_cache.insert(cache_key, entry.clone(), &guard);
            return Some(entry);
        }

        None
    }

    async fn resolve_machine(&self, name: &str, namespace: &str) -> Option<String> {
        let tenant = DEFAULT_AGENT_TENANT;
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

        debug!("DNS query: {} {:?}", name, record_type);

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
                            if let Some(entry) =
                                self.resolve_service(resource_name, namespace).await
                            {
                                if let Some(service_ip) = entry.service_ip {
                                    if let Ok(ip) = service_ip.parse::<Ipv4Addr>() {
                                        return vec![Record::from_rdata(
                                            name.clone(),
                                            self.default_ttl,
                                            RData::A(A(ip)),
                                        )];
                                    }
                                }
                            }
                        }
                        DnsSubdomain::Machine => {
                            // Machine query: <machine>.<namespace>.machine.lttle.local
                            if let Some(machine_ip) =
                                self.resolve_machine(resource_name, namespace).await
                            {
                                if let Ok(ip) = machine_ip.parse::<Ipv4Addr>() {
                                    return vec![Record::from_rdata(
                                        name.clone(),
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
        let mut response = MessageResponseBuilder::from_message_request(request);

        match request.message_type() {
            MessageType::Query => match request.op_code() {
                OpCode::Query => {
                    let answers = self.handle_query(request).await;

                    if answers.is_empty() {
                        response.error_msg(request.header(), ResponseCode::NXDomain);
                    } else {
                        for answer in answers {
                            response.answers(answer);
                        }
                    }
                }
                _ => {
                    response.error_msg(request.header(), ResponseCode::NotImp);
                }
            },
            _ => {
                response.error_msg(request.header(), ResponseCode::NotImp);
            }
        }

        let response_message = response.build_no_records(ResponseCode::NoError);

        match response_handle.send_response(response_message).await {
            Ok(info) => info,
            Err(e) => {
                warn!("Error sending DNS response: {}", e);
                ResponseInfo::empty()
            }
        }
    }
}
