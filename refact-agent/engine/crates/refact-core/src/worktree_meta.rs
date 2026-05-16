use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use crate::chat_types::{serialize_path, deserialize_path};

fn default_enforce() -> bool {
    false
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorktreeMeta {
    pub id: String,
    pub kind: String,
    #[serde(
        serialize_with = "serialize_path",
        deserialize_with = "deserialize_path"
    )]
    pub root: PathBuf,
    #[serde(
        serialize_with = "serialize_path",
        deserialize_with = "deserialize_path"
    )]
    pub source_workspace_root: PathBuf,
    #[serde(
        serialize_with = "serialize_path",
        deserialize_with = "deserialize_path"
    )]
    pub repo_root: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default = "default_enforce")]
    pub enforce: bool,
}
