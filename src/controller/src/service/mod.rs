use sds::{Collection, Store};
use util::result::Result;

use crate::model::service::Service;

pub struct ServicePool {
    store: Store,
    collection: Collection<Service>,
}

impl ServicePool {
    pub fn new(store: Store) -> Result<Self> {
        let collection = store.collection("services")?;

        Ok(Self { store, collection })
    }

    pub fn insert_service(&self, service: Service) -> Result<()> {
        let mut tx = self.store.write_txn()?;
        tx.put(&self.collection, &service.name, &service)?;
        tx.commit()?;

        Ok(())
    }

    pub fn remove_service(&self, service_name: &str) -> Result<()> {
        let mut tx = self.store.write_txn()?;
        tx.del(&self.collection, service_name)?;
        tx.commit()?;

        Ok(())
    }

    pub fn get_service(&self, service_name: &str) -> Result<Option<Service>> {
        let tx = self.store.read_txn()?;
        let service = tx.get(&self.collection, service_name);
        Ok(service.clone())
    }

    pub fn list_services(&self) -> Result<Vec<Service>> {
        let tx = self.store.read_txn()?;
        let services = tx.get_all_values(&self.collection)?;
        Ok(services)
    }
}
