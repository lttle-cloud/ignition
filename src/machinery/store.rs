// heed based RAFT replicated (todo: replication) KV store
#![allow(dead_code)]

use anyhow::Result;
use heed::{
    Database, Env, EnvOpenOptions,
    types::{Bytes, Str},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::{HashMap, HashSet},
    marker::PhantomData,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::fs::create_dir_all;

const CORE_TENANT: &str = "__core__";

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
    pub fn as_ref(&self) -> &KeyBuilder<MN, MK, T, C, N, K, D> {
        self
    }

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

#[derive(Clone, Debug)]
pub struct Key<D>
where
    D: Serialize + DeserializeOwned,
{
    key: String,
    tenant: String,
    namespace: Option<String>,
    _marker: PhantomData<D>,
}

#[derive(Clone, Debug)]
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

impl<D> From<&KeyBuilder<Set, Set, Set, Set, Set, Set, D>> for Key<D>
where
    D: Serialize + DeserializeOwned,
{
    fn from(builder: &KeyBuilder<Set, Set, Set, Set, Set, Set, D>) -> Self {
        let key = format!(
            "{}/{}/{}/{}",
            builder.tenant.as_ref().unwrap(),
            builder.collection.as_ref().unwrap(),
            builder.namespace.as_ref().unwrap(),
            builder.key.as_ref().unwrap(),
        );

        Key {
            key,
            tenant: builder.tenant.as_ref().unwrap().clone(),
            namespace: builder.namespace.clone(),
            _marker: PhantomData,
        }
    }
}

impl<D, MN, N> From<&KeyBuilder<MN, NotSet, Set, Set, N, NotSet, D>> for PartialKey<D>
where
    D: Serialize + DeserializeOwned,
{
    fn from(builder: &KeyBuilder<MN, NotSet, Set, Set, N, NotSet, D>) -> Self {
        if let Some(namespace) = &builder.namespace {
            PartialKey::<D>(
                format!(
                    "{}/{}/{}/",
                    builder.tenant.as_ref().unwrap(),
                    builder.collection.as_ref().unwrap(),
                    namespace
                ),
                PhantomData,
            )
        } else {
            PartialKey::<D>(
                format!(
                    "{}/{}/",
                    builder.tenant.as_ref().unwrap(),
                    builder.collection.as_ref().unwrap(),
                ),
                PhantomData,
            )
        }
    }
}

impl<D> From<&KeyBuilder<NotSet, Set, Set, Set, NotSet, Set, D>> for Key<D>
where
    D: Serialize + DeserializeOwned,
{
    fn from(builder: &KeyBuilder<NotSet, Set, Set, Set, NotSet, Set, D>) -> Self {
        let key = format!(
            "{}/{}/{}",
            builder.tenant.as_ref().unwrap(),
            builder.collection.as_ref().unwrap(),
            builder.key.as_ref().unwrap(),
        );

        Key {
            key,
            tenant: builder.tenant.as_ref().unwrap().clone(),
            namespace: None,
            _marker: PhantomData,
        }
    }
}

impl<D> ToString for Key<D>
where
    D: Serialize + DeserializeOwned,
{
    fn to_string(&self) -> String {
        self.key.clone()
    }
}

impl<D> ToString for PartialKey<D>
where
    D: Serialize + DeserializeOwned,
{
    fn to_string(&self) -> String {
        self.0.clone()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TrackedNamespaces {
    tenant: String,
    namespaces: HashMap<String, TrackedNamespace>,
}

impl TrackedNamespaces {
    fn new(tenant: String) -> Self {
        Self {
            tenant,
            namespaces: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackedNamespace {
    pub namespace: String,
    pub created_at: u64,
}

pub fn now_millis() -> u64 {
    let now = SystemTime::now();
    let since_the_epoch = now.duration_since(UNIX_EPOCH).unwrap();
    since_the_epoch.as_millis() as u64
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

        let env = unsafe {
            EnvOpenOptions::new()
                .map_size(100 * 1024 * 1024) // 100MB store
                .open(dir_path)?
        };

        let db = {
            let mut wtxn = env.write_txn()?;
            let db: Database<Str, Bytes> = env.create_database(&mut wtxn, None)?;
            wtxn.commit()?;

            db
        };

        Ok(Self { db, env })
    }

    fn track_namespace_for_key<D: Serialize + DeserializeOwned>(
        &self,
        key: impl Into<Key<D>>,
    ) -> Result<()> {
        let key: Key<D> = key.into();
        let Some(namespace) = key.namespace else {
            return Ok(());
        };

        if key.tenant == CORE_TENANT {
            return Ok(());
        }

        let tracked_namespace_key = Key::<TrackedNamespaces>::not_namespaced()
            .tenant(CORE_TENANT)
            .collection("tracked_namespaces")
            .key(key.tenant.clone());

        let tenant = key.tenant.clone();
        let mut tracked_namespaces = self
            .get(&tracked_namespace_key)?
            .unwrap_or_else(|| TrackedNamespaces::new(tenant));

        let tracked_namespace = tracked_namespaces
            .namespaces
            .get(&namespace)
            .cloned()
            .unwrap_or_else(|| TrackedNamespace {
                namespace: namespace.clone(),
                created_at: now_millis(),
            });

        tracked_namespaces
            .namespaces
            .insert(namespace, tracked_namespace);

        self.put(&tracked_namespace_key, &tracked_namespaces)?;

        Ok(())
    }

    pub fn untrack_namespace_for_tenant(
        &self,
        tenant: impl AsRef<str>,
        namespace: impl AsRef<str>,
    ) -> Result<()> {
        let key = Key::<TrackedNamespaces>::not_namespaced()
            .tenant(CORE_TENANT)
            .collection("tracked_namespaces")
            .key(tenant.as_ref().to_string());

        let mut tracked_namespaces = self
            .get(&key)?
            .unwrap_or_else(|| TrackedNamespaces::new(tenant.as_ref().to_string()));
        tracked_namespaces.namespaces.remove(namespace.as_ref());

        self.put(&key, &tracked_namespaces)?;

        Ok(())
    }

    pub fn list_tracked_namespaces(
        &self,
        tenant: impl AsRef<str>,
    ) -> Result<Vec<TrackedNamespace>> {
        let key = Key::<TrackedNamespaces>::not_namespaced()
            .tenant(CORE_TENANT)
            .collection("tracked_namespaces")
            .key(tenant.as_ref().to_string());

        if let Some(tracked_namespaces) = self.get(&key)? {
            Ok(tracked_namespaces.namespaces.values().cloned().collect())
        } else {
            Ok(vec![])
        }
    }

    pub fn list_tenants(&self) -> Result<Vec<String>> {
        let key = PartialKey::<TrackedNamespaces>::not_namespaced()
            .tenant(CORE_TENANT)
            .collection("tracked_namespaces");

        let mut tenants = HashSet::new();

        let tracked_namespaces = self.list(&key)?;
        for tracked_namespace in tracked_namespaces {
            tenants.insert(tracked_namespace.tenant);
        }

        Ok(tenants.into_iter().collect())
    }

    pub fn get<D: Serialize + DeserializeOwned>(
        &self,
        key: impl Into<Key<D>>,
    ) -> Result<Option<D>> {
        let key: Key<D> = key.into();
        let rtxn = self.env.read_txn()?;
        let value = self.db.get(&rtxn, &key.key)?;
        Ok(value.map(|v| serde_json::from_slice(v).unwrap()))
    }

    pub fn list<D: Serialize + DeserializeOwned>(
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

    pub fn list_keys<D: Serialize + DeserializeOwned>(
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

    pub fn put<D: Serialize + DeserializeOwned>(
        &self,
        key: impl Into<Key<D>>,
        value: impl Serialize,
    ) -> Result<()> {
        let key: Key<D> = key.into();
        let value = serde_json::to_string(&value)?.into_bytes();

        let mut wtxn = self.env.write_txn()?;
        self.db.put(&mut wtxn, &key.key, &value)?;
        wtxn.commit()?;

        self.track_namespace_for_key(key)?;

        Ok(())
    }

    pub fn delete<D: Serialize + DeserializeOwned>(&self, key: impl Into<Key<D>>) -> Result<()> {
        let key: Key<D> = key.into();
        let mut wtxn = self.env.write_txn()?;
        self.db.delete(&mut wtxn, &key.key)?;
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

        store.put(&key, "test_value").expect("failed to put value");

        let key = Key::<String>::not_namespaced()
            .tenant("test_tenant")
            .collection("test_collection")
            .key("test_key");

        store.put(&key, "test_value").expect("failed to put value");

        let key = PartialKey::<String>::not_namespaced()
            .tenant("test_tenant")
            .collection("test_collection");

        let keys = store.list_keys(&key).expect("failed to list keys");
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

        let keys = store.list_keys(&key).expect("failed to list keys");
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

        store.put(&key, "test_value").expect("failed to put value");

        // list
        let keys = store
            .list::<String>(&partial_key)
            .expect("failed to list values");
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0], "test_value");

        // list keys
        let keys = store.list_keys(&partial_key).expect("failed to list keys");
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0], "test_tenant/test_collection/test_key");

        // get
        let value = store.get(&key).expect("failed to get value");
        assert_eq!(value, Some("test_value".to_string()));

        // delete
        store.delete(&key).expect("failed to delete value");

        let value = store.get::<String>(&key).expect("failed to get value");
        assert_eq!(value, None);
    }
}
