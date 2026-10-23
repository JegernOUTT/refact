use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Duration, Utc};
use tokio::fs;

use crate::types::{BackgroundAgent, BgAgentStatus};

mod migrations;

pub const SCHEMA_VERSION: u32 = 2;
const RECORDS_FILE: &str = "records.json";
const RESULTS_DIR: &str = "results";
pub const TERMINAL_RETENTION_DAYS: i64 = 7;
pub const MAX_RECORDS: usize = 2000;

static TMP_WRITE_COUNTER: AtomicU64 = AtomicU64::new(0);
const RENAME_ATTEMPTS: usize = 5;
const RENAME_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(50);

// A shared temp name makes concurrent savers steal each other's file: the first rename moves it
// away and the second fails with ENOENT.
fn tmp_path_for(dest_path: &Path) -> PathBuf {
    let file_name = dest_path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| RECORDS_FILE.to_string());
    let unique = TMP_WRITE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp_name = format!("{file_name}.{}.{unique}.tmp", std::process::id());
    match dest_path.parent() {
        Some(parent) => parent.join(tmp_name),
        None => PathBuf::from(tmp_name),
    }
}

// Windows rename already replaces the destination, so deleting it first only opened a window for
// a concurrent saver to lose the file and then be denied access to it.
async fn atomic_write_file(tmp_path: &Path, dest_path: &Path) -> Result<(), String> {
    let mut last_error = None;
    for attempt in 0..RENAME_ATTEMPTS {
        match fs::rename(tmp_path, dest_path).await {
            Ok(()) => return Ok(()),
            Err(e) if is_transient_rename_error(&e) && attempt + 1 < RENAME_ATTEMPTS => {
                last_error = Some(e);
                tokio::time::sleep(RENAME_RETRY_DELAY).await;
            }
            Err(e) => {
                last_error = Some(e);
                break;
            }
        }
    }
    let _ = fs::remove_file(tmp_path).await;
    let error = last_error
        .map(|e| e.to_string())
        .unwrap_or_else(|| "unknown rename failure".to_string());
    Err(format!(
        "Failed to rename {tmp_path:?} to {dest_path:?}: {error}"
    ))
}

fn is_transient_rename_error(error: &std::io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(5) | Some(32) | Some(33) | Some(1224)
    )
}

fn record_terminal_timestamp(record: &BackgroundAgent) -> Option<DateTime<Utc>> {
    let finished = match record.status {
        BgAgentStatus::Completed
        | BgAgentStatus::Failed
        | BgAgentStatus::Cancelled
        | BgAgentStatus::Interrupted
        | BgAgentStatus::Closed => record.finished_at.or(Some(record.last_update_at)),
        _ => None,
    };
    finished
}

fn prune_records(records: &mut Vec<BackgroundAgent>, now: DateTime<Utc>) {
    let cutoff = now - Duration::days(TERMINAL_RETENTION_DAYS);
    records.retain(|record| {
        if !record.status.is_terminal() || !record.pending_deliveries.is_empty() {
            return true;
        }
        record_terminal_timestamp(record)
            .map(|timestamp| timestamp >= cutoff)
            .unwrap_or(false)
    });
    if records.len() <= MAX_RECORDS {
        return;
    }
    records.sort_by(|a, b| {
        a.created_at
            .cmp(&b.created_at)
            .then(a.agent_id.cmp(&b.agent_id))
    });
    let mut excess = records.len().saturating_sub(MAX_RECORDS);
    let mut retained = Vec::with_capacity(records.len().saturating_sub(excess));
    for record in records.drain(..) {
        if excess > 0 && record.status.is_terminal() && record.pending_deliveries.is_empty() {
            excess -= 1;
            continue;
        }
        retained.push(record);
    }
    *records = retained;
}

pub async fn save_all(
    storage_root: &Path,
    mut records: Vec<BackgroundAgent>,
) -> Result<(), String> {
    for record in &mut records {
        match record.schema_version {
            1 | SCHEMA_VERSION => record.schema_version = SCHEMA_VERSION,
            version => {
                return Err(format!(
                    "Unsupported background agent schema version: {version}"
                ))
            }
        }
    }
    let before: Vec<String> = records.iter().map(|r| r.agent_id.clone()).collect();
    prune_records(&mut records, Utc::now());
    fs::create_dir_all(storage_root)
        .await
        .map_err(|e| format!("Failed to create background agents directory: {e}"))?;
    let records_path = storage_root.join(RECORDS_FILE);
    let tmp_path = tmp_path_for(&records_path);
    let content = serde_json::to_string_pretty(&records)
        .map_err(|e| format!("Failed to serialize background agents: {e}"))?;
    if let Err(e) = fs::write(&tmp_path, content).await {
        let _ = fs::remove_file(&tmp_path).await;
        return Err(format!("Failed to write background agents file: {e}"));
    }
    atomic_write_file(&tmp_path, &records_path).await?;
    let retained: std::collections::HashSet<&str> =
        records.iter().map(|r| r.agent_id.as_str()).collect();
    for pruned in before.iter().filter(|id| !retained.contains(id.as_str())) {
        remove_agent_files(storage_root, pruned).await;
    }
    Ok(())
}

/// Best-effort retention: once a record has been pruned from the durable
/// registry, its accepted launch, per-generation run configurations and result
/// payload are unreachable and are removed so terminal history stays bounded.
/// Only files of records pruned by this very save are touched; nothing is
/// inferred from directory listings of ids the registry still knows.
async fn remove_agent_files(storage_root: &Path, agent_id: &str) {
    let Ok(launch) = accepted_launch_path(storage_root, agent_id) else {
        return;
    };
    let _ = fs::remove_file(&launch).await;
    let _ = fs::remove_file(
        storage_root
            .join(RESULTS_DIR)
            .join(format!("{agent_id}.json")),
    )
    .await;
    let runs = storage_root.join("runs");
    let Ok(mut entries) = fs::read_dir(&runs).await else {
        return;
    };
    let prefix = format!("{agent_id}.");
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with(&prefix)
            && name.ends_with(".json")
            && name[prefix.len()..name.len() - ".json".len()]
                .parse::<u64>()
                .is_ok()
        {
            let _ = fs::remove_file(entry.path()).await;
        }
    }
}

pub async fn load_all(storage_root: &Path) -> Result<HashMap<String, BackgroundAgent>, String> {
    Ok(load_all_with_report(storage_root).await?.0)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuarantineReport {
    pub path: PathBuf,
    pub record_count: Option<usize>,
    pub first_reason: String,
}

impl std::fmt::Display for QuarantineReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let count = self
            .record_count
            .map(|count| count.to_string())
            .unwrap_or_else(|| "unknown (entire file)".into());
        write!(
            f,
            "WARNING: Background agent data quarantined at {}. Record count: {count}. First reason: {}",
            self.path.display(),
            self.first_reason
        )
    }
}

async fn preserve_quarantine(source: &Path, whole_file: bool) -> Result<PathBuf, String> {
    let directory = source.parent().ok_or("Quarantine directory unavailable")?;
    let mut timestamp = Utc::now();
    loop {
        let stamp = timestamp.format("%Y%m%dT%H%M%SZ");
        let name = if whole_file {
            format!("records.json.unreadable-{stamp}")
        } else {
            format!("records.unreadable-{stamp}.json")
        };
        let destination = directory.join(name);
        match fs::hard_link(source, &destination).await {
            Ok(()) => {
                fs::remove_file(source).await.map_err(|error| {
                    format!(
                        "Failed to finish quarantine move to {}: {error}",
                        destination.display()
                    )
                })?;
                return Ok(destination);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                timestamp += Duration::seconds(1);
            }
            Err(error) => {
                return Err(format!(
                    "Failed to quarantine background agents at {}: {error}",
                    destination.display()
                ));
            }
        }
    }
}

pub(crate) async fn load_all_with_report(
    storage_root: &Path,
) -> Result<(HashMap<String, BackgroundAgent>, Option<QuarantineReport>), String> {
    let records_path = storage_root.join(RECORDS_FILE);
    let content = match fs::read(&records_path).await {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((HashMap::new(), None));
        }
        Err(error) => return Err(format!("Failed to read background agents file: {error}")),
    };
    let original: Vec<serde_json::Value> = match serde_json::from_slice(&content) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            let report = QuarantineReport {
                path: preserve_quarantine(&records_path, true).await?,
                record_count: None,
                first_reason: format!("Failed to parse background agents file: {error}"),
            };
            tracing::error!("{report}");
            return Ok((HashMap::new(), Some(report)));
        }
    };
    let mut snapshot = Vec::new();
    let mut records = HashMap::new();
    let mut unreadable = Vec::new();
    let mut migrated = false;
    for original in original {
        let mut value = original.clone();
        let loaded = (|| {
            let changed = migrations::migrate(std::slice::from_mut(&mut value))?;
            let record: BackgroundAgent = serde_json::from_value(value.clone())
                .map_err(|e| format!("Failed to parse background agents file: {e}"))?;
            for question in &record.questions {
                question.validate().map_err(|e| {
                    format!("Invalid background agent question {}: {e}", question.id)
                })?;
            }
            Ok::<_, String>((record, changed))
        })();
        match loaded {
            Ok((record, changed)) => {
                migrated |= changed;
                snapshot.push(value);
                records.insert(record.agent_id.clone(), record);
            }
            Err(reason) => {
                unreadable.push(serde_json::json!({"reason": reason, "record": original}))
            }
        }
    }
    let report = if unreadable.is_empty() {
        None
    } else {
        let bytes = serde_json::to_vec_pretty(&unreadable)
            .map_err(|error| format!("Failed to serialize background agent quarantine: {error}"))?;
        let tmp = tmp_path_for(&records_path);
        let staging = LaunchStaging(tmp.clone());
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .await
            .map_err(|error| format!("Failed to create background agent quarantine: {error}"))?;
        use tokio::io::AsyncWriteExt;
        file.write_all(&bytes)
            .await
            .map_err(|error| format!("Failed to write background agent quarantine: {error}"))?;
        file.sync_all()
            .await
            .map_err(|error| format!("Failed to flush background agent quarantine: {error}"))?;
        drop(file);
        let report = QuarantineReport {
            path: preserve_quarantine(&tmp, false).await?,
            record_count: Some(unreadable.len()),
            first_reason: unreadable[0]["reason"].as_str().unwrap().to_string(),
        };
        drop(staging);
        tracing::error!("{report}");
        Some(report)
    };
    if migrated || report.is_some() {
        let content = serde_json::to_vec_pretty(&snapshot)
            .map_err(|e| format!("Failed to serialize migrated background agents: {e}"))?;
        let tmp_path = tmp_path_for(&records_path);
        if let Err(e) = fs::write(&tmp_path, content).await {
            let _ = fs::remove_file(&tmp_path).await;
            return Err(format!(
                "Failed to write migrated background agents file: {e}"
            ));
        }
        atomic_write_file(&tmp_path, &records_path).await?;
    }
    Ok((records, report))
}

pub async fn save_record(storage_root: &Path, record: &BackgroundAgent) -> Result<(), String> {
    let mut records = load_all(storage_root).await?;
    records.insert(record.agent_id.clone(), record.clone());
    let mut ordered: Vec<BackgroundAgent> = records.into_values().collect();
    ordered.sort_by(|a, b| {
        a.created_at
            .cmp(&b.created_at)
            .then(a.agent_id.cmp(&b.agent_id))
    });
    save_all(storage_root, ordered).await
}

fn accepted_launch_path(storage_root: &Path, agent_id: &str) -> Result<PathBuf, String> {
    let id = agent_id
        .strip_prefix("bgagent-")
        .ok_or("invalid agent ID")?;
    uuid::Uuid::parse_str(id).map_err(|_| "invalid agent ID")?;
    Ok(storage_root
        .join("launches")
        .join(format!("{agent_id}.json")))
}

#[derive(Debug)]
pub struct AcceptedLaunchPayload {
    bytes: Vec<u8>,
}

struct LaunchStaging(PathBuf);

impl Drop for LaunchStaging {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

impl AcceptedLaunchPayload {
    pub fn serialize(value: &impl serde::Serialize, max_bytes: usize) -> Result<Self, String> {
        struct BoundWriter {
            bytes: Vec<u8>,
            limit: usize,
        }
        impl std::io::Write for BoundWriter {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
                    return Err(std::io::Error::other("accepted launch byte bound"));
                }
                let needed = self.bytes.len() + bytes.len();
                if needed > self.bytes.capacity() {
                    let capacity = needed
                        .max(self.bytes.capacity().saturating_mul(2))
                        .min(self.limit);
                    self.bytes
                        .try_reserve_exact(capacity - self.bytes.len())
                        .map_err(|_| std::io::Error::other("accepted launch allocation failed"))?;
                }
                self.bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut writer = BoundWriter {
            bytes: Vec::new(),
            limit: max_bytes,
        };
        serde_json::to_writer(&mut writer, value)
            .map_err(|_| "accepted launch exceeds configured byte bound or cannot be serialized")?;
        Ok(Self {
            bytes: writer.bytes,
        })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn decode<T: serde::de::DeserializeOwned>(&self) -> Result<T, String> {
        serde_json::from_slice(&self.bytes).map_err(|_| "accepted launch invalid".into())
    }
}

pub(crate) async fn save_accepted_launch(
    storage_root: &Path,
    agent_id: &str,
    payload: &AcceptedLaunchPayload,
) -> Result<(), String> {
    let path = accepted_launch_path(storage_root, agent_id)?;
    save_launch_payload(&path, payload).await
}

fn run_config_path(
    storage_root: &Path,
    agent_id: &str,
    generation: u64,
) -> Result<PathBuf, String> {
    accepted_launch_path(storage_root, agent_id)?;
    if generation == 0 {
        return Err("invalid run generation".into());
    }
    Ok(storage_root
        .join("runs")
        .join(format!("{agent_id}.{generation}.json")))
}

pub(crate) async fn save_run_config(
    storage_root: &Path,
    agent_id: &str,
    generation: u64,
    payload: &AcceptedLaunchPayload,
) -> Result<(), String> {
    save_launch_payload(
        &run_config_path(storage_root, agent_id, generation)?,
        payload,
    )
    .await
}

pub(crate) async fn load_run_config(
    storage_root: &Path,
    agent_id: &str,
    generation: u64,
    max_bytes: usize,
) -> Result<AcceptedLaunchPayload, String> {
    load_launch_payload(
        &run_config_path(storage_root, agent_id, generation)?,
        max_bytes,
    )
    .await
}

async fn save_launch_payload(path: &Path, payload: &AcceptedLaunchPayload) -> Result<(), String> {
    use tokio::io::AsyncWriteExt;
    let directory = path.parent().ok_or("launch directory unavailable")?;
    fs::create_dir_all(directory)
        .await
        .map_err(|_| "launch directory unavailable")?;
    let directory_meta = fs::symlink_metadata(directory)
        .await
        .map_err(|_| "launch directory unavailable")?;
    if !directory_meta.is_dir() || directory_meta.file_type().is_symlink() {
        return Err("launch directory is not a regular directory".into());
    }
    let tmp = tmp_path_for(&path);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(&tmp)
        .map_err(|_| "launch staging unavailable")?;
    let staging = LaunchStaging(tmp.clone());
    let mut file = fs::File::from_std(file);
    let result = async {
        file.write_all(payload.as_bytes())
            .await
            .map_err(|_| "accepted launch write failed")?;
        file.sync_all()
            .await
            .map_err(|_| "accepted launch flush failed")?;
        fs::hard_link(&tmp, &path)
            .await
            .map_err(|_| "accepted launch already exists or cannot be committed")?;
        #[cfg(unix)]
        {
            let mut options = fs::OpenOptions::new();
            options
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW);
            options
                .open(directory)
                .await
                .map_err(|_| "launch directory flush unavailable")?
                .sync_all()
                .await
                .map_err(|_| "launch directory flush failed")?;
        }
        Ok(())
    }
    .await;
    drop(file);
    drop(staging);
    result
}

pub(crate) async fn load_accepted_launch(
    storage_root: &Path,
    agent_id: &str,
    max_bytes: usize,
) -> Result<AcceptedLaunchPayload, String> {
    let path = accepted_launch_path(storage_root, agent_id)?;
    load_launch_payload(&path, max_bytes).await
}

async fn load_launch_payload(
    path: &Path,
    max_bytes: usize,
) -> Result<AcceptedLaunchPayload, String> {
    use tokio::io::AsyncReadExt;
    let directory_meta = fs::symlink_metadata(path.parent().ok_or("launch directory unavailable")?)
        .await
        .map_err(|_| "launch directory unavailable")?;
    if !directory_meta.is_dir() || directory_meta.file_type().is_symlink() {
        return Err("launch directory is not a regular directory".into());
    }
    let meta = fs::symlink_metadata(&path)
        .await
        .map_err(|_| "accepted launch unavailable")?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err("accepted launch is not a regular file".into());
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    let file = options
        .open(&path)
        .await
        .map_err(|_| "accepted launch unavailable")?;
    let opened = file
        .metadata()
        .await
        .map_err(|_| "accepted launch unavailable")?;
    if !opened.is_file() || opened.len() > max_bytes as u64 {
        return Err("accepted launch exceeds configured bound or is not regular".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.dev() != opened.dev() || meta.ino() != opened.ino() {
            return Err("accepted launch changed while opening".into());
        }
    }
    let mut bytes = Vec::new();
    let mut file = file.take(max_bytes.saturating_add(1) as u64);
    file.read_to_end(&mut bytes)
        .await
        .map_err(|_| "accepted launch read failed")?;
    if bytes.len() > max_bytes {
        return Err("accepted launch exceeds configured byte bound".into());
    }
    let after = file
        .get_ref()
        .metadata()
        .await
        .map_err(|_| "accepted launch unavailable")?;
    let current = fs::symlink_metadata(&path)
        .await
        .map_err(|_| "accepted launch unavailable")?;
    if !current.is_file()
        || current.len() != opened.len()
        || after.len() != opened.len()
        || after.modified().ok() != opened.modified().ok()
        || current.modified().ok() != opened.modified().ok()
    {
        return Err("accepted launch changed while reading".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if current.dev() != opened.dev()
            || current.ino() != opened.ino()
            || after.ctime() != opened.ctime()
            || after.ctime_nsec() != opened.ctime_nsec()
        {
            return Err("accepted launch changed while reading".into());
        }
    }
    Ok(AcceptedLaunchPayload { bytes })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn run_config_is_generation_bound_create_only_and_separate_from_launch() {
        let temp = tempfile::tempdir().unwrap();
        let id = format!("bgagent-{}", uuid::Uuid::new_v4());
        let first =
            AcceptedLaunchPayload::serialize(&serde_json::json!({"generation":1}), 128).unwrap();
        let second =
            AcceptedLaunchPayload::serialize(&serde_json::json!({"generation":2}), 128).unwrap();
        save_accepted_launch(temp.path(), &id, &first)
            .await
            .unwrap();
        save_run_config(temp.path(), &id, 1, &first).await.unwrap();
        save_run_config(temp.path(), &id, 2, &second).await.unwrap();
        assert!(save_run_config(temp.path(), &id, 1, &second).await.is_err());
        assert!(save_run_config(temp.path(), &id, 0, &first).await.is_err());
        assert!(load_run_config(temp.path(), &id, 3, 128).await.is_err());
        assert!(load_run_config(temp.path(), &id, 1, 1).await.is_err());
        assert_eq!(
            load_run_config(temp.path(), &id, 1, 128)
                .await
                .unwrap()
                .as_bytes(),
            first.as_bytes()
        );
        assert_eq!(
            load_run_config(temp.path(), &id, 2, 128)
                .await
                .unwrap()
                .as_bytes(),
            second.as_bytes()
        );
        assert_eq!(
            load_accepted_launch(temp.path(), &id, 128)
                .await
                .unwrap()
                .as_bytes(),
            first.as_bytes()
        );
    }

    #[tokio::test]
    async fn accepted_launch_is_create_only_bounded_and_recoverable() {
        let temp = tempfile::tempdir().unwrap();
        let id = format!("bgagent-{}", uuid::Uuid::new_v4());
        let payload = serde_json::json!({"revision":1,"source":"first"});
        let cap = serde_json::to_vec(&payload).unwrap().len();
        assert!(AcceptedLaunchPayload::serialize(&payload, cap - 1).is_err());
        let encoded = AcceptedLaunchPayload::serialize(&payload, cap).unwrap();
        save_accepted_launch(temp.path(), &id, &encoded)
            .await
            .unwrap();
        let replacement =
            AcceptedLaunchPayload::serialize(&serde_json::json!({"revision":2}), cap).unwrap();
        assert!(save_accepted_launch(temp.path(), &id, &replacement)
            .await
            .is_err());
        assert_eq!(
            load_accepted_launch(temp.path(), &id, cap)
                .await
                .unwrap()
                .decode::<serde_json::Value>()
                .unwrap(),
            payload
        );
        assert!(load_accepted_launch(temp.path(), &id, cap - 1)
            .await
            .is_err());
        assert!(save_accepted_launch(temp.path(), "../escape", &encoded)
            .await
            .is_err());
    }

    #[test]
    fn accepted_payload_serialization_stops_before_unbounded_input_is_traversed() {
        struct LargeSequence(std::cell::Cell<usize>);
        impl serde::Serialize for LargeSequence {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                use serde::ser::SerializeSeq;
                let mut seq = serializer.serialize_seq(None)?;
                for _ in 0..1_000_000 {
                    self.0.set(self.0.get() + 1);
                    seq.serialize_element("bounded")?;
                }
                seq.end()
            }
        }
        let input = LargeSequence(std::cell::Cell::new(0));
        assert!(AcceptedLaunchPayload::serialize(&input, 128).is_err());
        assert!(input.0.get() < 20);
    }

    #[tokio::test]
    async fn concurrent_accepted_launches_publish_one_complete_payload_and_clean_staging() {
        let temp = tempfile::tempdir().unwrap();
        let id = format!("bgagent-{}", uuid::Uuid::new_v4());
        let first =
            AcceptedLaunchPayload::serialize(&serde_json::json!({"source":"first"}), 128).unwrap();
        let second =
            AcceptedLaunchPayload::serialize(&serde_json::json!({"source":"second"}), 128).unwrap();
        let (a, b) = tokio::join!(
            save_accepted_launch(temp.path(), &id, &first),
            save_accepted_launch(temp.path(), &id, &second),
        );
        assert_ne!(a.is_ok(), b.is_ok());
        let loaded = load_accepted_launch(temp.path(), &id, 128).await.unwrap();
        assert_eq!(
            loaded.as_bytes(),
            if a.is_ok() {
                first.as_bytes()
            } else {
                second.as_bytes()
            }
        );
        assert_eq!(
            std::fs::read_dir(temp.path().join("launches"))
                .unwrap()
                .count(),
            1
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(accepted_launch_path(temp.path(), &id).unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn accepted_launch_rejects_symlink_directory_and_nonregular_records() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let foreign = tempfile::tempdir().unwrap();
        let id = format!("bgagent-{}", uuid::Uuid::new_v4());
        let payload =
            AcceptedLaunchPayload::serialize(&serde_json::json!({"source":"first"}), 128).unwrap();
        symlink(foreign.path(), temp.path().join("launches")).unwrap();
        assert!(save_accepted_launch(temp.path(), &id, &payload)
            .await
            .is_err());
        assert_eq!(std::fs::read_dir(foreign.path()).unwrap().count(), 0);
        std::fs::remove_file(temp.path().join("launches")).unwrap();
        std::fs::create_dir(temp.path().join("launches")).unwrap();
        let path = accepted_launch_path(temp.path(), &id).unwrap();
        symlink(foreign.path().join("missing"), &path).unwrap();
        assert!(save_accepted_launch(temp.path(), &id, &payload)
            .await
            .is_err());
        assert!(load_accepted_launch(temp.path(), &id, 128).await.is_err());
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(load_accepted_launch(temp.path(), &id, 128).await.is_err());
        assert_eq!(
            std::fs::read_dir(temp.path().join("launches"))
                .unwrap()
                .count(),
            1
        );
    }

    fn make_record(
        agent_id: &str,
        status: BgAgentStatus,
        created_at: DateTime<Utc>,
        finished_at: Option<DateTime<Utc>>,
    ) -> BackgroundAgent {
        BackgroundAgent {
            schema_version: SCHEMA_VERSION,
            run_generation: 1,
            stop_hook_generation: None,
            agent_id: agent_id.to_string(),
            parent_chat_id: "parent".to_string(),
            parent_root_chat_id: None,
            parent_tool_call_id: None,
            child_chat_id: None,
            kind: crate::types::BgAgentKind::Subagent,
            config_name: "cfg".to_string(),
            title: "title".to_string(),
            prompt: "prompt".to_string(),
            target_files: Vec::new(),
            status,
            progress: None,
            step_count: 0,
            last_activity: None,
            result_summary: None,
            result_payload_path: None,
            error: None,
            edited_files: Vec::new(),
            diff_summary: None,
            conflict_summary: None,
            completion_message_id: None,
            completion_pushed_at: None,
            deferred_at: None,
            close_reason: None,
            previous_runs: Vec::new(),
            completion_push: Default::default(),
            pending_deliveries: Vec::new(),
            delivery_ids: Vec::new(),
            dropped_deliveries: Vec::new(),
            model: "model".to_string(),
            model_type: None,
            current_tool: None,
            goal_summary: None,
            plan_present: false,
            worktree_id: None,
            worktree_branch: None,
            merge_status: None,
            questions: Vec::new(),
            tokens_used: 0,
            cost_usd: None,
            deliverable: None,
            deliverable_status: None,
            profile_name: None,
            handoff: None,
            created_at,
            started_at: None,
            finished_at,
            last_update_at: finished_at.unwrap_or(created_at),
            change_seq: 1,
        }
    }

    #[tokio::test]
    async fn load_migrates_v1_questions_and_persists_without_pruning() {
        let temp = tempfile::tempdir().unwrap();
        let now = Utc::now();
        let mut legacy = serde_json::to_value(make_record(
            "bgagent-legacy",
            BgAgentStatus::Completed,
            now - Duration::days(30),
            Some(now - Duration::days(20)),
        ))
        .unwrap();
        legacy["schema_version"] = serde_json::json!(1);
        legacy["questions"] = serde_json::json!([
            {
                "id": "answered",
                "text": "Which test?",
                "asked_at": now - Duration::days(21),
                "answer": "Run the unit tests",
                "answered_at": now - Duration::days(20)
            },
            {
                "id": "pending",
                "text": "Continue?",
                "asked_at": now,
                "answer": null,
                "answered_at": null
            }
        ]);
        legacy["extra_metadata"] = serde_json::json!({"keep": true});
        let mut modern = make_record("bgagent-modern", BgAgentStatus::Completed, now, Some(now));
        modern.schema_version = 1;
        modern.questions.push(
            refact_chat_api::QuestionContract::pending(
                "modern-question",
                refact_chat_api::QuestionKind::AgentParent,
                "bgagent-modern",
                "parent",
                "bgagent-modern:modern-question",
                "Modern question",
                None,
                now,
                None,
            )
            .unwrap(),
        );
        legacy["questions"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::to_value(&modern.questions[0]).unwrap());
        let current = make_record("bgagent-current", BgAgentStatus::Completed, now, Some(now));
        let mut no_questions = serde_json::to_value(&modern).unwrap();
        no_questions["agent_id"] = serde_json::json!("bgagent-no-questions");
        no_questions.as_object_mut().unwrap().remove("questions");
        let original = serde_json::json!([legacy, modern, current, no_questions]);
        let path = temp.path().join(RECORDS_FILE);
        fs::write(&path, serde_json::to_vec(&original).unwrap())
            .await
            .unwrap();

        let loaded = load_all(temp.path()).await.unwrap();

        assert_eq!(loaded.len(), 4);
        let record = &loaded["bgagent-legacy"];
        assert_eq!(record.schema_version, 2);
        assert_eq!(record.status, BgAgentStatus::Completed);
        let answered = &record.questions[0];
        assert_eq!(answered.kind, refact_chat_api::QuestionKind::AgentParent);
        assert_eq!(answered.asker, record.agent_id);
        assert_eq!(answered.answerer, record.parent_chat_id);
        assert_eq!(answered.correlation_id, "bgagent-legacy:answered");
        assert_eq!(answered.state, refact_chat_api::QuestionState::Answered);
        assert_eq!(answered.answered(), Some("Run the unit tests"));
        assert_eq!(answered.asked_at, now - Duration::days(21));
        assert_eq!(answered.answered_at, Some(now - Duration::days(20)));
        let pending = &record.questions[1];
        assert_eq!(pending.state, refact_chat_api::QuestionState::Pending);
        assert_eq!(pending.correlation_id, "bgagent-legacy:pending");
        assert!(pending.answer.is_none());
        assert!(pending.answered_at.is_none());
        assert!(pending.expires_at.is_none());
        assert!(pending.default_answer.is_none());
        assert_eq!(loaded["bgagent-modern"].questions, modern.questions);
        assert_eq!(loaded["bgagent-legacy"].questions[2], modern.questions[0]);
        assert_eq!(loaded["bgagent-current"], current);
        assert!(loaded["bgagent-no-questions"].questions.is_empty());

        let persisted = fs::read(&path).await.unwrap();
        let upgraded: serde_json::Value = serde_json::from_slice(&persisted).unwrap();
        for (before, after) in original
            .as_array()
            .unwrap()
            .iter()
            .zip(upgraded.as_array().unwrap())
        {
            assert_eq!(after["schema_version"], 2);
            for (key, value) in before.as_object().unwrap() {
                if key != "schema_version" && key != "questions" {
                    assert_eq!(&after[key], value, "changed field {key}");
                }
            }
        }
        assert_eq!(upgraded[1]["questions"], original[1]["questions"]);
        for (before, after) in original[0]["questions"]
            .as_array()
            .unwrap()
            .iter()
            .zip(upgraded[0]["questions"].as_array().unwrap())
        {
            for (key, value) in before.as_object().unwrap() {
                assert_eq!(&after[key], value, "changed question field {key}");
            }
        }
        assert_eq!(load_all(temp.path()).await.unwrap(), loaded);
        assert_eq!(fs::read(&path).await.unwrap(), persisted);
    }

    #[tokio::test]
    async fn current_snapshot_is_not_rewritten_and_new_writes_use_current_version() {
        let temp = tempfile::tempdir().unwrap();
        let now = Utc::now();
        let record = make_record("current", BgAgentStatus::Completed, now, Some(now));
        let path = temp.path().join(RECORDS_FILE);
        let original = serde_json::to_vec(&vec![record.clone()]).unwrap();
        fs::write(&path, &original).await.unwrap();
        let modified = fs::metadata(&path).await.unwrap().modified().unwrap();

        assert_eq!(load_all(temp.path()).await.unwrap()["current"], record);
        assert_eq!(fs::read(&path).await.unwrap(), original);
        assert_eq!(
            fs::metadata(&path).await.unwrap().modified().unwrap(),
            modified
        );

        let mut old_version = record.clone();
        old_version.schema_version = 1;
        save_all(temp.path(), vec![old_version]).await.unwrap();
        let written: Vec<BackgroundAgent> =
            serde_json::from_slice(&fs::read(&path).await.unwrap()).unwrap();
        assert_eq!(written, vec![record]);
    }

    #[tokio::test]
    async fn invalid_records_are_preserved_without_partially_migrating_them() {
        let temp = tempfile::tempdir().unwrap();
        let now = Utc::now();
        let mut valid = serde_json::to_value(make_record(
            "valid",
            BgAgentStatus::Completed,
            now,
            Some(now),
        ))
        .unwrap();
        valid["schema_version"] = serde_json::json!(1);
        let mut legacy = valid.clone();
        legacy["agent_id"] = serde_json::json!("invalid");
        legacy["questions"] = serde_json::json!([{
            "id": "question",
            "text": "Question?",
            "asked_at": now,
            "answer": null,
            "answered_at": null
        }]);
        let mut partial = legacy.clone();
        partial["questions"][0]["asker"] = serde_json::json!("invalid");
        let mut unknown_kind = legacy.clone();
        unknown_kind["questions"][0]["kind"] = serde_json::json!("unknown");
        let mut unanswered_timestamp = legacy.clone();
        unanswered_timestamp["questions"][0]["answered_at"] = serde_json::json!(now);
        let mut answer_without_timestamp = legacy.clone();
        answer_without_timestamp["questions"][0]["answer"] = serde_json::json!("yes");
        let mut v2_legacy_question = legacy.clone();
        v2_legacy_question["schema_version"] = serde_json::json!(SCHEMA_VERSION);
        let mut invalid_agent = valid.clone();
        invalid_agent.as_object_mut().unwrap().remove("model");
        let mut future = valid.clone();
        future["schema_version"] = serde_json::json!(SCHEMA_VERSION + 1);
        let mut missing_version = valid.clone();
        missing_version
            .as_object_mut()
            .unwrap()
            .remove("schema_version");
        let mut invalid_modern_question = legacy.clone();
        invalid_modern_question["questions"][0] = serde_json::json!({
            "id": "question", "text": "Question?", "asked_at": now,
            "kind": "agent_parent", "asker": "invalid", "answerer": "parent",
            "correlation_id": "invalid:question", "state": "answered"
        });
        let mut unknown_legacy_field = legacy;
        unknown_legacy_field["questions"][0]["irreversible"] = serde_json::json!(true);
        let path = temp.path().join(RECORDS_FILE);
        for invalid in [
            partial,
            unknown_kind,
            unanswered_timestamp,
            answer_without_timestamp,
            v2_legacy_question,
            invalid_agent,
            future,
            missing_version,
            invalid_modern_question,
            unknown_legacy_field,
        ] {
            let original = serde_json::to_vec(&serde_json::json!([valid, invalid])).unwrap();
            fs::write(&path, &original).await.unwrap();
            let (loaded, report) = load_all_with_report(temp.path()).await.unwrap();
            assert_eq!(loaded.len(), 1, "accepted {invalid}");
            assert_eq!(loaded["valid"].schema_version, SCHEMA_VERSION);
            let report = report.unwrap();
            assert_eq!(report.record_count, Some(1));
            let quarantined: serde_json::Value =
                serde_json::from_slice(&fs::read(report.path).await.unwrap()).unwrap();
            assert_eq!(quarantined[0]["record"], invalid);
            assert!(!quarantined[0]["reason"].as_str().unwrap().is_empty());
        }
        fs::write(&path, b"[{invalid").await.unwrap();
        let (loaded, report) = load_all_with_report(temp.path()).await.unwrap();
        assert!(loaded.is_empty());
        assert_eq!(fs::read(report.unwrap().path).await.unwrap(), b"[{invalid");
    }

    #[tokio::test]
    async fn registry_restores_migrated_questions_and_interrupts_without_replay() {
        let temp = tempfile::tempdir().unwrap();
        let now = Utc::now();
        let mut legacy = serde_json::to_value(make_record(
            "bgagent-restored",
            BgAgentStatus::Running,
            now,
            None,
        ))
        .unwrap();
        legacy["schema_version"] = serde_json::json!(1);
        legacy["questions"] = serde_json::json!([{
            "id": "restored-question",
            "text": "Continue?",
            "asked_at": now,
            "answer": null,
            "answered_at": null
        }]);
        fs::write(
            temp.path().join(RECORDS_FILE),
            serde_json::to_vec(&vec![legacy]).unwrap(),
        )
        .await
        .unwrap();

        let registry = crate::BackgroundAgentRegistry::new(temp.path().to_path_buf())
            .await
            .unwrap();
        let record = registry.get_any("bgagent-restored").await.unwrap();
        assert_eq!(record.schema_version, SCHEMA_VERSION);
        assert_eq!(record.status, BgAgentStatus::Interrupted);
        assert!(!registry.has_runtime("bgagent-restored").await);
        assert_eq!(
            load_all(temp.path()).await.unwrap()["bgagent-restored"],
            record
        );
        let question = registry.questions().get("restored-question").unwrap();
        assert_eq!(question, record.questions[0]);
        assert_eq!(question.state, refact_chat_api::QuestionState::Pending);
        assert!(registry
            .answer_question("bgagent-restored", "restored-question", "Yes".into())
            .await
            .is_ok());
        let answered = registry.get_any("bgagent-restored").await.unwrap();
        assert_eq!(answered.questions[0].answered(), Some("Yes"));
    }

    #[tokio::test]
    async fn every_current_question_state_round_trips_through_storage() {
        use refact_chat_api::{
            AskQuestion, QuestionKind, QuestionLimits, QuestionRegistry, QuestionState,
        };

        let temp = tempfile::tempdir().unwrap();
        let now = Utc::now();
        let mut record = make_record("current-states", BgAgentStatus::Completed, now, Some(now));
        for kind in [
            QuestionKind::UserInput,
            QuestionKind::AgentParent,
            QuestionKind::BoardPlanner,
            QuestionKind::PlanApproval,
            QuestionKind::PermissionEscalation,
        ] {
            for irreversible in [false, true] {
                for default in [None, Some("skip")] {
                    for state in [
                        QuestionState::Pending,
                        QuestionState::Answered,
                        QuestionState::Defaulted,
                        QuestionState::Rejected,
                        QuestionState::Cancelled,
                        QuestionState::Expired,
                    ] {
                        let service = QuestionRegistry::new(QuestionLimits::default()).unwrap();
                        let id = format!("{kind:?}-{irreversible}-{default:?}-{state:?}");
                        let request =
                            AskQuestion::new(kind, &record.agent_id, "parent", &id, "Continue?")
                                .with_id(&id)
                                .with_options(Some(vec!["yes".into(), "skip".into()]))
                                .with_expiry_secs(Some(60))
                                .irreversible(irreversible)
                                .with_default_answer(default.map(str::to_string));
                        if default.is_some()
                            && (irreversible
                                || matches!(
                                    kind,
                                    QuestionKind::PlanApproval | QuestionKind::PermissionEscalation
                                ))
                        {
                            assert!(service.ask(request, now).is_err());
                            continue;
                        }
                        if (state == QuestionState::Defaulted && default.is_none())
                            || (state == QuestionState::Expired && default.is_some())
                        {
                            continue;
                        }
                        let question = service.ask(request, now).unwrap();
                        let settled = match state {
                            QuestionState::Pending => question,
                            QuestionState::Answered => service
                                .answer(&id, "parent", "yes".into(), now + Duration::seconds(1))
                                .unwrap(),
                            QuestionState::Rejected => service.reject(&id, "parent", now).unwrap(),
                            QuestionState::Cancelled => {
                                service.cancel(&id, &record.agent_id, now).unwrap()
                            }
                            QuestionState::Defaulted | QuestionState::Expired => {
                                service.expire(now + Duration::seconds(61));
                                service.get(&id).unwrap()
                            }
                        };
                        assert_eq!(settled.state, state);
                        settled.validate().unwrap();
                        record.questions.push(settled);
                    }
                }
            }
        }
        assert_eq!(record.questions.len(), 65);
        for version in [1, SCHEMA_VERSION] {
            let mut to_save = record.clone();
            to_save.schema_version = version;
            save_all(temp.path(), vec![to_save]).await.unwrap();
            let path = temp.path().join(RECORDS_FILE);
            let bytes = fs::read(&path).await.unwrap();
            let modified = fs::metadata(&path).await.unwrap().modified().unwrap();
            assert_eq!(
                load_all(temp.path()).await.unwrap()[&record.agent_id],
                record
            );
            let restarted = crate::BackgroundAgentRegistry::new(temp.path().to_path_buf())
                .await
                .unwrap();
            assert_eq!(restarted.get_any(&record.agent_id).await.unwrap(), record);
            for question in &record.questions {
                assert_eq!(
                    restarted.questions().get(&question.id).as_ref(),
                    Some(question)
                );
            }
            assert_eq!(fs::read(&path).await.unwrap(), bytes);
            assert_eq!(
                fs::metadata(&path).await.unwrap().modified().unwrap(),
                modified
            );
        }
    }

    #[tokio::test]
    async fn unsupported_save_version_preserves_existing_records_and_creates_no_staging() {
        let temp = tempfile::tempdir().unwrap();
        let now = Utc::now();
        let record = make_record("saved", BgAgentStatus::Completed, now, Some(now));
        save_all(temp.path(), vec![record.clone()]).await.unwrap();
        let path = temp.path().join(RECORDS_FILE);
        let bytes = fs::read(&path).await.unwrap();
        for version in [0, SCHEMA_VERSION + 1, u32::MAX] {
            let mut unsupported = record.clone();
            unsupported.schema_version = version;
            assert_eq!(
                save_all(temp.path(), vec![unsupported]).await.unwrap_err(),
                format!("Unsupported background agent schema version: {version}")
            );
            assert_eq!(fs::read(&path).await.unwrap(), bytes);
            assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
        }
        assert_eq!(load_all(temp.path()).await.unwrap()["saved"], record);
    }

    #[test]
    fn prune_records_preserves_unacknowledged_delivery_payloads() {
        let now = Utc::now();
        let mut record = make_record(
            "pending",
            BgAgentStatus::Interrupted,
            now - Duration::days(30),
            Some(now - Duration::days(20)),
        );
        record
            .pending_deliveries
            .push(refact_chat_api::PendingDelivery::new(
                Vec::new(),
                refact_chat_api::PushMode::Append,
                "test",
                false,
            ));
        let mut records = vec![record];
        prune_records(&mut records, now);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].pending_deliveries.len(), 1);
    }

    #[tokio::test]
    async fn pruned_records_drop_their_launch_run_and_result_files_only() {
        let temp = tempfile::tempdir().unwrap();
        let now = Utc::now();
        let live = format!("bgagent-{}", uuid::Uuid::new_v4());
        let old = format!("bgagent-{}", uuid::Uuid::new_v4());
        let payload =
            AcceptedLaunchPayload::serialize(&serde_json::json!({"seed":1}), 128).unwrap();
        for id in [&live, &old] {
            save_accepted_launch(temp.path(), id, &payload)
                .await
                .unwrap();
            save_run_config(temp.path(), id, 1, &payload).await.unwrap();
            save_run_config(temp.path(), id, 2, &payload).await.unwrap();
            save_result_payload(temp.path(), id, &serde_json::json!({"done":true}))
                .await
                .unwrap();
        }
        let unrelated = temp.path().join("runs").join(format!("{old}.notes.json"));
        std::fs::write(&unrelated, "keep").unwrap();
        let records = vec![
            make_record(
                &live,
                BgAgentStatus::Running,
                now - Duration::days(30),
                None,
            ),
            make_record(
                &old,
                BgAgentStatus::Failed,
                now - Duration::days(20),
                Some(now - Duration::days(10)),
            ),
        ];
        save_all(temp.path(), records).await.unwrap();
        let loaded = load_all(temp.path()).await.unwrap();
        assert!(loaded.contains_key(&live));
        assert!(!loaded.contains_key(&old));
        assert!(load_accepted_launch(temp.path(), &live, 128).await.is_ok());
        assert!(load_run_config(temp.path(), &live, 1, 128).await.is_ok());
        assert!(load_run_config(temp.path(), &live, 2, 128).await.is_ok());
        assert!(temp
            .path()
            .join(RESULTS_DIR)
            .join(format!("{live}.json"))
            .exists());
        assert!(load_accepted_launch(temp.path(), &old, 128).await.is_err());
        assert!(load_run_config(temp.path(), &old, 1, 128).await.is_err());
        assert!(load_run_config(temp.path(), &old, 2, 128).await.is_err());
        assert!(!temp
            .path()
            .join(RESULTS_DIR)
            .join(format!("{old}.json"))
            .exists());
        assert_eq!(std::fs::read_to_string(&unrelated).unwrap(), "keep");
        let again = load_all(temp.path()).await.unwrap().into_values().collect();
        save_all(temp.path(), again).await.unwrap();
        assert!(load_accepted_launch(temp.path(), &live, 128).await.is_ok());
    }

    #[test]
    fn prune_records_keeps_live_and_recent_terminal_records() {
        let now = Utc::now();
        let mut records = vec![
            make_record(
                "running",
                BgAgentStatus::Running,
                now - Duration::days(30),
                None,
            ),
            make_record(
                "recent-terminal",
                BgAgentStatus::Completed,
                now - Duration::days(2),
                Some(now - Duration::days(1)),
            ),
            make_record(
                "old-terminal",
                BgAgentStatus::Failed,
                now - Duration::days(20),
                Some(now - Duration::days(10)),
            ),
        ];

        prune_records(&mut records, now);

        assert_eq!(records.len(), 2);
        assert!(records.iter().any(|record| record.agent_id == "running"));
        assert!(records
            .iter()
            .any(|record| record.agent_id == "recent-terminal"));
        assert!(!records
            .iter()
            .any(|record| record.agent_id == "old-terminal"));
    }

    #[test]
    fn prune_records_caps_oldest_terminal_records_first() {
        let now = Utc::now();
        let mut records = Vec::new();
        records.push(make_record("live", BgAgentStatus::Running, now, None));
        for index in 0..(MAX_RECORDS + 20) {
            let created_at = now - Duration::minutes((MAX_RECORDS + 20 - index) as i64);
            records.push(make_record(
                &format!("terminal-{index}"),
                BgAgentStatus::Completed,
                created_at,
                Some(created_at),
            ));
        }

        prune_records(&mut records, now);

        assert_eq!(records.len(), MAX_RECORDS);
        assert!(records.iter().any(|record| record.agent_id == "live"));
        assert!(!records.iter().any(|record| record.agent_id == "terminal-0"));
        assert!(records
            .iter()
            .any(|record| record.agent_id == "terminal-21"));
    }
}

pub async fn save_result_payload(
    storage_root: &Path,
    agent_id: &str,
    payload: &serde_json::Value,
) -> Result<PathBuf, String> {
    let results_dir = storage_root.join(RESULTS_DIR);
    fs::create_dir_all(&results_dir)
        .await
        .map_err(|e| format!("Failed to create background agent results directory: {e}"))?;
    let result_path = results_dir.join(format!("{agent_id}.json"));
    let tmp_path = tmp_path_for(&result_path);
    let content = serde_json::to_string_pretty(payload)
        .map_err(|e| format!("Failed to serialize background agent result: {e}"))?;
    if let Err(e) = fs::write(&tmp_path, content).await {
        let _ = fs::remove_file(&tmp_path).await;
        return Err(format!("Failed to write background agent result: {e}"));
    }
    atomic_write_file(&tmp_path, &result_path).await?;
    Ok(result_path)
}
