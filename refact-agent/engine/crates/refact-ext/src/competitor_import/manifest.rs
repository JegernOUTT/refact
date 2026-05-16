use std::io::{Error, ErrorKind, Read, Result};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use super::types::{Competitor, ImportKind, ImportReport, ImportSummary};

pub const IMPORTER_VERSION: &str = "competitor_import_v2";
pub const MAX_HASH_FILE_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_HASH_DIRECTORY_FILES: usize = 256;
pub const MAX_HASH_DIRECTORY_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_HASH_DIRECTORY_ENTRIES: usize = 1024;
pub const MAX_HASH_DIRECTORY_DEPTH: usize = 16;
pub const MAX_SCAN_MARKDOWN_FILES: usize = 256;
pub const MAX_SCAN_ENTRIES: usize = 4096;
pub const MAX_SCAN_DIRECT_CHILD_DIRS: usize = 512;
pub const MAX_SCAN_DEPTH: usize = 8;
pub const MAX_UNSUPPORTED_RULE_REPORTS: usize = 64;
const MANIFEST_VERSION: u32 = 1;
const HASH_BUFFER_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportManifest {
    pub version: u32,
    #[serde(default)]
    pub entries: Vec<ImportManifestEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_report: Option<ImportReport>,
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
    manifest.last_report = Some(ImportReport::from_summary(summary));
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
    let metadata = std::fs::symlink_metadata(path)?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() || !file_type.is_file() {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!("hash target is not a regular file: {}", path.display()),
        ));
    }
    if metadata.len() > MAX_HASH_FILE_BYTES {
        return Err(hash_limit_error(format!(
            "hash file exceeds {MAX_HASH_FILE_BYTES} byte limit: {} bytes",
            metadata.len()
        )));
    }

    let mut hasher = Sha256::new();
    let read = hash_file_content_into_limited(path, &mut hasher, MAX_HASH_FILE_BYTES)?;
    if read != metadata.len() {
        return Err(hash_limit_error(format!(
            "hash file changed while hashing: expected {} bytes, read {read} bytes",
            metadata.len()
        )));
    }
    Ok(hex::encode(hasher.finalize()))
}

pub fn hash_directory(path: &Path) -> Result<String> {
    let metadata = std::fs::symlink_metadata(path)?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() || !file_type.is_dir() {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!("hash target is not a regular directory: {}", path.display()),
        ));
    }

    let mut files = Vec::new();
    let mut file_count = 0usize;
    let mut entry_count = 0usize;
    let mut entries = walkdir::WalkDir::new(path)
        .follow_links(false)
        .sort_by_file_name()
        .max_depth(MAX_HASH_DIRECTORY_DEPTH + 1)
        .into_iter();
    while let Some(entry) = entries.next() {
        let entry = entry.map_err(|err| Error::new(ErrorKind::Other, err.to_string()))?;
        let entry_path = entry.path();
        if entry_path == path {
            continue;
        }
        entry_count += 1;
        if entry_count > MAX_HASH_DIRECTORY_ENTRIES {
            return Err(hash_limit_error(format!(
                "hash directory exceeds {MAX_HASH_DIRECTORY_ENTRIES} entry limit: {entry_count} entries"
            )));
        }
        if entry.depth() > MAX_HASH_DIRECTORY_DEPTH {
            if entry.file_type().is_dir() {
                entries.skip_current_dir();
            }
            return Err(hash_limit_error(format!(
                "hash directory exceeds {MAX_HASH_DIRECTORY_DEPTH} depth limit"
            )));
        }
        let metadata = std::fs::symlink_metadata(entry_path)?;
        let file_type = metadata.file_type();
        if file_type.is_symlink() || file_type.is_dir() || !file_type.is_file() {
            continue;
        }
        file_count += 1;
        if file_count > MAX_HASH_DIRECTORY_FILES {
            return Err(hash_limit_error(format!(
                "hash directory exceeds {MAX_HASH_DIRECTORY_FILES} file limit: {file_count} files"
            )));
        }
        let relative_path = entry_path
            .strip_prefix(path)
            .map_err(|err| Error::new(ErrorKind::InvalidData, err.to_string()))?
            .to_path_buf();
        files.push((relative_path, entry_path.to_path_buf(), metadata.len()));
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));

    let mut hasher = Sha256::new();
    let mut total_bytes = 0u64;
    for (relative_path, entry_path, len) in files {
        let next_total = total_bytes
            .checked_add(len)
            .ok_or_else(|| hash_limit_error("hash directory byte count overflow"))?;
        if next_total > MAX_HASH_DIRECTORY_BYTES {
            return Err(hash_limit_error(format!(
                "hash directory exceeds {MAX_HASH_DIRECTORY_BYTES} byte limit: {next_total} bytes"
            )));
        }
        let relative = relative_path.to_string_lossy().replace('\\', "/");
        hasher.update(relative.as_bytes());
        hasher.update([0]);
        hasher.update(len.to_le_bytes());
        hasher.update([0]);
        let remaining = MAX_HASH_DIRECTORY_BYTES - total_bytes;
        let read =
            hash_file_content_into_limited(&entry_path, &mut hasher, remaining).map_err(|err| {
                if is_hash_limit_error(&err) {
                    hash_limit_error(format!(
                    "hash directory exceeds {MAX_HASH_DIRECTORY_BYTES} byte limit while reading {}",
                    entry_path.display()
                ))
                } else {
                    err
                }
            })?;
        if read != len {
            return Err(hash_limit_error(format!(
                "hash directory file changed while hashing: {} expected {len} bytes, read {read} bytes",
                entry_path.display()
            )));
        }
        total_bytes = total_bytes
            .checked_add(read)
            .ok_or_else(|| hash_limit_error("hash directory byte count overflow"))?;
        if total_bytes > MAX_HASH_DIRECTORY_BYTES {
            return Err(hash_limit_error(format!(
                "hash directory exceeds {MAX_HASH_DIRECTORY_BYTES} byte limit: {total_bytes} bytes"
            )));
        }
        hasher.update([0]);
    }
    Ok(hex::encode(hasher.finalize()))
}

pub fn is_hash_limit_error(err: &Error) -> bool {
    err.kind() == ErrorKind::InvalidData && err.to_string().starts_with("hash ")
}

fn hash_file_content_into_limited(path: &Path, hasher: &mut Sha256, max_bytes: u64) -> Result<u64> {
    let mut file = std::fs::File::open(path)?;
    let mut buffer = [0u8; HASH_BUFFER_BYTES];
    let mut total = 0u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read as u64)
            .ok_or_else(|| hash_limit_error("hash file byte count overflow"))?;
        if total > max_bytes {
            return Err(hash_limit_error(format!(
                "hash file exceeds {max_bytes} byte limit while reading: {total} bytes"
            )));
        }
        hasher.update(&buffer[..read]);
    }
    Ok(total)
}

fn hash_limit_error(message: impl Into<String>) -> Error {
    Error::new(ErrorKind::InvalidData, message.into())
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
    fn hash_file_rejects_oversized_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("huge.bin");
        std::fs::File::create(&path)
            .unwrap()
            .set_len(MAX_HASH_FILE_BYTES + 1)
            .unwrap();

        let err = hash_file(&path).unwrap_err();

        assert!(is_hash_limit_error(&err));
        assert!(err.to_string().contains("exceeds"));
    }

    #[test]
    fn hash_file_content_limit_is_enforced_while_reading() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("growing.bin");
        std::fs::write(&path, vec![b'x'; 32]).unwrap();
        let mut hasher = Sha256::new();

        let err = hash_file_content_into_limited(&path, &mut hasher, 16).unwrap_err();

        assert!(is_hash_limit_error(&err));
        assert!(err.to_string().contains("while reading"));
    }

    #[test]
    fn hash_directory_rejects_file_count_and_byte_caps() {
        let too_many = tempfile::tempdir().unwrap();
        for index in 0..=MAX_HASH_DIRECTORY_FILES {
            std::fs::write(too_many.path().join(format!("file-{index}.txt")), "x").unwrap();
        }

        let err = hash_directory(too_many.path()).unwrap_err();

        assert!(is_hash_limit_error(&err));
        assert!(err.to_string().contains("file limit"));

        let too_large = tempfile::tempdir().unwrap();
        std::fs::File::create(too_large.path().join("huge.bin"))
            .unwrap()
            .set_len(MAX_HASH_DIRECTORY_BYTES + 1)
            .unwrap();

        let err = hash_directory(too_large.path()).unwrap_err();

        assert!(is_hash_limit_error(&err));
        assert!(err.to_string().contains("byte limit"));
    }

    #[test]
    fn hash_directory_rejects_entry_count_and_depth_caps() {
        let too_many_entries = tempfile::tempdir().unwrap();
        for index in 0..=MAX_HASH_DIRECTORY_ENTRIES {
            std::fs::create_dir(too_many_entries.path().join(format!("dir-{index}"))).unwrap();
        }

        let err = hash_directory(too_many_entries.path()).unwrap_err();

        assert!(is_hash_limit_error(&err));
        assert!(err.to_string().contains("entry limit"));

        let too_deep = tempfile::tempdir().unwrap();
        let mut current = too_deep.path().to_path_buf();
        for index in 0..=MAX_HASH_DIRECTORY_DEPTH {
            current = current.join(format!("d{index}"));
            std::fs::create_dir(&current).unwrap();
        }

        let err = hash_directory(too_deep.path()).unwrap_err();

        assert!(is_hash_limit_error(&err));
        assert!(err.to_string().contains("depth limit"));
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
