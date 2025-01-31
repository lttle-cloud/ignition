use std::{path::PathBuf, sync::Arc};

use util::{
    encoding::{DeserializeOwned, Serialize},
    result::Result,
};

#[derive(Debug, Clone)]
pub struct StoreConfig {
    dir_path: PathBuf,
    size_mib: usize,
}

#[derive(Clone)]
pub struct Store {
    env: Arc<heed::Env>,
}

#[derive(Clone)]
pub struct Collection<T>
where
    T: Serialize + DeserializeOwned,
{
    env: Arc<heed::Env>,
    database: heed::Database<heed::types::Str, heed::types::SerdeBincode<T>>,
}

pub struct ReadTransaction<'a, T>
where
    T: Serialize + DeserializeOwned,
{
    txn: heed::RoTxn<'a>,
    database: heed::Database<heed::types::Str, heed::types::SerdeBincode<T>>,
}

pub struct WriteTransaction<'a, T>
where
    T: Serialize + DeserializeOwned,
{
    txn: heed::RwTxn<'a>,
    database: heed::Database<heed::types::Str, heed::types::SerdeBincode<T>>,
}

impl Store {
    pub fn new(config: StoreConfig) -> Result<Self> {
        if !config.dir_path.exists() {
            std::fs::create_dir_all(&config.dir_path)?;
        }

        let env = unsafe {
            heed::EnvOpenOptions::new()
                .map_size(config.size_mib * 1024 * 1024)
                .max_dbs(3000)
                .open(&config.dir_path)?
        };
        let env = Arc::new(env);

        Ok(Store { env })
    }

    pub fn collection<V>(&self, name: impl AsRef<str>) -> Result<Collection<V>>
    where
        V: Serialize + DeserializeOwned + 'static,
    {
        let name = name.as_ref().to_string();
        let database = {
            let mut wtxn = self.env.write_txn()?;
            let db = self
                .env
                .create_database::<heed::types::Str, heed::types::SerdeBincode<V>>(
                    &mut wtxn,
                    Some(&name),
                )?;

            wtxn.commit()?;

            db
        };

        Ok(Collection {
            env: self.env.clone(),
            database,
        })
    }
}

impl<V> Collection<V>
where
    V: Serialize + DeserializeOwned,
{
    pub fn read(&self) -> Result<ReadTransaction<V>> {
        let txn = self.env.read_txn()?;
        Ok(ReadTransaction {
            txn,
            database: self.database.clone(),
        })
    }

    pub fn write(&self) -> Result<WriteTransaction<V>> {
        let txn = self.env.write_txn()?;
        Ok(WriteTransaction {
            txn,
            database: self.database.clone(),
        })
    }
}

impl<'a, V> ReadTransaction<'a, V>
where
    V: Serialize + DeserializeOwned,
{
    pub fn get(&self, key: &str) -> Option<V> {
        let Ok(Some(value)) = self.database.get(&self.txn, key) else {
            return None;
        };

        Some(value)
    }

    pub fn iter(&self) -> Result<heed::RoIter<'_, heed::types::Str, heed::types::SerdeBincode<V>>> {
        let iter = self.database.iter(&self.txn)?;
        Ok(iter)
    }

    pub fn prefix_iter(
        &self,
        prefix: impl AsRef<str>,
    ) -> Result<heed::RoPrefix<'_, heed::types::Str, heed::types::SerdeBincode<V>>> {
        let iter = self.database.prefix_iter(&self.txn, prefix.as_ref())?;
        Ok(iter)
    }
}

impl<'a, V> WriteTransaction<'a, V>
where
    V: Serialize + DeserializeOwned,
{
    pub fn get(&self, key: &str) -> Option<V> {
        let Ok(Some(value)) = self.database.get(&self.txn, key) else {
            return None;
        };

        Some(value)
    }

    pub fn iter(&self) -> Result<heed::RoIter<'_, heed::types::Str, heed::types::SerdeBincode<V>>> {
        let iter = self.database.iter(&self.txn)?;
        Ok(iter)
    }

    pub fn prefix_iter(
        &self,
        prefix: impl AsRef<str>,
    ) -> Result<heed::RoPrefix<'_, heed::types::Str, heed::types::SerdeBincode<V>>> {
        let iter = self.database.prefix_iter(&self.txn, prefix.as_ref())?;
        Ok(iter)
    }

    pub fn iter_mut(
        &mut self,
    ) -> Result<heed::RwIter<'_, heed::types::Str, heed::types::SerdeBincode<V>>> {
        let iter = self.database.iter_mut(&mut self.txn)?;
        Ok(iter)
    }

    pub fn put(&mut self, key: &str, value: &V) -> Result<()> {
        self.database.put(&mut self.txn, key, value)?;
        Ok(())
    }

    pub fn del(&mut self, key: &str) -> Result<()> {
        self.database.delete(&mut self.txn, key)?;
        Ok(())
    }

    pub fn commit(self) -> Result<()> {
        self.txn.commit()?;
        Ok(())
    }
}
