use std::path::{Component, Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use tokio::fs;
use uuid::Uuid;

pub const RECEIPTS_CAP: usize = 200;
pub const RECEIPT_RETENTION_DAYS: i64 = 7;
const ALLOWED_EXTENSIONS: &[&str] = &["yaml", "yml", "md", "json"];

static RECEIPTS_IO_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuddyReceipt {
    pub id: String,
    pub action_kind: String,
    pub target_path: String,
    pub pre_image: Option<String>,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub undone: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undone_at: Option<DateTime<Utc>>,
}

pub fn receipts_path(project_root: &Path) -> PathBuf {
    project_root
        .join(".refact")
        .join("buddy")
        .join("receipts.json")
}

pub async fn load_receipts(project_root: &Path) -> Vec<BuddyReceipt> {
    let path = receipts_path(project_root);
    let content = match fs::read_to_string(&path).await {
        Ok(content) => content,
        Err(_) => return Vec::new(),
    };
    let receipts: Vec<BuddyReceipt> = serde_json::from_str(&content).unwrap_or_default();
    let cutoff = Utc::now() - Duration::days(RECEIPT_RETENTION_DAYS);
    receipts
        .into_iter()
        .filter(|r| r.created_at >= cutoff)
        .collect()
}

async fn save_receipts(project_root: &Path, receipts: &[BuddyReceipt]) -> Result<(), String> {
    let path = receipts_path(project_root);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("failed to create {:?}: {}", parent, e))?;
    }
    super::storage::atomic_write_json(&path, &receipts.to_vec()).await
}

pub fn config_patch_target_allowed(target_rel: &str) -> Result<PathBuf, String> {
    let rel = relative_refact_target(target_rel)?;
    let mut components = rel.components();
    components.next();
    if is_buddy_internal_component(components.next()) {
        return Err("config patches may not target .refact/buddy internal state".to_string());
    }
    let extension = rel
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_lowercase)
        .unwrap_or_default();
    if !ALLOWED_EXTENSIONS.contains(&extension.as_str()) {
        return Err(format!(
            "config patches may only write {:?} files",
            ALLOWED_EXTENSIONS
        ));
    }
    Ok(rel)
}

pub fn undo_target_allowed(target_rel: &str) -> Result<PathBuf, String> {
    relative_refact_target(target_rel)
}

fn relative_refact_target(target_rel: &str) -> Result<PathBuf, String> {
    let rel = Path::new(target_rel);
    if rel.is_absolute() {
        return Err("target_path must be relative to the project root".to_string());
    }
    for component in rel.components() {
        match component {
            Component::Normal(part) => {
                if normalized_component_text(part).is_empty() {
                    return Err("target_path contains an empty path component".to_string());
                }
            }
            _ => return Err("target_path must not contain '..' or special components".to_string()),
        }
    }
    let mut components = rel.components();
    if components.next() != Some(Component::Normal(".refact".as_ref())) {
        return Err("config patches may only write under .refact/".to_string());
    }
    Ok(rel.to_path_buf())
}

fn normalized_component_text(part: &std::ffi::OsStr) -> String {
    part.to_string_lossy()
        .trim_end_matches(['.', ' '])
        .to_ascii_lowercase()
}

fn is_buddy_internal_component(component: Option<Component>) -> bool {
    match component {
        Some(Component::Normal(part)) => normalized_component_text(part) == "buddy",
        _ => false,
    }
}

async fn nearest_existing_ancestor(path: &Path) -> Option<PathBuf> {
    let mut probe = path.to_path_buf();
    loop {
        if fs::symlink_metadata(&probe).await.is_ok() {
            return Some(probe);
        }
        probe = probe.parent()?.to_path_buf();
    }
}

async fn resolved_target_guard(
    project_root: &Path,
    rel: &Path,
    deny_buddy_subtree: bool,
) -> Result<(), String> {
    let root_canon = fs::canonicalize(project_root)
        .await
        .map_err(|e| format!("failed to resolve project root {:?}: {}", project_root, e))?;
    let absolute = project_root.join(rel);
    let anchor = nearest_existing_ancestor(&absolute)
        .await
        .ok_or_else(|| "target_path resolves outside the project root".to_string())?;
    let anchor_canon = fs::canonicalize(&anchor)
        .await
        .map_err(|e| format!("failed to resolve {:?}: {}", anchor, e))?;
    if !anchor_canon.starts_with(&root_canon) {
        return Err("target_path resolves outside the project root".to_string());
    }
    if deny_buddy_subtree {
        let buddy_dir = root_canon.join(".refact").join("buddy");
        if let Ok(buddy_canon) = fs::canonicalize(&buddy_dir).await {
            if anchor_canon.starts_with(&buddy_canon) {
                return Err(
                    "config patches may not target .refact/buddy internal state".to_string()
                );
            }
        }
    }
    Ok(())
}

pub async fn apply_config_patch(
    project_root: &Path,
    target_rel: &str,
    content: &str,
) -> Result<BuddyReceipt, String> {
    let _guard = RECEIPTS_IO_LOCK.lock().await;
    let rel = config_patch_target_allowed(target_rel)?;
    resolved_target_guard(project_root, &rel, true).await?;
    let absolute = project_root.join(&rel);
    let pre_image = fs::read_to_string(&absolute).await.ok();
    if let Some(parent) = absolute.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("failed to create {:?}: {}", parent, e))?;
    }
    super::storage::atomic_write_text(&absolute, content).await?;
    let receipt = BuddyReceipt {
        id: Uuid::new_v4().to_string(),
        action_kind: "apply_config_patch".to_string(),
        target_path: rel.to_string_lossy().to_string(),
        pre_image,
        created_at: Utc::now(),
        undone: false,
        undone_at: None,
    };
    let mut receipts = load_receipts(project_root).await;
    receipts.push(receipt.clone());
    let overflow = receipts.len().saturating_sub(RECEIPTS_CAP);
    if overflow > 0 {
        receipts.drain(..overflow);
    }
    save_receipts(project_root, &receipts).await?;
    Ok(receipt)
}

pub async fn undo_receipt(project_root: &Path, receipt_id: &str) -> Result<BuddyReceipt, String> {
    let _guard = RECEIPTS_IO_LOCK.lock().await;
    let mut receipts = load_receipts(project_root).await;
    let receipt = receipts
        .iter_mut()
        .find(|r| r.id == receipt_id)
        .ok_or_else(|| format!("receipt not found: {}", receipt_id))?;
    if receipt.undone {
        return Err(format!("receipt already undone: {}", receipt_id));
    }
    let rel = undo_target_allowed(&receipt.target_path)?;
    resolved_target_guard(project_root, &rel, false).await?;
    let absolute = project_root.join(&rel);
    match &receipt.pre_image {
        Some(pre_image) => {
            super::storage::atomic_write_text(&absolute, pre_image).await?;
        }
        None => {
            if let Err(err) = fs::remove_file(&absolute).await {
                if err.kind() != std::io::ErrorKind::NotFound {
                    return Err(format!("failed to remove {:?}: {}", absolute, err));
                }
            }
        }
    }
    receipt.undone = true;
    receipt.undone_at = Some(Utc::now());
    let updated = receipt.clone();
    save_receipts(project_root, &receipts).await?;
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitelist_rejects_escapes_and_source_paths() {
        assert!(config_patch_target_allowed(".refact/knowledge/handbook/doc.md").is_ok());
        assert!(config_patch_target_allowed(".refact/commands/ship.yaml").is_ok());
        assert!(config_patch_target_allowed(".refact/buddy/settings.json").is_err());
        assert!(config_patch_target_allowed(".refact/buddy/receipts.json").is_err());
        assert!(config_patch_target_allowed("src/main.rs").is_err());
        assert!(config_patch_target_allowed(".refact/../src/main.rs").is_err());
        assert!(config_patch_target_allowed("/etc/passwd").is_err());
        assert!(config_patch_target_allowed(".refact/tool.sh").is_err());
        assert!(config_patch_target_allowed(".refact").is_err());
    }

    #[tokio::test]
    async fn apply_and_undo_round_trip_with_pre_image() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let rel = ".refact/knowledge/test.yaml";
        fs::create_dir_all(root.join(".refact/knowledge"))
            .await
            .unwrap();
        fs::write(root.join(rel), "original: true\n").await.unwrap();

        let receipt = apply_config_patch(root, rel, "patched: true\n")
            .await
            .unwrap();
        assert_eq!(
            fs::read_to_string(root.join(rel)).await.unwrap(),
            "patched: true\n"
        );
        assert_eq!(receipt.pre_image.as_deref(), Some("original: true\n"));

        let undone = undo_receipt(root, &receipt.id).await.unwrap();
        assert!(undone.undone);
        assert_eq!(
            fs::read_to_string(root.join(rel)).await.unwrap(),
            "original: true\n"
        );
        assert!(undo_receipt(root, &receipt.id).await.is_err());
    }

    #[tokio::test]
    async fn undo_removes_file_created_by_patch() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let rel = ".refact/knowledge/handbook/new.md";

        let receipt = apply_config_patch(root, rel, "# fresh\n").await.unwrap();
        assert!(receipt.pre_image.is_none());
        assert!(root.join(rel).exists());

        undo_receipt(root, &receipt.id).await.unwrap();
        assert!(!root.join(rel).exists());
    }

    #[test]
    fn whitelist_rejects_case_and_normalization_aliases() {
        assert!(config_patch_target_allowed(".refact/Buddy/settings.json").is_err());
        assert!(config_patch_target_allowed(".refact/BUDDY/settings.json").is_err());
        assert!(config_patch_target_allowed(".refact/buddy./settings.json").is_err());
        assert!(config_patch_target_allowed(".refact/buddy /settings.json").is_err());
        assert!(config_patch_target_allowed(".refact/knowledge/ok.md").is_ok());
    }

    #[test]
    fn undo_target_allows_legacy_buddy_paths_but_keeps_shape_checks() {
        assert!(undo_target_allowed(".refact/buddy/legacy.json").is_ok());
        assert!(undo_target_allowed(".refact/knowledge/doc.md").is_ok());
        assert!(undo_target_allowed("/etc/passwd").is_err());
        assert!(undo_target_allowed(".refact/../src/main.rs").is_err());
        assert!(undo_target_allowed("src/main.rs").is_err());
    }

    #[tokio::test]
    async fn undo_restores_legacy_buddy_receipt() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let rel = ".refact/buddy/legacy.json";
        fs::create_dir_all(root.join(".refact/buddy"))
            .await
            .unwrap();
        fs::write(root.join(rel), "{\"patched\":true}\n")
            .await
            .unwrap();
        let legacy = BuddyReceipt {
            id: "legacy-receipt".to_string(),
            action_kind: "apply_config_patch".to_string(),
            target_path: rel.to_string(),
            pre_image: Some("{\"original\":true}\n".to_string()),
            created_at: Utc::now(),
            undone: false,
            undone_at: None,
        };
        save_receipts(root, &[legacy]).await.unwrap();

        let undone = undo_receipt(root, "legacy-receipt").await.unwrap();
        assert!(undone.undone);
        assert_eq!(
            fs::read_to_string(root.join(rel)).await.unwrap(),
            "{\"original\":true}\n"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn apply_rejects_symlink_into_buddy_internal_state() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join(".refact/buddy"))
            .await
            .unwrap();
        tokio::fs::symlink(root.join(".refact/buddy"), root.join(".refact/knowledge"))
            .await
            .unwrap();
        let err = apply_config_patch(root, ".refact/knowledge/handbook/x.md", "# nope\n")
            .await
            .unwrap_err();
        assert!(err.contains("internal state"), "unexpected error: {}", err);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn apply_rejects_symlink_escaping_project_root() {
        let outside = tempfile::tempdir().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join(".refact")).await.unwrap();
        tokio::fs::symlink(outside.path(), root.join(".refact/kb"))
            .await
            .unwrap();
        let err = apply_config_patch(root, ".refact/kb/x.md", "# nope\n")
            .await
            .unwrap_err();
        assert!(
            err.contains("outside the project root"),
            "unexpected error: {}",
            err
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn undo_rejects_symlink_escaping_project_root() {
        let outside = tempfile::tempdir().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join(".refact/buddy"))
            .await
            .unwrap();
        tokio::fs::symlink(outside.path(), root.join(".refact/escape"))
            .await
            .unwrap();
        let receipt = BuddyReceipt {
            id: "escape-receipt".to_string(),
            action_kind: "apply_config_patch".to_string(),
            target_path: ".refact/escape/x.md".to_string(),
            pre_image: Some("old\n".to_string()),
            created_at: Utc::now(),
            undone: false,
            undone_at: None,
        };
        save_receipts(root, &[receipt]).await.unwrap();
        let err = undo_receipt(root, "escape-receipt").await.unwrap_err();
        assert!(
            err.contains("outside the project root"),
            "unexpected error: {}",
            err
        );
    }

    #[tokio::test]
    async fn receipts_persist_and_cap() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for i in 0..3 {
            apply_config_patch(root, ".refact/commands/x.yaml", &format!("v: {}\n", i))
                .await
                .unwrap();
        }
        let receipts = load_receipts(root).await;
        assert_eq!(receipts.len(), 3);
        assert_eq!(receipts[1].pre_image.as_deref(), Some("v: 0\n"));
    }
}
