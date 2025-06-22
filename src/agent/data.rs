pub const DEFAULT_AGENT_TENANT: &str = "agent";

pub enum Collections {
    ServiceIpReservation,
    VmIpReservation,
    Volume,
    Image,
    ImageLayer,
}

impl AsRef<str> for Collections {
    fn as_ref(&self) -> &str {
        match self {
            Collections::ServiceIpReservation => "service_ip_reservations",
            Collections::VmIpReservation => "vm_ip_reservations",
            Collections::Volume => "volumes",
            Collections::Image => "images",
            Collections::ImageLayer => "image_layers",
        }
    }
}
