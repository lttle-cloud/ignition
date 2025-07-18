use anyhow::Result;
use meta::resource;

use crate::resources::{Convert, ConvertResource, FromResource};

#[resource(name = "Machine", tag = "machine")]
mod machine {
    #[version(stored + served)]
    struct V1 {
        bleah: u32,
    }

    #[version(served + latest)]
    struct V2 {
        bleah: u32,
        bleah2: u32,
    }

    #[status]
    struct Status {
        test: u32,
        hash: u64,
    }
}

impl ConvertResource<MachineV2> for MachineV1 {
    fn convert_up(this: Self) -> MachineV2 {
        MachineV2 {
            name: this.name,
            namespace: this.namespace,
            tags: None,
            bleah: this.bleah,
            bleah2: 0,
        }
    }

    fn convert_down(this: MachineV2) -> Self {
        MachineV1 {
            namespace: this.namespace,
            name: this.name,
            tags: None,
            bleah: this.bleah,
        }
    }
}

impl FromResource<Machine> for MachineStatus {
    fn from_resource(resource: Machine) -> Result<Self> {
        let machine = resource.latest();

        Ok(MachineStatus {
            test: machine.bleah + machine.bleah2,
            hash: 0,
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::resources::{ProvideKey, ProvideMetadata, metadata::Namespace};

    #[test]
    fn test_resource_versioning() {
        let x = Machine::V1(MachineV1 {
            namespace: Some("test_ns".into()),
            name: "test".into(),
            tags: None,
            bleah: 1,
        });

        let metadata = x.metadata();
        assert_eq!(metadata.namespace, Some("test_ns".into()));
        assert_eq!(metadata.name, "test");

        let key = Machine::key("tenant".into(), metadata).expect("failed to get key");
        assert_eq!(key.to_string(), "tenant/machine/test_ns/test");

        let x = Machine::V1(MachineV1 {
            namespace: None,
            name: "test".into(),
            tags: None,
            bleah: 1,
        });

        let metadata = x.metadata();
        assert_eq!(metadata.namespace, Some("default".into()));
        assert_eq!(metadata.name, "test");

        let key = Machine::key("tenant".into(), metadata).expect("failed to get key");
        assert_eq!(key.to_string(), "tenant/machine/default/test");
    }

    #[test]
    fn test_resource_status() {
        let x = Machine::V1(MachineV1 {
            namespace: Some("test_ns".into()),
            name: "test".into(),
            tags: None,
            bleah: 1,
        });

        let metadata = x.metadata();

        let status_key =
            MachineStatus::key("tenant".into(), metadata.clone()).expect("failed to get key");
        assert_eq!(status_key.to_string(), "tenant/status-machine/test_ns/test");

        let status_key = MachineStatus::partial_key(
            "tenant".into(),
            Namespace::from_value(metadata.namespace.clone()),
        )
        .expect("failed to get key");
        assert_eq!(status_key.to_string(), "tenant/status-machine/test_ns/");
    }
}
