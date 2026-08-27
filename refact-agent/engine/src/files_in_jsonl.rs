use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use futures::channel::mpsc::{channel, Receiver};
use futures::{SinkExt, StreamExt};
use tracing::{info, error};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::Value;
use tokio::fs::File;
use tokio::io::AsyncBufReadExt;
use tokio::io::BufReader;

use crate::global_context::GlobalContext;

const JSONL_DEBOUNCE_WINDOW: tokio::time::Duration = tokio::time::Duration::from_millis(400);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JsonlFingerprint {
    Missing,
    Content(u64),
}

pub async fn enqueue_all_docs_from_jsonl(
    gcx: Arc<GlobalContext>,
    paths: Vec<PathBuf>,
    force: bool,
    vecdb_only: bool,
) {
    if paths.is_empty() {
        return;
    }
    let mut docs: Vec<String> = vec![];
    for d in paths.iter() {
        docs.push(d.to_string_lossy().to_string());
    }
    {
        crate::files_correction::mark_files_cache_dirty(&gcx.documents_state.cache_dirty).await;
        let jsonl_files = &mut gcx.documents_state.jsonl_files.lock().unwrap();
        jsonl_files.clear();
        jsonl_files.extend(paths);
    }
    crate::indexing_routing::route_index_enqueue(gcx.clone(), &docs, force, vecdb_only).await;
}

pub async fn enqueue_all_docs_from_jsonl_but_read_first(
    gcx: Arc<GlobalContext>,
    force: bool,
    vecdb_only: bool,
) {
    let paths = read_the_jsonl(gcx.clone()).await;
    enqueue_all_docs_from_jsonl(gcx.clone(), paths, force, vecdb_only).await;
}

async fn parse_jsonl(jsonl_path: &String) -> Result<Vec<PathBuf>, String> {
    if jsonl_path.is_empty() {
        return Ok(vec![]);
    }
    let file = File::open(jsonl_path)
        .await
        .map_err(|_| format!("File not found: {:?}", jsonl_path))?;
    let reader = BufReader::new(file);
    let base_path = PathBuf::from(jsonl_path)
        .parent()
        .or(Some(Path::new("/")))
        .unwrap()
        .to_path_buf();

    let mut lines = reader.lines();

    let mut paths = Vec::new();
    while let Some(line) = lines.next_line().await.transpose() {
        let line = line.map_err(|_| "Error reading line".to_string())?;
        if let Ok(value) = serde_json::from_str::<Value>(&line) {
            if value.is_object() {
                if let Some(filename) = value.get("path").and_then(|v| v.as_str()) {
                    // TODO: join, why it's there?
                    let path = base_path.join(filename);
                    paths.push(path);
                }
            }
        }
    }
    Ok(paths)
}

pub async fn read_the_jsonl(gcx: Arc<GlobalContext>) -> Vec<PathBuf> {
    let files_jsonl_path = gcx.cmdline.files_jsonl_path.clone();
    read_jsonl_path(&files_jsonl_path).await
}

async fn read_jsonl_path(files_jsonl_path: &str) -> Vec<PathBuf> {
    match parse_jsonl(&files_jsonl_path.to_string()).await {
        Ok(docs) => docs,
        Err(e) => {
            info!("invalid jsonl file {:?}: {:?}", files_jsonl_path, e);
            vec![]
        }
    }
}

fn make_async_watcher() -> notify::Result<(RecommendedWatcher, Receiver<notify::Result<Event>>)> {
    let (mut tx, rx) = channel(1);

    let watcher = RecommendedWatcher::new(
        move |res| {
            futures::executor::block_on(async {
                tx.send(res).await.unwrap();
            })
        },
        Config::default(),
    )?;

    Ok((watcher, rx))
}

async fn jsonl_fingerprint(files_jsonl_path: &str) -> JsonlFingerprint {
    match tokio::fs::read(files_jsonl_path).await {
        Ok(content) => {
            let mut hasher = DefaultHasher::new();
            content.hash(&mut hasher);
            JsonlFingerprint::Content(hasher.finish())
        }
        Err(_) => JsonlFingerprint::Missing,
    }
}

fn is_relevant_jsonl_event(event: &Event) -> bool {
    matches!(
        &event.kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    ) && event
        .paths
        .iter()
        .any(|path| !crate::file_filter::is_transient_tmp_path(path))
}

async fn handle_jsonl_event_if_changed(
    gcx: Arc<GlobalContext>,
    files_jsonl_path: &str,
    last_fingerprint: &mut JsonlFingerprint,
) {
    let fingerprint = jsonl_fingerprint(files_jsonl_path).await;
    if fingerprint == *last_fingerprint {
        return;
    }
    *last_fingerprint = fingerprint;
    match fingerprint {
        JsonlFingerprint::Missing => {
            info!("files_jsonl_path {:?} was removed", files_jsonl_path);
            enqueue_all_docs_from_jsonl(gcx, vec![], false, false).await;
        }
        JsonlFingerprint::Content(_) => {
            info!("files_jsonl_path {:?} was modified", files_jsonl_path);
            enqueue_all_docs_from_jsonl_but_read_first(gcx, false, false).await;
        }
    }
}

pub async fn reload_if_jsonl_changes_background_task(gcx: Arc<GlobalContext>) {
    let (mut watcher, mut rx) = make_async_watcher().expect("Failed to make file watcher");
    let files_jsonl_path = gcx.cmdline.files_jsonl_path.clone();
    let mut last_fingerprint = jsonl_fingerprint(&files_jsonl_path).await;
    enqueue_all_docs_from_jsonl_but_read_first(gcx.clone(), false, false).await;
    if watcher
        .watch(
            &PathBuf::from(files_jsonl_path.clone()),
            RecursiveMode::Recursive,
        )
        .is_err()
    {
        error!(
            "file watcher {:?} failed to start watching",
            files_jsonl_path
        );
        return;
    }
    while let Some(res) = rx.next().await {
        let mut pending_kind = match res {
            Ok(event) if is_relevant_jsonl_event(&event) => Some(event.kind),
            Ok(_) => None,
            Err(e) => {
                info!("file watch error: {:?}", e);
                None
            }
        };
        if pending_kind.is_none() {
            continue;
        }
        loop {
            match tokio::time::timeout(JSONL_DEBOUNCE_WINDOW, rx.next()).await {
                Ok(Some(Ok(event))) if is_relevant_jsonl_event(&event) => {
                    pending_kind = Some(event.kind);
                }
                Ok(Some(Ok(_))) => {}
                Ok(Some(Err(e))) => info!("file watch error: {:?}", e),
                Ok(None) | Err(_) => break,
            }
        }
        if pending_kind.is_some() {
            handle_jsonl_event_if_changed(gcx.clone(), &files_jsonl_path, &mut last_fingerprint)
                .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracked_jsonl_files(gcx: &Arc<GlobalContext>) -> Vec<PathBuf> {
        gcx.documents_state.jsonl_files.lock().unwrap().clone()
    }

    #[tokio::test]
    async fn modified_jsonl_event_rereads_contents() {
        let temp = tempfile::tempdir().unwrap();
        let jsonl_path = temp.path().join("files.jsonl");
        let first_path = temp.path().join("first.rs");
        let second_path = temp.path().join("second.rs");
        let jsonl_path_str = jsonl_path.to_string_lossy().to_string();
        let mut gcx = crate::global_context::tests::make_test_gcx().await;
        Arc::get_mut(&mut gcx)
            .expect("test owns gcx")
            .cmdline
            .files_jsonl_path = jsonl_path_str.clone();

        let mut fingerprint = jsonl_fingerprint(&jsonl_path_str).await;
        assert_eq!(fingerprint, JsonlFingerprint::Missing);

        std::fs::write(&jsonl_path, "{\"path\":\"first.rs\"}\n").unwrap();
        handle_jsonl_event_if_changed(gcx.clone(), &jsonl_path_str, &mut fingerprint).await;
        assert_eq!(tracked_jsonl_files(&gcx), vec![first_path.clone()]);

        std::fs::write(&jsonl_path, "{\"path\":\"second.rs\"}\n").unwrap();
        handle_jsonl_event_if_changed(gcx.clone(), &jsonl_path_str, &mut fingerprint).await;
        assert_eq!(tracked_jsonl_files(&gcx), vec![second_path]);

        gcx.documents_state.jsonl_files.lock().unwrap().clear();
        handle_jsonl_event_if_changed(gcx.clone(), &jsonl_path_str, &mut fingerprint).await;
        assert!(tracked_jsonl_files(&gcx).is_empty());

        std::fs::remove_file(&jsonl_path).unwrap();
        handle_jsonl_event_if_changed(gcx.clone(), &jsonl_path_str, &mut fingerprint).await;
        assert_eq!(fingerprint, JsonlFingerprint::Missing);
        assert!(tracked_jsonl_files(&gcx).is_empty());
    }
}
