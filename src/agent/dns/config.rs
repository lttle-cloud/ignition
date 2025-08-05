#[derive(Debug, Clone)]
pub struct DnsAgentConfig {
    /// Address to bind the DNS server to (e.g., "10.0.1.1:53")
    pub bind_address: String,
    /// DNS zone suffix (e.g., "lttle.local")
    pub zone_suffix: String,
    /// TTL for DNS records in seconds
    pub default_ttl: u32,
}
