pub const DEFAULT_AGENT_TENANT: &str = "agent";

pub enum Collections {
    ServiceIpReservation,
    VmIpReservation,
}

impl AsRef<str> for Collections {
    fn as_ref(&self) -> &str {
        match self {
            Collections::ServiceIpReservation => "service_ip_reservation",
            Collections::VmIpReservation => "vm_ip_reservation",
        }
    }
}
