use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use serde_json::{json, Value};
use refact_python_kernel::protocol::WriteOriginal;
use crate::{
    call_validation::{ChatMessage, DiffChunk},
    privacy::records,
};

pub fn merge_taint(message: &mut ChatMessage, values: &[Value]) -> Result<(), String> {
    for value in values {
        for key in ["privacy_observation", "privacy_shell"] {
            if let Some(observation) = value.get(key) {
                fn rank(v: &Value) -> u8 {
                    match v["status"].as_str() {
                        Some("unavailable") => 4,
                        Some("incomplete") => 3,
                        Some("pending") => 2,
                        _ => u8::from(v["degraded"] == true || v["incomplete"] == true),
                    }
                }
                if message
                    .extra
                    .get(key)
                    .is_none_or(|old| rank(old) <= rank(observation))
                {
                    message.extra.insert(key.into(), observation.clone());
                }
            }
        }
        if value.get("files").is_some() {
            let privacy: refact_privacy::PrivacyRecord =
                serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
            records::merge_records(message, privacy.files);
        }
    }
    Ok(())
}

pub async fn restore_ledger(
    workspace: &Path,
    zones: &records::DerivedPrivacyZones,
) -> Result<(), String> {
    let path = workspace.join("privacy_ledger.json");
    let bytes = match tokio::fs::read(path).await {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.to_string()),
    };
    let entries: Vec<(PathBuf, records::DerivedZone)> =
        serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    zones
        .write()
        .unwrap()
        .extend(entries.into_iter().rev().take(10000));
    Ok(())
}

/// Files with run data the engine writes into a run folder: `code.py`, `stdout.txt`,
/// `result.json`, `displays/*` and `calls/*` (nested-call and model-call logs).
pub async fn run_data_files(run_dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = ["code.py", "stdout.txt", "result.json"]
        .iter()
        .map(|name| run_dir.join(name))
        .collect();
    for dir in ["displays", "calls"] {
        if let Ok(mut entries) = tokio::fs::read_dir(run_dir.join(dir)).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                if entry.file_type().await.is_ok_and(|t| t.is_file()) {
                    files.push(entry.path());
                }
            }
        }
    }
    files
}

pub async fn save_ledger(
    workspace: &Path,
    paths: impl IntoIterator<Item = PathBuf>,
    gcx: &std::sync::Arc<crate::global_context::GlobalContext>,
    zones: &records::DerivedPrivacyZones,
) -> Result<(), String> {
    let path = workspace.join("privacy_ledger.json");
    let mut entries: Vec<(PathBuf, records::DerivedZone)> = match tokio::fs::read(&path).await {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| e.to_string())?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => vec![],
        Err(e) => return Err(e.to_string()),
    };
    for path in paths {
        if let Some(zone) = records::derived_zone_for_path(gcx, &path, zones) {
            entries.retain(|(p, _)| p != &path);
            entries.push((path, zone));
        }
    }
    if entries.len() > 10000 {
        entries.drain(..entries.len() - 10000);
    }
    super::workspace::atomic_write(
        &path,
        &serde_json::to_vec(&entries).map_err(|e| e.to_string())?,
    )
    .await
}

pub async fn written_files(
    writes: &BTreeMap<String, u64>,
    captures: &BTreeMap<String, WriteOriginal>,
    renames: &BTreeMap<String, String>,
    run_dir: &Path,
) -> Vec<Value> {
    written_files_budget(writes, captures, renames, run_dir, 256 * 1024).await
}
pub async fn written_files_budget(
    writes: &BTreeMap<String, u64>,
    captures: &BTreeMap<String, WriteOriginal>,
    renames: &BTreeMap<String, String>,
    run_dir: &Path,
    mut budget: usize,
) -> Vec<Value> {
    let mut files = Vec::new();

    for (path, bytes) in writes {
        if renames.values().any(|dst| dst == path) {
            continue;
        }
        let chunks = if let Some(dst) = renames.get(path) {
            let mut chunks = captured_diff(dst, captures.get(path)).await;
            for chunk in &mut chunks {
                chunk.file_name = path.clone();
                chunk.file_name_rename = Some(dst.clone());
                chunk.file_action = "rename".into();
            }
            if chunks.is_empty() && text(Path::new(dst)).await.is_some() {
                chunks.push(DiffChunk {
                    file_name: path.clone(),
                    file_name_rename: Some(dst.clone()),
                    file_action: "rename".into(),
                    ..Default::default()
                });
            }
            chunks
        } else {
            captured_diff(path, captures.get(path)).await
        };
        let encoded = serde_json::to_vec(&chunks).unwrap_or_default();
        let mut note = Value::Null;
        let diffs = if encoded.len() <= budget {
            budget -= encoded.len();
            json!(chunks)
        } else {
            let name = format!("diff-{}.json", files.len());
            let _ = tokio::fs::write(run_dir.join(&name), encoded).await;
            note = json!(format!("Diff is in the run directory: {name}"));
            json!([])
        };
        files.push(json!({"path":path,"bytes":bytes,"diffs":diffs,"note":note}));
    }
    files
}
async fn text(path: &Path) -> Option<String> {
    if tokio::fs::metadata(path).await.ok()?.len() > 2 * 1024 * 1024 {
        return None;
    }
    let bytes = tokio::fs::read(path).await.ok()?;
    if bytes.len() > 2 * 1024 * 1024 || bytes.contains(&0) {
        return None;
    }
    String::from_utf8(bytes).ok()
}
async fn captured_diff(path: &str, original: Option<&WriteOriginal>) -> Vec<DiffChunk> {
    let path = Path::new(path);
    let before = match original {
        Some(WriteOriginal::Missing) => String::new(),
        Some(WriteOriginal::File {
            capture_path,
        }) => match text(Path::new(capture_path)).await {
            Some(s) => s,
            None => return vec![],
        },
        _ => return vec![],
    };
    let after = if path.exists() {
        match text(path).await {
            Some(s) => s,
            None => return vec![],
        }
    } else {
        String::new()
    };
    if matches!(original, Some(WriteOriginal::Missing)) {
        return vec![DiffChunk {
            file_name: path.to_string_lossy().into_owned(),
            file_action: "add".into(),
            line1: 1,
            line2: 1,
            lines_add: after,
            ..Default::default()
        }];
    }
    if !path.exists() {
        return vec![DiffChunk {
            file_name: path.to_string_lossy().into_owned(),
            file_action: "remove".into(),
            line1: 1,
            line2: before.lines().count() + 1,
            lines_remove: before,
            ..Default::default()
        }];
    }
    crate::tools::file_edit::auxiliary::convert_edit_to_diffchunks(
        path.to_path_buf(),
        &before,
        &after,
    )
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn capture_diffs_cover_modified_deleted_renamed_binary_and_budget() {
        let temp = tempfile::tempdir().unwrap();
        let original = temp.path().join("original");
        tokio::fs::write(&original, "before\n").await.unwrap();
        let mut writes = BTreeMap::new();
        let mut captures = BTreeMap::new();
        for name in ["modified", "deleted", "renamed", "binary", "large"] {
            let path = temp.path().join(name).to_string_lossy().into_owned();
            writes.insert(path.clone(), 1);
            captures.insert(
                path,
                WriteOriginal::File {
                    capture_path: original.to_string_lossy().into_owned(),
                },
            );
        }
        tokio::fs::write(temp.path().join("modified"), "after\n")
            .await
            .unwrap();
        tokio::fs::write(temp.path().join("destination"), "after\n")
            .await
            .unwrap();
        tokio::fs::write(temp.path().join("binary"), [0, 1])
            .await
            .unwrap();
        tokio::fs::write(temp.path().join("large"), "x".repeat(300_000))
            .await
            .unwrap();
        let renames = BTreeMap::from([(
            temp.path().join("renamed").to_string_lossy().into_owned(),
            temp.path()
                .join("destination")
                .to_string_lossy()
                .into_owned(),
        )]);
        let files = written_files(&writes, &captures, &renames, temp.path()).await;
        let find = |name: &str| {
            files
                .iter()
                .find(|f| f["path"].as_str().unwrap().ends_with(name))
                .unwrap()
        };
        assert_eq!(find("modified")["diffs"][0]["file_action"], "edit");
        assert_eq!(find("deleted")["diffs"][0]["file_action"], "remove");
        assert_eq!(find("renamed")["diffs"][0]["file_action"], "rename");
        assert!(find("binary")["diffs"].as_array().unwrap().is_empty());
        assert!(find("large")["diffs"].as_array().unwrap().is_empty());
        assert!(find("large")["note"]
            .as_str()
            .unwrap()
            .contains("run directory"));
    }
}

#[cfg(test)]
mod observation_tests {
    use super::*;
    #[test]
    fn nested_observation_survives_taint_merge() {
        let mut message = ChatMessage::default();
        merge_taint(
            &mut message,
            &[json!({"privacy_observation":{"status":"incomplete"}})],
        )
        .unwrap();
        assert_eq!(message.extra["privacy_observation"]["status"], "incomplete");
    }
}
