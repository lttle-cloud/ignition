use util::encoding::codec;

use crate::ignition_proto::image;

#[codec]
#[derive(Clone)]
pub struct ImagePullSecret {
    pub owner_id: String,
    pub name: String,
    pub username: String,
    pub password: String,
}

impl From<ImagePullSecret> for image::ImagePullSecret {
    fn from(secret: ImagePullSecret) -> Self {
        Self {
            name: secret.name,
            username: secret.username,
        }
    }
}

pub fn image_pull_secret_collection_prefix(owner_id: &str) -> String {
    format!("{}:", owner_id)
}

pub fn image_pull_secret_key(owner_id: &str, name: &str) -> String {
    format!("{}{}", image_pull_secret_collection_prefix(owner_id), name)
}
