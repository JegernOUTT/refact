pub mod operations;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

fn serialize_path<S: serde::Serializer>(path: &PathBuf, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&path.to_string_lossy())
}

fn deserialize_path<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<PathBuf, D::Error> {
    Ok(PathBuf::from(String::deserialize(deserializer)?))
}

#[derive(Serialize, Deserialize, Debug)]
pub struct CommitInfo {
    pub project_path: url::Url,
    pub commit_message: String,
    pub staged_changes: Vec<FileChange>,
    pub unstaged_changes: Vec<FileChange>,
}

impl CommitInfo {
    pub fn get_project_name(&self) -> String {
        self.project_path
            .to_file_path()
            .ok()
            .and_then(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| "".to_string())
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FileChange {
    #[serde(
        serialize_with = "serialize_path",
        deserialize_with = "deserialize_path"
    )]
    pub relative_path: PathBuf,
    #[serde(
        serialize_with = "serialize_path",
        deserialize_with = "deserialize_path"
    )]
    pub absolute_path: PathBuf,
    pub status: FileChangeStatus,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum FileChangeStatus {
    ADDED,
    MODIFIED,
    DELETED,
}

impl FileChangeStatus {
    pub fn initial(&self) -> char {
        match self {
            FileChangeStatus::ADDED => 'A',
            FileChangeStatus::MODIFIED => 'M',
            FileChangeStatus::DELETED => 'D',
        }
    }
}

pub fn from_unix_glob_pattern_to_gitignore(pattern: &str) -> String {
    let parts = pattern
        .split('/')
        .skip_while(|&p| p.is_empty())
        .map(|part| if part == "*" { "**" } else { part })
        .collect::<Vec<_>>();

    if parts.first() != Some(&"**") {
        format!("**/{}", parts.join("/"))
    } else {
        parts.join("/")
    }
}
