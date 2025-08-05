#[derive(Debug, Clone)]
pub struct DnsAgentConfig {
    /// Address to bind the DNS server to (e.g., "10.1.0.1:53")
    pub bind_address: String,
    /// DNS zone suffix (e.g., "lttle.local")
    pub zone_suffix: String,
    /// TTL for DNS records in seconds
    pub default_ttl: u32,
    /// Upstream DNS servers for passthrough (e.g., ["8.8.8.8:53", "8.8.4.4:53"])
    pub upstream_dns_servers: Vec<String>,
}
