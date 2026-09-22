//! Non-destructive, content-addressed trajectory archives. Originals are never removed.
//! Native writers and callers must use the same mutation scope. Cross-process/native
//! rollout remains closed until all writers participate; this is not a filesystem CAS.
use std::path::{Component, Path, PathBuf};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

static MUTATION: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
tokio::task_local! { static HELD: (); }

pub async fn with_trajectory_mutation_lock<F: std::future::Future>(future: F) -> F::Output {
    if HELD.try_with(|_| ()).is_ok() {
        return future.await;
    }
    let _guard = MUTATION.lock().await;
    HELD.scope((), future).await
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArchiveIdentity {
    pub trajectory_id: String,
    pub relative_path: PathBuf,
    pub digest: String,
    pub byte_count: usize,
}

pub fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub fn checked_path(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|p| !matches!(p, Component::Normal(_)))
    {
        return Err("invalid archive-relative path".into());
    }
    if std::fs::canonicalize(root).map_err(|e| e.to_string())? != root {
        return Err("archive root must be canonical".into());
    }
    let mut path = root.to_path_buf();
    for part in relative.components() {
        path.push(part);
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err("archive path contains symlink".into())
            }
            Ok(_) => (),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(path)
}

pub fn prepare(
    root: &Path,
    relative: &Path,
    id: &str,
) -> Result<(ArchiveIdentity, Vec<u8>), String> {
    super::trajectories::validate_trajectory_id(id).map_err(|e| e.message)?;
    if !relative.starts_with(".refact/trajectories") {
        return Err("not a project trajectory".into());
    }
    let path = checked_path(root, relative)?;
    if std::fs::metadata(&path).map_err(|e| e.to_string())?.len() > 32 * 1024 * 1024 {
        return Err("trajectory exceeds bounded archive size".into());
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let json: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if json.get("id").and_then(|v| v.as_str()) != Some(id) || protected_metadata(&json) {
        return Err("trajectory identity, pin, goal or pending decision prevents archive".into());
    }
    Ok((
        ArchiveIdentity {
            trajectory_id: id.into(),
            relative_path: relative.into(),
            digest: digest(&bytes),
            byte_count: bytes.len(),
        },
        bytes,
    ))
}

pub fn protected_metadata(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(fields) => fields.iter().any(|(key, value)| {
            let protected = key.to_ascii_lowercase().contains("pinned")
                || matches!(
                    key.as_str(),
                    "goal"
                        | "active_goal"
                        | "goal_state"
                        | "goal_id"
                        | "task_meta"
                        | "work_item_id"
                        | "pending_questions"
                        | "pending_browser_context"
                );
            (protected
                && !value.is_null()
                && value != &serde_json::Value::Bool(false)
                && value != &serde_json::json!([]))
                || protected_metadata(value)
        }),
        serde_json::Value::Array(values) => values.iter().any(protected_metadata),
        _ => false,
    }
}

fn archive_relative(identity: &ArchiveIdentity) -> Result<PathBuf, String> {
    super::trajectories::validate_trajectory_id(&identity.trajectory_id).map_err(|e| e.message)?;
    if identity.digest.len() != 64 || !identity.digest.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("invalid archive digest".into());
    }
    Ok(PathBuf::from(".refact/trajectory_archive").join(format!(
        "{}-{}.json",
        identity.trajectory_id, identity.digest
    )))
}

/// Call while holding native mutation coordination. `protected` is an observed
/// owner reference snapshot, never a draft field. New references remain safe because
/// both original and immutable archived bytes are retained; this does not grant deletion.
pub fn archive(root: &Path, identity: &ArchiveIdentity, protected: bool) -> Result<(), String> {
    if protected {
        return Err("active or durably referenced trajectory".into());
    }
    let (current, bytes) = prepare(root, &identity.relative_path, &identity.trajectory_id)?;
    if current != *identity {
        return Err("trajectory version changed".into());
    }
    let path = checked_path(root, &archive_relative(identity)?)?;
    if path.exists() {
        return if std::fs::read(path).map_err(|e| e.to_string())? == bytes {
            Ok(())
        } else {
            Err("archive collision".into())
        };
    }
    std::fs::create_dir_all(path.parent().ok_or("missing archive parent")?)
        .map_err(|e| e.to_string())?;
    checked_path(root, &archive_relative(identity)?)?;
    // Unique temporary plus no-clobber hard link: a crash never exposes a partial archive.
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        use std::io::Write;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary).map_err(|e| e.to_string())?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        std::fs::hard_link(&temporary, &path).map_err(|e| e.to_string())?;
        Ok(())
    })();
    let _ = std::fs::remove_file(temporary);
    result
}

pub fn verify(root: &Path, identity: &ArchiveIdentity) -> Result<bool, String> {
    let bytes = std::fs::read(checked_path(root, &archive_relative(identity)?)?)
        .map_err(|e| e.to_string())?;
    Ok(bytes.len() == identity.byte_count && digest(&bytes) == identity.digest)
}

/// Restore only an absent original, or acknowledge an identical original. Never
/// overwrite a newer trajectory and never remove the retained archive evidence.
pub fn recover(root: &Path, identity: &ArchiveIdentity) -> Result<(), String> {
    if !verify(root, identity)? {
        return Err("archive post-image changed".into());
    }
    let archived_bytes = std::fs::read(checked_path(root, &archive_relative(identity)?)?)
        .map_err(|e| e.to_string())?;
    let json: serde_json::Value =
        serde_json::from_slice(&archived_bytes).map_err(|e| e.to_string())?;
    if json.get("id").and_then(|v| v.as_str()) != Some(identity.trajectory_id.as_str()) {
        return Err("archive identity changed".into());
    }
    let destination = checked_path(root, &identity.relative_path)?;
    if !identity.relative_path.starts_with(".refact/trajectories") {
        return Err("invalid recovery target".into());
    }
    let archive = checked_path(root, &archive_relative(identity)?)?;
    match std::fs::read(&destination) {
        Ok(bytes) => {
            if digest(&bytes) == identity.digest {
                Ok(())
            } else {
                Err("recovery conflicts with current trajectory".into())
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir_all(destination.parent().ok_or("missing trajectory parent")?)
                .map_err(|e| e.to_string())?;
            // Publish a distinct inode so subsequent native writes cannot alter evidence.
            let bytes = std::fs::read(archive).map_err(|e| e.to_string())?;
            let temporary = destination.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
            let result = (|| {
                use std::io::Write;
                let mut options = std::fs::OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                let mut file = options.open(&temporary).map_err(|e| e.to_string())?;
                file.write_all(&bytes).map_err(|e| e.to_string())?;
                file.sync_all().map_err(|e| e.to_string())?;
                std::fs::hard_link(&temporary, &destination).map_err(|e| e.to_string())
            })();
            let _ = std::fs::remove_file(temporary);
            result
        }
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn archive_coordination_blocks_native_publish_without_sleep() {
        use std::future::Future;
        use std::task::Poll;
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("chat.json");
        let temporary = dir.path().join("chat.tmp");
        tokio::fs::write(&temporary, b"new native bytes")
            .await
            .unwrap();
        with_trajectory_mutation_lock(async {
            let (started, ready) = tokio::sync::oneshot::channel();
            let tmp = temporary.clone();
            let dest = destination.clone();
            let writer = tokio::spawn(async move {
                let mut write =
                    Box::pin(super::super::trajectories::atomic_write_file(&tmp, &dest));
                std::future::poll_fn(|cx| {
                    assert!(matches!(write.as_mut().poll(cx), Poll::Pending));
                    Poll::Ready(())
                })
                .await;
                started.send(()).unwrap();
                write.await
            });
            ready.await.unwrap();
            assert!(!destination.exists());
            writer
        })
        .await
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            tokio::fs::read(destination).await.unwrap(),
            b"new native bytes"
        );
    }

    #[tokio::test]
    async fn owner_archive_preserves_exact_bytes_and_recovers_without_overwrite() {
        with_trajectory_mutation_lock(async {
            let dir = tempfile::tempdir().unwrap();
            let root = std::fs::canonicalize(dir.path()).unwrap();
            let relative = Path::new(".refact/trajectories/chat-a.json");
            let source = root.join(relative);
            std::fs::create_dir_all(source.parent().unwrap()).unwrap();
            let bytes = b"{\"id\":\"chat-a\",\"unknown\":17,\"messages\":[]}";
            std::fs::write(&source, bytes).unwrap();
            let (identity, _) = prepare(&root, relative, "chat-a").unwrap();
            assert!(archive(&root, &identity, true).is_err());
            assert!(!root.join(".refact/trajectory_archive").exists());
            archive(&root, &identity, false).unwrap();
            assert_eq!(std::fs::read(&source).unwrap(), bytes);
            std::fs::remove_file(&source).unwrap(); // scratch only: simulate native retention
            recover(&root, &identity).unwrap();
            assert_eq!(std::fs::read(&source).unwrap(), bytes);
            std::fs::write(&source, b"{\"id\":\"chat-a\",\"unknown\":18}").unwrap();
            assert!(archive(&root, &identity, false).is_err());
            assert!(recover(&root, &identity).is_err());
            assert!(verify(&root, &identity).unwrap());
        })
        .await;
    }
}
