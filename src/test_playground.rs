#[cfg(test)]
mod test {
    use std::sync::{Arc, Weak};

    use crate::api_client;
    use crate::machinery::store::Store;
    use crate::repository::Repository;
    use crate::resources::machine::{Machine, MachineV1};
    use crate::resources::metadata::Namespace;
    use crate::resources::{Convert, ProvideMetadata};

    #[tokio::test]
    async fn test_machine_repo() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::new(temp.path())
            .await
            .expect("failed to create store");

        let store = Arc::new(store);

        let repository = Repository::new(store, Weak::new());
        let machine_repo = repository.machine("test_tenant");

        let machine = Machine::V1(MachineV1 {
            name: "test".to_string(),
            namespace: None, // will use default
            bleah: 12,
        });

        machine_repo
            .set(machine)
            .await
            .expect("failed to set machine");

        //list the machines
        let machines = machine_repo
            .list(Namespace::Unspecified)
            .expect("failed to list machines");
        let machines = machines.latest();

        assert_eq!(machines.len(), 1);

        let metadata = machines[0].metadata();

        assert_eq!(machines[0].name, "test");
        assert_eq!(machines[0].namespace, Some("default".to_string()));
        assert_eq!(machines[0].name, metadata.name);
        assert_eq!(machines[0].namespace, metadata.namespace.into());
        assert_eq!(machines[0].bleah, 12);
    }

    #[tokio::test]
    async fn test_api_client() {
        let client = api_client::ApiClient::new(api_client::ApiClientConfig {
            base_url: "http://localhost:3000".to_string(),
        });

        let machines = client
            .machine()
            .list(Namespace::Unspecified)
            .await
            .expect("failed to list machines");

        println!("{:?}", machines);
    }
}
