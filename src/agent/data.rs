pub enum Collections {
    ServiceIpReservation,
    VmIpReservation,
    Volume,
    Image,
    ImageLayer,
    AcmeAccount,
    AcmeChallenge,
    TrackedResourceOwner,
}

impl AsRef<str> for Collections {
    fn as_ref(&self) -> &str {
        match self {
            Collections::ServiceIpReservation => "service_ip_reservations",
            Collections::VmIpReservation => "vm_ip_reservations",
            Collections::Volume => "volumes",
            Collections::Image => "images",
            Collections::ImageLayer => "image_layers",
            Collections::AcmeAccount => "acme_accounts",
            Collections::AcmeChallenge => "acme_challenges",
            Collections::TrackedResourceOwner => "tracked_resource_owners",
        }
    }
}
