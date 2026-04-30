use std::io::{Error, ErrorKind, Result};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use super::types::{Competitor, ImportKind, ImportSummary};

pub const IMPORTER_VERSION: &str = "competitor_import_v1";
const MANIFEST_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportManifest {
    pub version: u32,
    #[serde(default)]
    pub entries: Vec<ImportManifestEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_report: Option<ImportSummary>,
}

impl Default for ImportManifest {
    fn default() -> Self {
        Self {
            version: MANIFEST_VERSION,
            entries: Vec::new(),
            last_report: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportManifestEntry {
    pub competitor: Competitor,
    pub kind: ImportKind,
    pub source_path: PathBuf,
    pub source_hash: String,
    pub dest_path: PathBuf,
    pub dest_hash: String,
    pub importer_version: String,
    pub last_imported_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

impl ImportManifest {
    pub async fn read_from_path(path: &Path) -> Result<Self> {
        let content = match tokio::fs::read_to_string(path).await {
            Ok(content) => content,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(Self::default()),
            Err(err) => return Err(err),
        };
        let manifest: Self = serde_json::from_str(&content)
            .map_err(|err| Error::new(ErrorKind::InvalidData, err.to_string()))?;
        if manifest.version != MANIFEST_VERSION {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!("unsupported import manifest version {}", manifest.version),
            ));
        }
        Ok(manifest)
    }

    pub async fn write_to_path(&self, path: &Path) -> Result<()> {
        let content = serde_json::to_string_pretty(self)
            .map_err(|err| Error::new(ErrorKind::InvalidData, err.to_string()))?;
        write_string_atomic(path, &content).await
    }

    pub fn entry_for_dest(&self, dest_path: &Path) -> Option<&ImportManifestEntry> {
        self.entries
            .iter()
            .find(|entry| entry.dest_path == dest_path)
    }

    pub fn upsert_entry(&mut self, entry: ImportManifestEntry) {
        if let Some(existing) = self
            .entries
            .iter_mut()
            .find(|existing| existing.dest_path == entry.dest_path)
        {
            *existing = entry;
        } else {
            self.entries.push(entry);
            self.entries
                .sort_by(|left, right| left.dest_path.cmp(&right.dest_path));
        }
    }
}

pub fn manifest_path_for_scope_root(scope_root: &Path) -> PathBuf {
    scope_root.join("imports").join("competitors.json")
}

pub async fn write_last_report(scope_root: &Path, summary: &ImportSummary) -> Result<()> {
    let path = manifest_path_for_scope_root(scope_root);
    let mut manifest = ImportManifest::read_from_path(&path).await?;
    manifest.last_report = Some(summary.clone());
    manifest.write_to_path(&path).await
}

pub fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

pub fn hash_string(content: &str) -> String {
    hash_bytes(content.as_bytes())
}

pub fn hash_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(hash_bytes(&bytes))
}

pub fn hash_directory(path: &Path) -> Result<String> {
    let mut files = Vec::new();
    for entry in walkdir::WalkDir::new(path)
        .follow_links(false)
        .sort_by_file_name()
    {
        let entry = entry.map_err(|err| Error::new(ErrorKind::Other, err.to_string()))?;
        let entry_path = entry.path();
        if entry_path == path {
            continue;
        }
        let file_type = entry.file_type();
        if file_type.is_symlink() || file_type.is_dir() {
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let relative_path = entry_path
            .strip_prefix(path)
            .map_err(|err| Error::new(ErrorKind::InvalidData, err.to_string()))?
            .to_path_buf();
        files.push((relative_path, std::fs::read(entry_path)?));
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));

    let mut hasher = Sha256::new();
    for (relative_path, content) in files {
        let relative = relative_path.to_string_lossy().replace('\\', "/");
        hasher.update(relative.as_bytes());
        hasher.update([0]);
        hasher.update((content.len() as u64).to_le_bytes());
        hasher.update([0]);
        hasher.update(content);
        hasher.update([0]);
    }
    Ok(hex::encode(hasher.finalize()))
}

pub async fn write_string_atomic(path: &Path, content: &str) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    tokio::fs::create_dir_all(parent).await?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("import");
    let tmp_path = parent.join(format!(".{}.{}.tmp", file_name, uuid::Uuid::new_v4()));
    let write_result = async {
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)
            .await?;
        file.write_all(content.as_bytes()).await?;
        file.flush().await?;
        tokio::fs::rename(&tmp_path, path).await
    }
    .await;
    if write_result.is_err() {
        let _ = tokio::fs::remove_file(&tmp_path).await;
    }
    write_result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_and_bytes_hash_match() {
        assert_eq!(hash_string("abc"), hash_bytes(b"abc"));
    }

    #[tokio::test]
    async fn manifest_roundtrip_is_atomic_json() {
        let temp = tempfile::tempdir().unwrap();
        let path = manifest_path_for_scope_root(temp.path());
        let mut manifest = ImportManifest::default();
        manifest.entries.push(ImportManifestEntry {
            competitor: Competitor::ClaudeCode,
            kind: ImportKind::Command,
            source_path: PathBuf::from("/source/cmd.md"),
            source_hash: hash_string("source"),
            dest_path: PathBuf::from("/dest/cmd.md"),
            dest_hash: hash_string("dest"),
            importer_version: IMPORTER_VERSION.to_string(),
            last_imported_at: Utc::now(),
            metadata: Some(serde_json::json!({"original_name": "cmd"})),
        });

        manifest.write_to_path(&path).await.unwrap();
        let loaded = ImportManifest::read_from_path(&path).await.unwrap();

        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].source_hash, hash_string("source"));
    }

    #[test]
    fn directory_hash_skips_symlinks_and_is_deterministic() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("nested")).unwrap();
        std::fs::write(temp.path().join("b.txt"), "b").unwrap();
        std::fs::write(temp.path().join("nested").join("a.txt"), "a").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(temp.path().join("b.txt"), temp.path().join("link.txt"))
            .unwrap();

        let first = hash_directory(temp.path()).unwrap();
        let second = hash_directory(temp.path()).unwrap();

        assert_eq!(first, second);
        #[cfg(unix)]
        {
            let without_link = tempfile::tempdir().unwrap();
            std::fs::create_dir_all(without_link.path().join("nested")).unwrap();
            std::fs::write(without_link.path().join("b.txt"), "b").unwrap();
            std::fs::write(without_link.path().join("nested").join("a.txt"), "a").unwrap();
            assert_eq!(first, hash_directory(without_link.path()).unwrap());
        }
    }
}
