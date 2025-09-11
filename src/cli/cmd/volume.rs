use anyhow::Result;
use ignition::{
    resources::volume::{VolumeLatest, VolumeMode, VolumeStatus},
    utils::size::format_human_readable_size,
};
use meta::{summary, table};

use crate::{
    client::get_api_client,
    cmd::{DeleteNamespacedArgs, GetNamespacedArgs, ListNamespacedArgs},
    config::Config,
    ui::message::{message_info, message_warn},
};

#[table]
pub struct VolumeTable {
    #[field(name = "name")]
    name: String,

    #[field(name = "namespace")]
    namespace: Option<String>,

    #[field(name = "mode", cell_style = important)]
    mode: String,

    #[field(name = "size")]
    size: String,
}

#[summary]
pub struct VolumeSummary {
    #[field(name = "name")]
    name: String,

    #[field(name = "namespace")]
    namespace: Option<String>,

    #[field(name = "tags")]
    tags: Vec<String>,

    #[field(name = "mode", cell_style = important)]
    mode: String,

    #[field(name = "size")]
    size: String,

    #[field(name = "volume id (internal)")]
    volume_id: Option<String>,

    #[field(name = "size in bytes (internal)")]
    size_bytes: String,
}

impl From<(VolumeLatest, VolumeStatus)> for VolumeTableRow {
    fn from((volume, status): (VolumeLatest, VolumeStatus)) -> Self {
        let mode = match volume.mode {
            VolumeMode::ReadOnly => "read-only".to_string(),
            VolumeMode::Writeable => "writeable".to_string(),
        };

        let size = format_human_readable_size(status.size_bytes);

        Self {
            name: volume.name,
            namespace: volume.namespace,
            mode,
            size,
        }
    }
}

impl From<(VolumeLatest, VolumeStatus)> for VolumeSummary {
    fn from((volume, status): (VolumeLatest, VolumeStatus)) -> Self {
        let mode = match volume.mode {
            VolumeMode::ReadOnly => "read-only".to_string(),
            VolumeMode::Writeable => "writeable".to_string(),
        };
        let size = format_human_readable_size(status.size_bytes);

        let volume_id = status.volume_id.clone();
        let size_bytes = status.size_bytes;

        Self {
            name: volume.name,
            namespace: volume.namespace,
            tags: volume.tags.unwrap_or_default(),
            mode,
            size,
            volume_id,
            size_bytes: size_bytes.to_string(),
        }
    }
}

pub async fn run_volume_list(config: &Config, args: ListNamespacedArgs) -> Result<()> {
    let api_client = get_api_client(config.try_into()?);
    let volumes = api_client.volume().list(args.into()).await?;

    let mut table = VolumeTable::new();

    for (volume, status) in volumes {
        table.add_row(VolumeTableRow::from((volume, status)));
    }

    table.print();

    Ok(())
}

pub async fn run_volume_get(config: &Config, args: GetNamespacedArgs) -> Result<()> {
    let api_client = get_api_client(config.try_into()?);
    let (volume, status) = api_client
        .volume()
        .get(args.clone().into(), args.name)
        .await?;

    let summary = VolumeSummary::from((volume, status));
    summary.print();

    Ok(())
}

pub async fn run_volume_delete(config: &Config, args: DeleteNamespacedArgs) -> Result<()> {
    let api_client = get_api_client(config.try_into()?);
    if !args.confirm {
        message_warn(format!(
            "You are about to delete the volume '{}'. This action cannot be undone. To confirm, run the command with --yes (or -y).",
            args.name
        ));
        return Ok(());
    }

    api_client
        .volume()
        .delete(args.clone().into(), args.name.clone())
        .await?;

    message_info(format!("Volume '{}' has been deleted.", args.name));

    Ok(())
}
