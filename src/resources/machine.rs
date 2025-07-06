use meta::resource;

#[resource(name = "Machine", tag = "machine")]
mod machine {
    #[version]
    struct V1 {
        bleah: u32,
    }

    #[status]
    struct Status {}
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::resources::{ProvideKey, ProvideMetadata};

    #[test]
    fn test_resource_versioning() {
        let x = Machine::V1(MachineV1 {
            namespace: Some("test_ns".into()),
            name: "test".into(),
            bleah: 1,
        });

        let metadata = x.metadata();
        assert_eq!(metadata.namespace, "test_ns");
        assert_eq!(metadata.name, "test");

        let key = Machine::key("tenant".into(), metadata);
        assert_eq!(key.to_string(), "tenant/machine/test_ns/test");

        let x = Machine::V1(MachineV1 {
            namespace: None,
            name: "test".into(),
            bleah: 1,
        });

        let metadata = x.metadata();
        assert_eq!(metadata.namespace, "default");
        assert_eq!(metadata.name, "test");

        let key = Machine::key("tenant".into(), metadata);
        assert_eq!(key.to_string(), "tenant/machine/default/test");
    }

    #[test]
    fn test_resource_status() {
        let x = Machine::V1(MachineV1 {
            namespace: Some("test_ns".into()),
            name: "test".into(),
            bleah: 1,
        });

        let metadata = x.metadata();

        let status_key = MachineStatus::key("tenant".into(), metadata.clone());
        assert_eq!(status_key.to_string(), "tenant/status-machine/test_ns/test");

        let status_key = MachineStatus::partial_key("tenant".into(), metadata.namespace.into());
        assert_eq!(status_key.to_string(), "tenant/status-machine/test_ns/");
    }
}
