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
    gcx: &std::sync::Arc<crate::global_context::GlobalContext>,
    zones: &records::DerivedPrivacyZones,
) -> Result<(), String> {
    let entries = read_ledger(workspace).await?.unwrap_or_default();
    records::merge_derived_zones(gcx, zones, entries)
}

async fn read_ledger(
    workspace: &Path,
) -> Result<Option<Vec<(PathBuf, records::DerivedZone)>>, String> {
    let path = workspace.join("privacy_ledger.json");
    let bytes = match tokio::fs::read(path).await {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    let entries: Vec<(PathBuf, records::DerivedZone)> =
        serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if entries.len() > 10000 {
        return Err("Python privacy ledger exceeds 10000 entries".into());
    }
    Ok(Some(entries))
}

pub async fn import_parent_ledger(
    artifacts: &Path,
    gcx: &std::sync::Arc<crate::global_context::GlobalContext>,
    zones: &records::DerivedPrivacyZones,
) -> Result<(), String> {
    let workspace = artifacts
        .parent()
        .ok_or("Invalid parent artifacts directory")?;
    let ledger = read_ledger(workspace).await?;
    match tokio::fs::symlink_metadata(artifacts).await {
        Ok(meta) if meta.file_type().is_symlink() || is_reparse_point(&meta) => {
            return Err("Linked parent Python artifacts root refused".into());
        }
        Ok(meta) if !meta.is_dir() => {
            return Err("Parent Python artifacts root is not a directory".into());
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.to_string()),
    }
    let root = match tokio::fs::canonicalize(artifacts).await {
        Ok(root) => root,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.to_string()),
    };
    let Some(entries) = ledger else {
        let mut entries = tokio::fs::read_dir(&root)
            .await
            .map_err(|e| e.to_string())?;
        return if entries
            .next_entry()
            .await
            .map_err(|e| e.to_string())?
            .is_none()
        {
            Ok(())
        } else {
            Err("Parent Python artifacts have no privacy ledger".into())
        };
    };
    let mut imported = Vec::new();
    for (path, zone) in entries {
        if !path.is_absolute() || !(path.starts_with(artifacts) || path.starts_with(&root)) {
            continue;
        }
        let canonical = match tokio::fs::canonicalize(&path).await {
            Ok(path) => path,
            Err(e) if names_no_file(&e) => continue,
            Err(e) => return Err(e.to_string()),
        };
        let Ok(relative) = canonical.strip_prefix(&root) else {
            continue;
        };
        match tokio::fs::symlink_metadata(&canonical).await {
            Ok(meta) if meta.is_file() => {}
            Ok(_) => continue,
            Err(e) if names_no_file(&e) => continue,
            Err(e) => return Err(e.to_string()),
        }
        imported.push((path, zone.clone()));
        imported.push((artifacts.join(relative), zone.clone()));
        imported.push((canonical, zone));
    }
    records::merge_derived_zones(gcx, zones, imported)
}

fn names_no_file(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
    )
}

async fn bounded_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    let mut pending = vec![(root.to_path_buf(), 0usize)];
    let mut visits = 0usize;
    while let Some((path, depth)) = pending.pop() {
        visits += 1;
        if visits > 10000 || depth > 4 {
            return Err("Python data file enumeration limit exceeded".into());
        }
        let meta = tokio::fs::symlink_metadata(&path)
            .await
            .map_err(|e| e.to_string())?;
        if meta.file_type().is_symlink() || is_reparse_point(&meta) {
            return Err(format!(
                "Linked Python data path refused: {}",
                path.display()
            ));
        }
        if meta.is_file() {
            files.push(path);
        } else if meta.is_dir() {
            let mut entries = tokio::fs::read_dir(&path)
                .await
                .map_err(|e| e.to_string())?;
            while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
                if visits + pending.len() >= 10000 {
                    return Err("Python data file enumeration limit exceeded".into());
                }
                pending.push((entry.path(), depth + 1));
            }
        } else {
            return Err(format!(
                "Non-regular Python data path refused: {}",
                path.display()
            ));
        }
    }
    files.sort();
    Ok(files)
}

fn is_reparse_point(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        let _ = meta;
        false
    }
}

pub async fn run_data_files(run_dir: &Path) -> Result<Vec<PathBuf>, String> {
    bounded_files(run_dir).await
}

pub async fn persist_run_privacy(
    gcx: &std::sync::Arc<crate::global_context::GlobalContext>,
    workspace: &Path,
    run_dir: &Path,
    reads: &[refact_privacy::FileRecord],
    mut paths: Vec<PathBuf>,
    zones: &records::DerivedPrivacyZones,
) -> Result<(), String> {
    paths.extend(run_data_files(run_dir).await?);
    let summary = run_dir.join(super::workspace::RUN_SUMMARY_FILE);
    paths.push(summary.clone());
    let policy = gcx.privacy_policy_load.read().unwrap().policy.clone();
    records::inherit_observed_write_zones(gcx, &policy, reads, paths.iter().cloned(), zones)?;
    if let Some(zone) = records::derived_zone_for_path(gcx, &run_dir.join("run.json"), zones) {
        records::merge_derived_zones(gcx, zones, [(summary, zone)])?;
    }
    save_ledger(workspace, paths, gcx, zones).await
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
        return Err("Python privacy ledger exceeds 10000 entries".into());
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
) -> Result<Vec<Value>, String> {
    written_files_budget(writes, captures, renames, run_dir, 256 * 1024).await
}
pub async fn written_files_budget(
    writes: &BTreeMap<String, u64>,
    captures: &BTreeMap<String, WriteOriginal>,
    renames: &BTreeMap<String, String>,
    run_dir: &Path,
    mut budget: usize,
) -> Result<Vec<Value>, String> {
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
        let encoded = serde_json::to_vec(&chunks).map_err(|e| e.to_string())?;
        let mut note = Value::Null;
        let diffs = if encoded.len() <= budget {
            budget -= encoded.len();
            json!(chunks)
        } else {
            let name = format!("diff-{}.json", files.len());
            super::workspace::atomic_write(&run_dir.join(&name), &encoded).await?;
            note = json!(format!("Diff is in the run directory: {name}"));
            json!([])
        };
        files.push(json!({"path":path,"bytes":bytes,"diffs":diffs,"note":note}));
    }
    Ok(files)
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
    async fn parent_import_is_confined_monotonic_and_enforces_destination_privacy() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("parent");
        let artifacts = workspace.join("artifacts");
        std::fs::create_dir_all(&artifacts).unwrap();
        let artifacts = dunce::canonicalize(artifacts).unwrap();
        let guarded = artifacts.join("guarded.txt");
        let stricter = artifacts.join("stricter.txt");
        let outside = temp.path().join("parentx.txt");
        for path in [&guarded, &stricter, &outside] {
            std::fs::write(path, "derived secret").unwrap();
        }
        let gcx = crate::global_context::tests::make_test_gcx().await;
        gcx.privacy_policy_load.write().unwrap().policy =
            std::sync::Arc::new(refact_privacy::PrivacyPolicy {
                zones: vec![refact_privacy::Zone {
                    name: "restricted".into(),
                    patterns: vec!["source-only.txt".into()],
                    send_to: vec!["trusted".into()],
                    ..Default::default()
                }],
                ..Default::default()
            });
        let label = |zone: &str| records::DerivedZone {
            zone: zone.into(),
            origin: Some("source-only.txt".into()),
        };
        let entries = vec![
            (guarded.clone(), label("restricted")),
            (stricter.clone(), label("normal")),
            (outside.clone(), label("restricted")),
        ];
        std::fs::write(
            workspace.join("privacy_ledger.json"),
            serde_json::to_vec(&entries).unwrap(),
        )
        .unwrap();
        let child = records::new_derived_privacy_zones();
        child
            .write()
            .unwrap()
            .insert(stricter.clone(), label("blocked"));
        import_parent_ledger(&artifacts, &gcx, &child)
            .await
            .unwrap();
        assert_eq!(child.read().unwrap()[&stricter].zone, "blocked");
        assert!(!child.read().unwrap().contains_key(&outside));
        let classified =
            records::observed_file_records_with_derived(&gcx, [guarded.clone()], &child).unwrap();
        assert_eq!(classified[0].zone, "restricted");
        let mut message = ChatMessage::new("tool".into(), "derived secret".into());
        records::apply_shell_observation(
            &gcx,
            "cat guarded.txt",
            &artifacts,
            &records::provider_destination("untrusted/model"),
            refact_exec::ObservationStatus::Observed(refact_exec::ObservedAccess {
                reads: vec![guarded],
                writes: vec![],
            }),
            &child,
            &mut message,
        )
        .await
        .unwrap();
        assert_eq!(message.extra["privacy_shell"]["withheld"], true);
        restore_ledger(&workspace, &gcx, &child).await.unwrap();
        assert_eq!(child.read().unwrap()[&stricter].zone, "blocked");
        std::fs::write(workspace.join("privacy_ledger.json"), "malformed").unwrap();
        assert!(
            import_parent_ledger(&artifacts, &gcx, &records::new_derived_privacy_zones())
                .await
                .is_err()
        );
        assert_eq!(
            std::fs::read(workspace.join("privacy_ledger.json")).unwrap(),
            b"malformed"
        );
        std::fs::remove_file(workspace.join("privacy_ledger.json")).unwrap();
        assert!(import_parent_ledger(&artifacts, &gcx, &child)
            .await
            .is_err());
        std::fs::remove_file(artifacts.join("guarded.txt")).unwrap();
        std::fs::remove_file(artifacts.join("stricter.txt")).unwrap();
        import_parent_ledger(&artifacts, &gcx, &child)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn parent_import_labels_plain_paths_without_scanning_deep_artifacts() {
        let temp = tempfile::tempdir().unwrap();
        let artifacts = temp.path().join("parent/artifacts");
        let lexical = artifacts.join("deep/normal/artifact/tree/beyond/scan/file.txt");
        std::fs::create_dir_all(lexical.parent().unwrap()).unwrap();
        std::fs::write(&lexical, "secret").unwrap();
        let canonical = std::fs::canonicalize(&lexical).unwrap();
        std::fs::write(
            artifacts.parent().unwrap().join("privacy_ledger.json"),
            serde_json::to_vec(&vec![(
                canonical.clone(),
                records::DerivedZone {
                    zone: "blocked".into(),
                    origin: Some("source.txt".into()),
                },
            )])
            .unwrap(),
        )
        .unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let zones = records::new_derived_privacy_zones();
        import_parent_ledger(&artifacts, &gcx, &zones)
            .await
            .unwrap();
        let zones = zones.read().unwrap();
        for path in [&canonical, &lexical] {
            assert_eq!(zones[path].zone, "blocked");
            assert_eq!(zones[path].origin.as_deref(), Some("source.txt"));
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn parent_import_labels_lexically_aliased_ancestor() {
        let temp = tempfile::tempdir().unwrap();
        let actual = temp.path().join("actual");
        std::fs::create_dir_all(actual.join("parent/artifacts")).unwrap();
        let alias = temp.path().join("alias");
        std::os::unix::fs::symlink(&actual, &alias).unwrap();
        let artifacts = alias.join("parent/artifacts");
        let lexical = artifacts.join("file.txt");
        std::fs::write(&lexical, "secret").unwrap();
        let canonical = std::fs::canonicalize(&lexical).unwrap();
        assert_ne!(canonical, lexical);
        std::fs::write(
            artifacts.parent().unwrap().join("privacy_ledger.json"),
            serde_json::to_vec(&vec![(
                canonical.clone(),
                records::DerivedZone {
                    zone: "blocked".into(),
                    origin: Some("source.txt".into()),
                },
            )])
            .unwrap(),
        )
        .unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let zones = records::new_derived_privacy_zones();
        import_parent_ledger(&artifacts, &gcx, &zones)
            .await
            .unwrap();
        let zones = zones.read().unwrap();
        for path in [&canonical, &lexical] {
            assert_eq!(zones[path].zone, "blocked");
            assert_eq!(zones[path].origin.as_deref(), Some("source.txt"));
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn parent_import_labels_confined_links_skips_escapes_and_refuses_linked_root() {
        let temp = tempfile::tempdir().unwrap();
        let artifacts = temp.path().join("parent/artifacts");
        std::fs::create_dir_all(&artifacts).unwrap();
        let outside = temp.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("secret"), "secret").unwrap();
        std::fs::write(artifacts.join("regular"), "secret").unwrap();
        std::os::unix::fs::symlink("regular", artifacts.join("linked-file")).unwrap();
        std::os::unix::fs::symlink(&outside, artifacts.join("escape")).unwrap();
        std::os::unix::fs::symlink(outside.join("secret"), artifacts.join("escaping-file"))
            .unwrap();
        let entries: Vec<_> = [
            "linked-file",
            "escape/secret",
            "escaping-file",
            "missing",
            "regular/child",
        ]
        .into_iter()
        .map(|name| {
            (
                artifacts.join(name),
                records::DerivedZone {
                    zone: "blocked".into(),
                    origin: None,
                },
            )
        })
        .collect();
        std::fs::write(
            artifacts.parent().unwrap().join("privacy_ledger.json"),
            serde_json::to_vec(&entries).unwrap(),
        )
        .unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let zones = records::new_derived_privacy_zones();
        import_parent_ledger(&artifacts, &gcx, &zones)
            .await
            .unwrap();
        let expected: std::collections::HashSet<PathBuf> = [
            artifacts.join("linked-file"),
            artifacts.join("regular"),
            std::fs::canonicalize(artifacts.join("regular")).unwrap(),
        ]
        .into_iter()
        .collect();
        {
            let zones = zones.read().unwrap();
            assert_eq!(
                zones
                    .keys()
                    .cloned()
                    .collect::<std::collections::HashSet<_>>(),
                expected
            );
            assert!(zones.values().all(|zone| zone.zone == "blocked"));
        }
        let linked_root = artifacts.parent().unwrap().join("linked-artifacts");
        std::os::unix::fs::symlink(&artifacts, &linked_root).unwrap();
        let error = import_parent_ledger(&linked_root, &gcx, &zones)
            .await
            .unwrap_err();
        assert!(error.contains("artifacts root refused"), "{error}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn parent_import_preserves_safe_original_ledger_alias() {
        let temp = tempfile::tempdir().unwrap();
        let artifacts = temp.path().join("parent/artifacts");
        std::fs::create_dir_all(artifacts.join("real")).unwrap();
        std::fs::write(artifacts.join("real/file.txt"), "secret").unwrap();
        std::os::unix::fs::symlink("real", artifacts.join("alias")).unwrap();
        let original = artifacts.join("alias/file.txt");
        let canonical = std::fs::canonicalize(&original).unwrap();
        std::fs::write(
            artifacts.parent().unwrap().join("privacy_ledger.json"),
            serde_json::to_vec(&vec![(
                original.clone(),
                records::DerivedZone {
                    zone: "blocked".into(),
                    origin: None,
                },
            )])
            .unwrap(),
        )
        .unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let zones = records::new_derived_privacy_zones();
        import_parent_ledger(&artifacts, &gcx, &zones)
            .await
            .unwrap();
        let zones = zones.read().unwrap();
        for path in [original, canonical, artifacts.join("real/file.txt")] {
            assert_eq!(zones[&path].zone, "blocked");
        }
    }

    #[tokio::test]
    async fn parent_without_ledger_requires_no_direct_entries() {
        let temp = tempfile::tempdir().unwrap();
        let artifacts = temp.path().join("artifacts");
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let zones = records::new_derived_privacy_zones();
        import_parent_ledger(&artifacts, &gcx, &zones)
            .await
            .unwrap();
        std::fs::create_dir(&artifacts).unwrap();
        import_parent_ledger(&artifacts, &gcx, &zones)
            .await
            .unwrap();
        std::fs::create_dir(artifacts.join("empty-directory")).unwrap();
        let error = import_parent_ledger(&artifacts, &gcx, &zones)
            .await
            .unwrap_err();
        assert!(error.contains("no privacy ledger"), "{error}");
        std::fs::remove_dir(artifacts.join("empty-directory")).unwrap();
        std::fs::remove_dir(&artifacts).unwrap();
        std::fs::write(&artifacts, "not a directory").unwrap();
        let error = import_parent_ledger(&artifacts, &gcx, &zones)
            .await
            .unwrap_err();
        assert!(error.contains("not a directory"), "{error}");
    }

    #[tokio::test]
    async fn planned_summary_privacy_is_saved_before_summary_exists() {
        let temp = tempfile::tempdir().unwrap();
        let run = temp.path().join("runs/one");
        std::fs::create_dir_all(&run).unwrap();
        super::super::workspace::write_running_run_json(&run, "one", 1, false, &Value::Null)
            .await
            .unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let zones = records::new_derived_privacy_zones();
        let read = refact_privacy::FileRecord {
            path: "source.txt".into(),
            zone: "blocked".into(),
            attribution: refact_privacy::Attribution::Observed,
        };
        persist_run_privacy(&gcx, temp.path(), &run, &[read], vec![], &zones)
            .await
            .unwrap();
        let summary = run.join(super::super::workspace::RUN_SUMMARY_FILE);
        assert!(!summary.exists());
        let metadata: Value =
            serde_json::from_slice(&std::fs::read(run.join("run.json")).unwrap()).unwrap();
        assert_eq!(metadata["status"], "running");
        let entries = read_ledger(temp.path()).await.unwrap().unwrap();
        for path in [run.join("run.json"), summary] {
            let zone = &entries.iter().find(|(entry, _)| entry == &path).unwrap().1;
            assert_eq!(zone.zone, "blocked");
            assert_eq!(zone.origin.as_deref(), Some("source.txt"));
        }
    }

    #[tokio::test]
    async fn complete_run_files_are_labeled_persisted_and_refreshed() {
        let temp = tempfile::tempdir().unwrap();
        let run = temp.path().join("runs/one");
        let names = [
            "code.py",
            "stdout.txt",
            "result.json",
            "run.json",
            "summary.json",
            "diff-0.json",
            "captures/original",
            "calls/one.json",
            "displays/one.json",
        ];
        for name in names {
            let path = run.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "secret").unwrap();
        }
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let zones = records::new_derived_privacy_zones();
        let policy = gcx.privacy_policy_load.read().unwrap().policy.clone();
        let read = refact_privacy::FileRecord {
            path: "secret".into(),
            zone: "blocked".into(),
            attribution: refact_privacy::Attribution::Observed,
        };
        let paths = run_data_files(&run).await.unwrap();
        assert_eq!(paths.len(), names.len());
        records::inherit_observed_write_zones(&gcx, &policy, &[read], paths.clone(), &zones)
            .unwrap();
        save_ledger(temp.path(), paths.clone(), &gcx, &zones)
            .await
            .unwrap();
        let restored = records::new_derived_privacy_zones();
        restore_ledger(temp.path(), &gcx, &restored).await.unwrap();
        for path in &paths {
            assert_eq!(restored.read().unwrap()[path].zone, "blocked");
        }
        let capture = run.join("captures/original");
        std::fs::write(&capture, "later contents").unwrap();
        save_ledger(temp.path(), [capture.clone()], &gcx, &restored)
            .await
            .unwrap();
        let entries = read_ledger(temp.path()).await.unwrap().unwrap();
        assert_eq!(
            entries.iter().filter(|(path, _)| path == &capture).count(),
            1
        );
        assert_eq!(
            entries
                .iter()
                .find(|(path, _)| path == &capture)
                .unwrap()
                .1
                .zone,
            "blocked"
        );
        std::fs::create_dir_all(run.join("too/deep/for/a/scan")).unwrap();
        assert!(run_data_files(&run).await.is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_file_enumeration_rejects_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("missing", temp.path().join("capture")).unwrap();
        assert!(run_data_files(temp.path()).await.is_err());
    }

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
        let files = written_files(&writes, &captures, &renames, temp.path())
            .await
            .unwrap();
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
