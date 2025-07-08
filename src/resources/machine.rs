use anyhow::Result;
use meta::resource;

use crate::resources::{ConvertResource, FromResourceAsync};

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
    struct Status {}
}

impl ConvertResource<MachineV2> for MachineV1 {
    fn convert_up(this: Self) -> MachineV2 {
        MachineV2 {
            name: this.name,
            namespace: this.namespace,
            bleah: this.bleah,
            bleah2: 0,
        }
    }

    fn convert_down(this: MachineV2) -> Self {
        MachineV1 {
            namespace: this.namespace,
            name: this.name,
            bleah: this.bleah,
        }
    }
}

impl FromResourceAsync<Machine> for MachineStatus {
    async fn from_resource(_resource: Machine) -> Result<Self> {
        Ok(MachineStatus {})
    }
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
