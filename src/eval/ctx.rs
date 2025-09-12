use std::fmt::Debug;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
pub struct GitInfo {
    pub branch: Option<String>,

    #[serde(rename = "commitSha")]
    pub commit_sha: String, // 8 chars

    #[serde(rename = "commitMessage")]
    pub commit_message: String,

    #[serde(rename = "tag")]
    pub tag: Option<String>,

    #[serde(rename = "latestTag")]
    pub latest_tag: Option<String>,

    #[serde(rename = "ref")]
    pub r#ref: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct LttleInfo {
    pub tenant: String,
    pub user: String,
    pub profile: String,
}

impl Debug for GitInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("")
            .field("branch", &self.branch)
            .field("commitSha", &self.commit_sha)
            .field("commitMessage", &self.commit_message)
            .field("tag", &self.tag)
            .field("latestTag", &self.latest_tag)
            .field("ref", &self.r#ref)
            .finish()
    }
}

impl Debug for LttleInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("")
            .field("tenant", &self.tenant)
            .field("user", &self.user)
            .field("profile", &self.profile)
            .finish()
    }
}
