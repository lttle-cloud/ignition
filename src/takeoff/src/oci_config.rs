use std::collections::{HashMap, HashSet};

use serde::{ser::SerializeMap, Deserialize, Deserializer, Serialize, Serializer};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "PascalCase")]
pub struct OciConfig {
    /// The username or UID which is a platform-specific structure
    /// that allows specific control over which user the process run as. This acts as a default value to use when the value is
    /// not specified when creating a container. For Linux based
    /// systems, all of the following are valid: `user`, `uid`,
    /// `user:group`, `uid:gid`, `uid:group`, `user:gid`. If `group`/`gid` is
    /// not specified, the default group and supplementary groups
    /// of the given `user`/`uid` in `/etc/passwd` from the container are
    /// applied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,

    /// A set of ports to expose from a container running this
    /// image. Its keys can be in the format of: `port/tcp`, `port/udp`,
    /// `port` with the default protocol being `tcp` if not specified.
    /// These values act as defaults and are merged with any
    /// specified when creating a container.
    #[serde(
        skip_serializing_if = "is_option_hashset_empty",
        deserialize_with = "optional_hashset_from_str",
        serialize_with = "serialize_optional_hashset",
        default
    )]
    pub exposed_ports: Option<HashSet<String>>,

    /// Entries are in the format of `VARNAME=VARVALUE`.
    #[serde(skip_serializing_if = "is_option_vec_empty")]
    pub env: Option<Vec<String>>,

    /// Default arguments to the entrypoint of the container.
    #[serde(skip_serializing_if = "is_option_vec_empty")]
    pub cmd: Option<Vec<String>>,

    /// A list of arguments to use as the command to execute when
    /// the container starts..
    #[serde(skip_serializing_if = "is_option_vec_empty")]
    pub entrypoint: Option<Vec<String>>,

    /// A set of directories describing where the process is likely write data specific to a container instance.
    #[serde(
        skip_serializing_if = "is_option_hashset_empty",
        deserialize_with = "optional_hashset_from_str",
        serialize_with = "serialize_optional_hashset",
        default
    )]
    pub volumes: Option<HashSet<String>>,

    /// Sets the current working directory of the entrypoint
    /// process in the container.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,

    /// The field contains arbitrary metadata for the container.
    /// This property MUST use the [annotation rules](https://github.com/opencontainers/image-spec/blob/v1.0/annotations.md#rules).
    #[serde(skip_serializing_if = "is_option_hashmap_empty")]
    pub labels: Option<HashMap<String, String>>,

    /// The field contains the system call signal that will be sent
    /// to the container to exit. The signal can be a signal name
    /// in the format `SIGNAME`, for instance `SIGKILL` or `SIGRTMIN+3`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_signal: Option<String>,
}

fn is_option_hashset_empty<T>(opt_hash: &Option<HashSet<T>>) -> bool {
    if let Some(hash) = opt_hash {
        hash.is_empty()
    } else {
        true
    }
}

fn is_option_hashmap_empty<T, V>(opt_hash: &Option<HashMap<T, V>>) -> bool {
    if let Some(hash) = opt_hash {
        hash.is_empty()
    } else {
        true
    }
}

/// Default value of the type of a [`Rootfs`]
pub const ROOTFS_TYPE: &str = "layers";

/// The rootfs key references the layer content addresses used by the image.
/// This makes the image config hash depend on the filesystem hash.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Rootfs {
    /// MUST be set to `layers`.
    pub r#type: String,

    /// An array of layer content hashes (`DiffIDs`), in order from first to last.
    pub diff_ids: Vec<String>,
}

impl Default for Rootfs {
    fn default() -> Self {
        Self {
            r#type: String::from(ROOTFS_TYPE),
            diff_ids: Default::default(),
        }
    }
}

fn is_option_vec_empty<T>(opt_vec: &Option<Vec<T>>) -> bool {
    if let Some(vec) = opt_vec {
        vec.is_empty()
    } else {
        true
    }
}

/// Helper struct to be serialized into and deserialized from `{}`
#[derive(Deserialize, Serialize)]
struct Empty {}

/// Helper to deserialize a `map[string]struct{}` of golang
fn optional_hashset_from_str<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<HashSet<String>>, D::Error> {
    let res = <Option<HashMap<String, Empty>>>::deserialize(d)?.map(|h| h.into_keys().collect());
    Ok(res)
}

/// Helper to serialize an optional hashset
fn serialize_optional_hashset<T, S>(
    value: &Option<HashSet<T>>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    T: Serialize,
    S: Serializer,
{
    match value {
        Some(set) => {
            let empty = Empty {};
            let mut map = serializer.serialize_map(Some(set.len()))?;
            for k in set {
                map.serialize_entry(k, &empty)?;
            }

            map.end()
        }
        None => serializer.serialize_none(),
    }
}
