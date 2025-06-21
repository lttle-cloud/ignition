// heed based RAFT replicated (todo: replication) KV store

use anyhow::Result;
use heed::{
    Database, Env, EnvOpenOptions,
    types::{Bytes, Str},
};
use serde::{Serialize, de::DeserializeOwned};
use std::{marker::PhantomData, path::Path};
use tokio::fs::create_dir_all;

pub struct Set;
pub struct NotSet;

pub struct KeyBuilder<MN, MK, T, C, N, K, D>
where
    D: Serialize + DeserializeOwned,
{
    tenant: Option<String>,
    collection: Option<String>,
    namespace: Option<String>,
    key: Option<String>,

    _marker: PhantomData<(MN, MK, T, C, N, K, D)>,
}

impl<MN, MK, T, C, N, K, D> KeyBuilder<MN, MK, T, C, N, K, D>
where
    D: Serialize + DeserializeOwned,
{
    pub fn tenant(self, tenant: impl AsRef<str>) -> KeyBuilder<MN, MK, Set, C, N, K, D> {
        KeyBuilder {
            tenant: Some(tenant.as_ref().to_string()),
            collection: self.collection,
            namespace: self.namespace,
            key: self.key,
            _marker: PhantomData,
        }
    }

    pub fn collection(self, collection: impl AsRef<str>) -> KeyBuilder<MN, MK, T, Set, N, K, D> {
        KeyBuilder {
            tenant: self.tenant,
            collection: Some(collection.as_ref().to_string()),
            namespace: self.namespace,
            key: self.key,
            _marker: PhantomData,
        }
    }
}

impl<MK, T, C, N, K, D> KeyBuilder<Set, MK, T, C, N, K, D>
where
    D: Serialize + DeserializeOwned,
{
    pub fn namespace(self, namespace: impl AsRef<str>) -> KeyBuilder<Set, MK, T, C, Set, K, D> {
        KeyBuilder {
            tenant: self.tenant,
            collection: self.collection,
            namespace: Some(namespace.as_ref().to_string()),
            key: self.key,
            _marker: PhantomData,
        }
    }
}

impl<MN, T, C, N, K, D> KeyBuilder<MN, Set, T, C, N, K, D>
where
    D: Serialize + DeserializeOwned,
{
    pub fn key(self, key: impl AsRef<str>) -> KeyBuilder<MN, Set, T, C, N, Set, D> {
        KeyBuilder {
            tenant: self.tenant,
            collection: self.collection,
            namespace: self.namespace,
            key: Some(key.as_ref().to_string()),
            _marker: PhantomData,
        }
    }
}

pub struct Key<D>(String, PhantomData<D>)
where
    D: Serialize + DeserializeOwned;
pub struct PartialKey<D>(String, PhantomData<D>)
where
    D: Serialize + DeserializeOwned;

impl<D> Key<D>
where
    D: Serialize + DeserializeOwned,
{
    pub fn namespaced() -> KeyBuilder<Set, Set, NotSet, NotSet, NotSet, NotSet, D>
    where
        D: Serialize + DeserializeOwned,
    {
        KeyBuilder {
            tenant: None,
            collection: None,
            namespace: None,
            key: None,
            _marker: PhantomData,
        }
    }

    pub fn not_namespaced() -> KeyBuilder<NotSet, Set, NotSet, NotSet, NotSet, NotSet, D>
    where
        D: Serialize + DeserializeOwned,
    {
        KeyBuilder {
            tenant: None,
            collection: None,
            namespace: None,
            key: None,
            _marker: PhantomData,
        }
    }
}

impl<D> PartialKey<D>
where
    D: Serialize + DeserializeOwned,
{
    pub fn namespaced() -> KeyBuilder<Set, NotSet, NotSet, NotSet, NotSet, NotSet, D> {
        KeyBuilder {
            tenant: None,
            collection: None,
            namespace: None,
            key: None,
            _marker: PhantomData,
        }
    }

    pub fn not_namespaced() -> KeyBuilder<NotSet, NotSet, NotSet, NotSet, NotSet, NotSet, D> {
        KeyBuilder {
            tenant: None,
            collection: None,
            namespace: None,
            key: None,
            _marker: PhantomData,
        }
    }
}

impl<D> From<&KeyBuilder<NotSet, NotSet, Set, Set, NotSet, NotSet, D>> for PartialKey<D>
where
    D: Serialize + DeserializeOwned,
{
    fn from(builder: &KeyBuilder<NotSet, NotSet, Set, Set, NotSet, NotSet, D>) -> Self {
        PartialKey::<D>(
            format!(
                "{}/{}",
                builder.tenant.as_ref().unwrap(),
                builder.collection.as_ref().unwrap(),
            ),
            PhantomData,
        )
    }
}

impl<D> From<&KeyBuilder<Set, Set, Set, Set, Set, Set, D>> for Key<D>
where
    D: Serialize + DeserializeOwned,
{
    fn from(builder: &KeyBuilder<Set, Set, Set, Set, Set, Set, D>) -> Self {
        Key::<D>(
            format!(
                "{}/{}/{}/{}",
                builder.tenant.as_ref().unwrap(),
                builder.collection.as_ref().unwrap(),
                builder.namespace.as_ref().unwrap(),
                builder.key.as_ref().unwrap(),
            ),
            PhantomData,
        )
    }
}

impl<D> From<&KeyBuilder<Set, NotSet, Set, Set, Set, NotSet, D>> for PartialKey<D>
where
    D: Serialize + DeserializeOwned,
{
    fn from(builder: &KeyBuilder<Set, NotSet, Set, Set, Set, NotSet, D>) -> Self {
        PartialKey::<D>(
            format!(
                "{}/{}/{}",
                builder.tenant.as_ref().unwrap(),
                builder.collection.as_ref().unwrap(),
                builder.namespace.as_ref().unwrap(),
            ),
            PhantomData,
        )
    }
}

impl<D> From<&KeyBuilder<NotSet, Set, Set, Set, NotSet, Set, D>> for Key<D>
where
    D: Serialize + DeserializeOwned,
{
    fn from(builder: &KeyBuilder<NotSet, Set, Set, Set, NotSet, Set, D>) -> Self {
        Key::<D>(
            format!(
                "{}/{}/{}",
                builder.tenant.as_ref().unwrap(),
                builder.collection.as_ref().unwrap(),
                builder.key.as_ref().unwrap(),
            ),
            PhantomData,
        )
    }
}

pub struct Store {
    db: Database<Str, Bytes>,
    env: Env,
}

impl Store {
    pub async fn new(dir_path: impl AsRef<Path>) -> Result<Self> {
        let dir_path = dir_path.as_ref();
        if !dir_path.exists() {
            create_dir_all(dir_path).await?;
        }

        let env = unsafe { EnvOpenOptions::new().open(dir_path)? };

        let db = {
            let mut wtxn = env.write_txn()?;
            let db: Database<Str, Bytes> = env.create_database(&mut wtxn, None)?;
            wtxn.commit()?;

            db
        };

        Ok(Self { db, env })
    }

    pub async fn get<D: Serialize + DeserializeOwned>(
        &self,
        key: impl Into<Key<D>>,
    ) -> Result<Option<D>> {
        let key: Key<D> = key.into();
        let rtxn = self.env.read_txn()?;
        let value = self.db.get(&rtxn, &key.0)?;
        Ok(value.map(|v| serde_json::from_slice(v).unwrap()))
    }

    pub async fn list<D: Serialize + DeserializeOwned>(
        &self,
        key: impl Into<PartialKey<D>>,
    ) -> Result<Vec<D>> {
        let key: PartialKey<D> = key.into();
        let rtxn = self.env.read_txn()?;
        let mut iter = self.db.prefix_iter(&rtxn, &key.0)?;

        let mut values = Vec::new();
        while let Some(Ok((_, v))) = iter.next() {
            let value: D = serde_json::from_slice(v)?;
            values.push(value);
        }
        Ok(values)
    }

    pub async fn list_keys<D: Serialize + DeserializeOwned>(
        &self,
        key: impl Into<PartialKey<D>>,
    ) -> Result<Vec<String>> {
        let key: PartialKey<D> = key.into();
        let rtxn = self.env.read_txn()?;
        let mut iter = self.db.prefix_iter(&rtxn, &key.0)?;

        let mut keys = Vec::new();
        while let Some(Ok((k, _))) = iter.next() {
            keys.push(k.to_string());
        }
        Ok(keys)
    }

    pub async fn put<D: Serialize + DeserializeOwned>(
        &self,
        key: impl Into<Key<D>>,
        value: impl Serialize,
    ) -> Result<()> {
        let key: Key<D> = key.into();
        let value = serde_json::to_string(&value)?.into_bytes();

        let mut wtxn = self.env.write_txn()?;
        self.db.put(&mut wtxn, &key.0, &value)?;
        wtxn.commit()?;

        Ok(())
    }

    pub async fn delete<D: Serialize + DeserializeOwned>(
        &self,
        key: impl Into<Key<D>>,
    ) -> Result<()> {
        let key: Key<D> = key.into();
        let mut wtxn = self.env.write_txn()?;
        self.db.delete(&mut wtxn, &key.0)?;
        wtxn.commit()?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_key_builder() {
        let dir = tempfile::tempdir().expect("failed to create tempdir");

        let store = Store::new(dir.path())
            .await
            .expect("failed to create store");

        let key = Key::<String>::namespaced()
            .tenant("test_tenant")
            .collection("test_collection")
            .namespace("test_namespace")
            .key("test_key");

        store
            .put(&key, "test_value")
            .await
            .expect("failed to put value");

        let key = Key::<String>::not_namespaced()
            .tenant("test_tenant")
            .collection("test_collection")
            .key("test_key");

        store
            .put(&key, "test_value")
            .await
            .expect("failed to put value");

        let key = PartialKey::<String>::not_namespaced()
            .tenant("test_tenant")
            .collection("test_collection");

        let keys = store.list_keys(&key).await.expect("failed to list keys");
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0], "test_tenant/test_collection/test_key");
        assert_eq!(
            keys[1],
            "test_tenant/test_collection/test_namespace/test_key"
        );

        let key = PartialKey::<String>::namespaced()
            .tenant("test_tenant")
            .collection("test_collection")
            .namespace("test_namespace");

        let keys = store.list_keys(&key).await.expect("failed to list keys");
        assert_eq!(keys.len(), 1);
        assert_eq!(
            keys[0],
            "test_tenant/test_collection/test_namespace/test_key"
        );
    }

    #[tokio::test]
    async fn test_store() {
        let dir = tempfile::tempdir().expect("failed to create tempdir");

        let store = Store::new(dir.path())
            .await
            .expect("failed to create store");

        // put
        let key = Key::not_namespaced()
            .tenant("test_tenant")
            .collection("test_collection")
            .key("test_key");

        let partial_key = PartialKey::not_namespaced()
            .tenant("test_tenant")
            .collection("test_collection");

        store
            .put(&key, "test_value")
            .await
            .expect("failed to put value");

        // list
        let keys = store
            .list::<String>(&partial_key)
            .await
            .expect("failed to list values");
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0], "test_value");

        // list keys
        let keys = store
            .list_keys(&partial_key)
            .await
            .expect("failed to list keys");
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0], "test_tenant/test_collection/test_key");

        // get
        let value = store.get(&key).await.expect("failed to get value");
        assert_eq!(value, Some("test_value".to_string()));

        // delete
        store.delete(&key).await.expect("failed to delete value");

        let value = store
            .get::<String>(&key)
            .await
            .expect("failed to get value");
        assert_eq!(value, None);
    }
}
