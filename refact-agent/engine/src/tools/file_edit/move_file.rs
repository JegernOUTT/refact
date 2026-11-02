use std::path::Path;
use std::sync::Arc;

use crate::file_activity::{begin_writes, WriteIntent, WriteKind};
use crate::files_in_workspace::{get_file_text_from_memory_or_disk, remove_memory_document_for_path};
use crate::global_context::GlobalContext;

pub async fn move_file(
    gcx: Arc<GlobalContext>,
    actor: Option<&str>,
    source: &Path,
    destination: &Path,
    overwrite: bool,
) -> Result<(String, String), String> {
    let source_meta = tokio::fs::symlink_metadata(source)
        .await
        .map_err(|error| error.to_string())?;
    if !source_meta.is_file() || source_meta.file_type().is_symlink() {
        return Err("file move requires a regular source file".into());
    }
    let prior_destination = match tokio::fs::read(destination).await {
        Ok(bytes) if overwrite => Some(bytes),
        Ok(_) => return Err("move destination already exists".into()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.to_string()),
    };
    let destination_intent = if prior_destination.is_none() {
        WriteIntent::Create
    } else if actor.is_some() {
        WriteIntent::Overwrite { authorized: false }
    } else {
        WriteIntent::Contextual
    };
    let mut guards = begin_writes(
        actor,
        &[
            (source.to_path_buf(), WriteIntent::Rename),
            (destination.to_path_buf(), destination_intent),
        ],
        "mv",
    )
    .await?;
    let destination_guard = guards.pop().ok_or("missing destination guard")?;
    let source_guard = guards.pop().ok_or("missing source guard")?;
    if source_guard.key() == destination_guard.key() {
        return Err("source and destination identify the same file".into());
    }
    let source_bytes = tokio::fs::read(source)
        .await
        .map_err(|error| error.to_string())?;
    let current_destination = match tokio::fs::read(destination).await {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.to_string()),
    };
    if prior_destination != current_destination {
        return Err("move destination changed while waiting; reread before retrying".into());
    }
    let source_text = std::str::from_utf8(&source_bytes).ok();
    if let Some(text) = source_text {
        let visible = get_file_text_from_memory_or_disk(gcx.clone(), &source.to_path_buf()).await?;
        if visible != text {
            return Err("source buffer differs from disk; save or reload before moving".into());
        }
    }
    if let Some(bytes) = &current_destination {
        if let Ok(text) = std::str::from_utf8(bytes) {
            let visible =
                get_file_text_from_memory_or_disk(gcx.clone(), &destination.to_path_buf()).await?;
            if visible != text {
                return Err(
                    "destination buffer differs from disk; save or reload before moving".into(),
                );
            }
        }
    }
    tokio::fs::rename(source, destination)
        .await
        .map_err(|error| error.to_string())?;
    destination_guard.commit_bytes(WriteKind::Renamed, Some(&source_bytes));
    source_guard.commit(WriteKind::Deleted, None);
    remove_memory_document_for_path(gcx.clone(), &source.to_path_buf()).await;
    remove_memory_document_for_path(gcx, &destination.to_path_buf()).await;
    Ok((
        source_text.unwrap_or_default().into(),
        current_destination
            .as_deref()
            .and_then(|bytes| std::str::from_utf8(bytes).ok())
            .unwrap_or_default()
            .into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_activity::{record_read, observations};

    #[tokio::test]
    async fn historical_reads_do_not_block_moves() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let destination = dir.path().join("destination");
        std::fs::write(&source, "old").unwrap();
        record_read(Some("stale-mover"), &source, "old", 1, 1, 1, None);
        std::fs::write(&source, "new").unwrap();
        move_file(
            gcx.clone(),
            Some("stale-mover"),
            &source,
            &destination,
            false,
        )
        .await
        .unwrap();
        assert!(!source.exists());
        assert_eq!(std::fs::read_to_string(&destination).unwrap(), "new");
        std::fs::write(&source, "new").unwrap();
        std::fs::write(&destination, "other").unwrap();
        record_read(Some("dest-mover"), &destination, "other", 1, 1, 1, None);
        std::fs::write(&destination, "external").unwrap();
        move_file(gcx, Some("dest-mover"), &source, &destination, true)
            .await
            .unwrap();
        assert_eq!(std::fs::read_to_string(&destination).unwrap(), "new");
        assert!(!source.exists());
    }

    #[tokio::test]
    async fn competing_moves_have_one_winner_and_accurate_binary_observations() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        let destination = dir.path().join("dest");
        std::fs::write(&a, [0xff, 1]).unwrap();
        std::fs::write(&b, [0xfe, 1]).unwrap();
        let (one, two) = tokio::join!(
            move_file(gcx.clone(), Some("one"), &a, &destination, false),
            move_file(gcx, Some("two"), &b, &destination, false),
        );
        assert_ne!(one.is_ok(), two.is_ok());
        let bytes = std::fs::read(&destination).unwrap();
        let writes = observations(Some(dir.path().to_str().unwrap())).writes;
        assert_eq!(writes.len(), 2);
        let created = writes
            .iter()
            .find(|write| write.path == destination.to_string_lossy())
            .unwrap();
        assert_eq!(
            created.digest_after.as_deref(),
            Some(crate::file_activity::digest_bytes(&bytes).as_str())
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn hardlink_alias_move_is_rejected_without_destroying_either_link() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let destination = dir.path().join("alias");
        std::fs::write(&source, "same").unwrap();
        std::fs::hard_link(&source, &destination).unwrap();
        assert!(move_file(gcx, None, &source, &destination, true)
            .await
            .is_err());
        assert_eq!(std::fs::read(&source).unwrap(), b"same");
        assert_eq!(std::fs::read(&destination).unwrap(), b"same");
    }
}
