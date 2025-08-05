use anyhow::{Result, bail};
use meta::resource;

use crate::{
    resources::{AdmissionCheckStatus, Convert, FromResource, ProvideMetadata},
    utils::size::parse_human_readable_size,
};

#[resource(name = "Volume", tag = "volume")]
mod volume {

    #[version(stored + served + latest)]
    struct V1 {
        mode: VolumeMode,
        /// The size of the volume in human readable format
        size: String,
    }

    #[schema]
    enum VolumeMode {
        #[serde(rename = "read-only")]
        ReadOnly,
        #[serde(rename = "writeable")]
        Writeable,
    }

    #[status]
    struct Status {
        hash: u64,
        volume_id: Option<String>,
        size_bytes: u64,
    }
}

impl FromResource<Volume> for VolumeStatus {
    fn from_resource(volume: Volume) -> Result<Self> {
        let volume = volume.latest();
        let size_bytes = parse_human_readable_size(&volume.size)?;

        Ok(VolumeStatus {
            volume_id: None,
            hash: 0,
            size_bytes,
        })
    }
}

impl Volume {
    pub fn hash_with_updated_metadata(&self) -> u64 {
        use std::hash::{DefaultHasher, Hash, Hasher};

        let metadata = self.metadata();
        let mut volume = self.stored();
        volume.namespace = metadata.namespace;
        let volume: Volume = volume.into();

        let mut hasher = DefaultHasher::new();
        volume.hash(&mut hasher);
        hasher.finish()
    }
}

impl AdmissionCheckStatus<VolumeStatus> for Volume {
    fn admission_check_status(&self, status: &VolumeStatus) -> Result<()> {
        let hash = self.hash_with_updated_metadata();

        if hash != status.hash {
            bail!("Volumes are not allowed to change their configuration after creation");
        }

        Ok(())
    }
}
