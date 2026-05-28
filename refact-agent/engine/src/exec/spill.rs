use std::path::PathBuf;

use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::exec::types::ExecProcessId;

#[derive(Debug, Clone)]
pub struct SpillTarget {
    root: PathBuf,
    chat_component: String,
    process_component: String,
}

impl SpillTarget {
    pub fn new(chat_id: &str, process_id: &ExecProcessId) -> Result<Self, String> {
        let root = default_spill_root()?;
        Ok(Self::with_root(root, chat_id, process_id))
    }

    pub fn with_root(root: PathBuf, chat_id: &str, process_id: &ExecProcessId) -> Self {
        Self {
            root,
            chat_component: safe_spill_component("chat", chat_id),
            process_component: safe_spill_component("process", process_id.as_str()),
        }
    }

    pub fn dir(&self) -> PathBuf {
        self.root.join(&self.chat_component)
    }

    pub fn path(&self) -> PathBuf {
        self.dir().join(format!("{}.log", self.process_component))
    }
}

pub struct SpillWriter {
    path: PathBuf,
    file: tokio::fs::File,
}

impl SpillWriter {
    pub async fn create(target: &SpillTarget) -> Result<Self, String> {
        let dir = target.dir();
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|error| format!("failed to create exec spill directory: {error}"))?;
        let path = target.path();
        let file = tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&path)
            .await
            .map_err(|error| format!("failed to open exec spill log: {error}"))?;
        Ok(Self { path, file })
    }

    pub async fn write_line(&mut self, line: &str) -> Result<(), String> {
        self.file
            .write_all(line.as_bytes())
            .await
            .map_err(|error| format!("failed to write exec spill log: {error}"))?;
        self.file
            .flush()
            .await
            .map_err(|error| format!("failed to flush exec spill log: {error}"))
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }
}

fn default_spill_root() -> Result<PathBuf, String> {
    let home = home::home_dir().ok_or_else(|| "failed to resolve home directory".to_string())?;
    Ok(home.join(".cache").join("refact").join("exec"))
}

fn safe_spill_component(prefix: &str, raw: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    format!("{prefix}_{}", hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use std::path::Component;

    use super::*;

    #[tokio::test]
    async fn spill_target_writes_with_hashed_path_components() {
        let temp = tempfile::tempdir().unwrap();
        let target = SpillTarget::with_root(
            temp.path().to_path_buf(),
            "../escape/chat",
            &ExecProcessId("exec_../../evil".to_string()),
        );
        let mut writer = SpillWriter::create(&target).await.unwrap();
        writer.write_line("safe\n").await.unwrap();
        drop(writer);

        let relative = target
            .path()
            .strip_prefix(temp.path())
            .expect("path should stay under root")
            .to_path_buf();
        assert!(relative
            .components()
            .all(|component| matches!(component, Component::Normal(_))));
        assert!(!relative.to_string_lossy().contains(".."));
        assert!(!relative.to_string_lossy().contains("escape"));
        assert!(!relative.to_string_lossy().contains("evil"));
        assert_eq!(
            tokio::fs::read_to_string(target.path()).await.unwrap(),
            "safe\n"
        );
    }

    #[tokio::test]
    async fn spill_writer_truncates_reused_service_log() {
        let temp = tempfile::tempdir().unwrap();
        let process_id = ExecProcessId("exec_service_api_deadbeef".to_string());
        let target = SpillTarget::with_root(temp.path().to_path_buf(), "chat-a", &process_id);

        let mut first = SpillWriter::create(&target).await.unwrap();
        first.write_line("old\n").await.unwrap();
        drop(first);

        let mut second = SpillWriter::create(&target).await.unwrap();
        second.write_line("new\n").await.unwrap();
        drop(second);

        assert_eq!(
            tokio::fs::read_to_string(target.path()).await.unwrap(),
            "new\n"
        );
    }
}
