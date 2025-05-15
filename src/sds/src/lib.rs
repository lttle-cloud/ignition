use std::{path::PathBuf, sync::Arc};

use util::{
    encoding::{DeserializeOwned, Serialize},
    result::{Context, Result},
};

pub use heed::Error;

#[derive(Debug, Clone)]
pub struct StoreConfig {
    pub dir_path: PathBuf,
    pub size_mib: usize,
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
    database: heed::Database<heed::types::Str, heed::types::SerdeBincode<T>>,
}

pub struct ReadTxn<'env> {
    txn: heed::RoTxn<'env>,
}

pub struct WriteTxn<'env> {
    txn: heed::RwTxn<'env>,
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

        Ok(Collection { database })
    }

    pub fn read_txn(&self) -> Result<ReadTxn> {
        let txn = self.env.read_txn()?;
        Ok(ReadTxn { txn })
    }

    pub fn write_txn(&self) -> Result<WriteTxn> {
        let txn = self.env.write_txn()?;
        Ok(WriteTxn { txn })
    }
}

impl<'env> ReadTxn<'env> {
    pub fn get<V>(&self, collection: &Collection<V>, key: &str) -> Option<V>
    where
        V: Serialize + DeserializeOwned,
    {
        let Ok(Some(value)) = collection.database.get(&self.txn, key) else {
            return None;
        };
        Some(value)
    }

    pub fn iter<V>(
        &self,
        collection: &Collection<V>,
    ) -> Result<heed::RoIter<'_, heed::types::Str, heed::types::SerdeBincode<V>>>
    where
        V: Serialize + DeserializeOwned,
    {
        Ok(collection.database.iter(&self.txn)?)
    }

    pub fn prefix_iter<V>(
        &self,
        collection: &Collection<V>,
        prefix: impl AsRef<str>,
    ) -> Result<heed::RoPrefix<'_, heed::types::Str, heed::types::SerdeBincode<V>>>
    where
        V: Serialize + DeserializeOwned,
    {
        Ok(collection
            .database
            .prefix_iter(&self.txn, prefix.as_ref())?)
    }

    pub fn find_first<V, F>(
        &self,
        collection: &Collection<V>,
        predicate: F,
    ) -> Result<Option<(String, V)>>
    where
        V: Serialize + DeserializeOwned,
        F: Fn(&str, &V) -> bool,
    {
        let iter = collection.database.iter(&self.txn)?;
        for item in iter {
            let (key, value) = item?;
            if predicate(key, &value) {
                return Ok(Some((key.to_string(), value)));
            }
        }
        Ok(None)
    }

    pub fn get_all_keys<V>(&self, collection: &Collection<V>) -> Result<Vec<String>>
    where
        V: Serialize + DeserializeOwned,
    {
        let iter = self.iter(collection)?;
        let data = iter
            .map(|item| item.map(|(k, _)| k.to_string()))
            .collect::<Result<Vec<_>, Error>>()
            .context("failed to collect data")?;

        Ok(data)
    }

    pub fn get_all_values<V>(&self, collection: &Collection<V>) -> Result<Vec<V>>
    where
        V: Serialize + DeserializeOwned,
    {
        let iter = self.iter(collection)?;
        let data = iter
            .map(|item| item.map(|(_, v)| v))
            .collect::<Result<Vec<_>, Error>>()
            .context("failed to collect data")?;

        Ok(data)
    }

    pub fn get_all<V>(&self, collection: &Collection<V>) -> Result<Vec<(String, V)>>
    where
        V: Serialize + DeserializeOwned,
    {
        let iter = self.iter(collection)?;
        let data = iter
            .map(|item| item.map(|(k, v)| (k.to_string(), v)))
            .collect::<Result<Vec<_>, Error>>()
            .context("failed to collect data")?;

        Ok(data)
    }

    pub fn get_all_keys_prefix<V>(
        &self,
        collection: &Collection<V>,
        prefix: &str,
    ) -> Result<Vec<String>>
    where
        V: Serialize + DeserializeOwned,
    {
        let iter = self.prefix_iter(collection, prefix)?;
        let data = iter
            .map(|item| item.map(|(k, _)| k.to_string()))
            .collect::<Result<Vec<_>, Error>>()
            .context("failed to collect data")?;

        Ok(data)
    }

    pub fn get_all_values_prefix<V>(
        &self,
        collection: &Collection<V>,
        prefix: &str,
    ) -> Result<Vec<V>>
    where
        V: Serialize + DeserializeOwned,
    {
        let iter = self.prefix_iter(collection, prefix)?;
        let data = iter
            .map(|item| item.map(|(_, v)| v))
            .collect::<Result<Vec<_>, Error>>()
            .context("failed to collect data")?;

        Ok(data)
    }

    pub fn get_all_prefix<V>(
        &self,
        collection: &Collection<V>,
        prefix: &str,
    ) -> Result<Vec<(String, V)>>
    where
        V: Serialize + DeserializeOwned,
    {
        let iter = self.prefix_iter(collection, prefix)?;
        let data = iter
            .map(|item| item.map(|(k, v)| (k.to_string(), v)))
            .collect::<Result<Vec<_>, Error>>()
            .context("failed to collect data")?;

        Ok(data)
    }
}

impl<'env> WriteTxn<'env> {
    pub fn get<V>(&self, collection: &Collection<V>, key: &str) -> Option<V>
    where
        V: Serialize + DeserializeOwned,
    {
        let Ok(Some(value)) = collection.database.get(&self.txn, key) else {
            return None;
        };
        Some(value)
    }

    pub fn iter<V>(
        &self,
        collection: &Collection<V>,
    ) -> Result<heed::RoIter<'_, heed::types::Str, heed::types::SerdeBincode<V>>>
    where
        V: Serialize + DeserializeOwned,
    {
        Ok(collection.database.iter(&self.txn)?)
    }

    pub fn prefix_iter<V>(
        &self,
        collection: &Collection<V>,
        prefix: impl AsRef<str>,
    ) -> Result<heed::RoPrefix<'_, heed::types::Str, heed::types::SerdeBincode<V>>>
    where
        V: Serialize + DeserializeOwned,
    {
        Ok(collection
            .database
            .prefix_iter(&self.txn, prefix.as_ref())?)
    }

    pub fn get_all_keys<V>(&self, collection: &Collection<V>) -> Result<Vec<String>>
    where
        V: Serialize + DeserializeOwned,
    {
        let iter = self.iter(collection)?;
        let data = iter
            .map(|item| item.map(|(k, _)| k.to_string()))
            .collect::<Result<Vec<_>, Error>>()
            .context("failed to collect data")?;

        Ok(data)
    }

    pub fn get_all_values<V>(&self, collection: &Collection<V>) -> Result<Vec<V>>
    where
        V: Serialize + DeserializeOwned,
    {
        let iter = self.iter(collection)?;
        let data = iter
            .map(|item| item.map(|(_, v)| v))
            .collect::<Result<Vec<_>, Error>>()
            .context("failed to collect data")?;

        Ok(data)
    }

    pub fn get_all<V>(&self, collection: &Collection<V>) -> Result<Vec<(String, V)>>
    where
        V: Serialize + DeserializeOwned,
    {
        let iter = self.iter(collection)?;
        let data = iter
            .map(|item| item.map(|(k, v)| (k.to_string(), v)))
            .collect::<Result<Vec<_>, Error>>()
            .context("failed to collect data")?;

        Ok(data)
    }

    pub fn get_all_keys_prefix<V>(
        &self,
        collection: &Collection<V>,
        prefix: &str,
    ) -> Result<Vec<String>>
    where
        V: Serialize + DeserializeOwned,
    {
        let iter = self.prefix_iter(collection, prefix)?;
        let data = iter
            .map(|item| item.map(|(k, _)| k.to_string()))
            .collect::<Result<Vec<_>, Error>>()
            .context("failed to collect data")?;

        Ok(data)
    }

    pub fn get_all_values_prefix<V>(
        &self,
        collection: &Collection<V>,
        prefix: &str,
    ) -> Result<Vec<V>>
    where
        V: Serialize + DeserializeOwned,
    {
        let iter = self.prefix_iter(collection, prefix)?;
        let data = iter
            .map(|item| item.map(|(_, v)| v))
            .collect::<Result<Vec<_>, Error>>()
            .context("failed to collect data")?;

        Ok(data)
    }

    pub fn get_all_prefix<V>(
        &self,
        collection: &Collection<V>,
        prefix: &str,
    ) -> Result<Vec<(String, V)>>
    where
        V: Serialize + DeserializeOwned,
    {
        let iter = self.prefix_iter(collection, prefix)?;
        let data = iter
            .map(|item| item.map(|(k, v)| (k.to_string(), v)))
            .collect::<Result<Vec<_>, Error>>()
            .context("failed to collect data")?;

        Ok(data)
    }

    pub fn put<V>(&mut self, collection: &Collection<V>, key: &str, value: &V) -> Result<()>
    where
        V: Serialize + DeserializeOwned,
    {
        collection.database.put(&mut self.txn, key, value)?;
        Ok(())
    }

    pub fn del<V>(&mut self, collection: &Collection<V>, key: &str) -> Result<()>
    where
        V: Serialize + DeserializeOwned,
    {
        collection.database.delete(&mut self.txn, key)?;
        Ok(())
    }

    pub fn commit(self) -> Result<()> {
        self.txn.commit()?;
        Ok(())
    }

    pub fn find_first<V, F>(
        &self,
        collection: &Collection<V>,
        predicate: F,
    ) -> Result<Option<(String, V)>>
    where
        V: Serialize + DeserializeOwned,
        F: Fn(&str, &V) -> bool,
    {
        let iter = collection.database.iter(&self.txn)?;
        for item in iter {
            let (key, value) = item?;
            if predicate(key, &value) {
                return Ok(Some((key.to_string(), value)));
            }
        }
        Ok(None)
    }
}
