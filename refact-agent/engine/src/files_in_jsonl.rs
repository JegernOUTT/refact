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
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        *gcx.documents_state.cache_dirty.lock().await = now;
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

async fn handle_jsonl_event(
    gcx: Arc<GlobalContext>,
    files_jsonl_path: &str,
    event_kind: EventKind,
) {
    match event_kind {
        EventKind::Any => {}
        EventKind::Access(_) => {}
        EventKind::Create(_) => {
            info!("files_jsonl_path {:?} was created", files_jsonl_path);
            enqueue_all_docs_from_jsonl_but_read_first(gcx.clone(), false, false).await;
        }
        EventKind::Modify(_) => {
            info!("files_jsonl_path {:?} was modified", files_jsonl_path);
            enqueue_all_docs_from_jsonl_but_read_first(gcx.clone(), false, false).await;
        }
        EventKind::Remove(_) => {
            info!("files_jsonl_path {:?} was removed", files_jsonl_path);
            enqueue_all_docs_from_jsonl(gcx.clone(), vec![], false, false).await;
        }
        EventKind::Other => {}
    }
}

pub async fn reload_if_jsonl_changes_background_task(gcx: Arc<GlobalContext>) {
    let (mut watcher, mut rx) = make_async_watcher().expect("Failed to make file watcher");
    let files_jsonl_path = gcx.cmdline.files_jsonl_path.clone();
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
        match res {
            Ok(event) => handle_jsonl_event(gcx.clone(), &files_jsonl_path, event.kind).await,
            Err(e) => info!("file watch error: {:?}", e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{CreateKind, DataChange, ModifyKind};

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

        std::fs::write(&jsonl_path, "{\"path\":\"first.rs\"}\n").unwrap();
        handle_jsonl_event(
            gcx.clone(),
            &jsonl_path_str,
            EventKind::Create(CreateKind::File),
        )
        .await;
        assert_eq!(tracked_jsonl_files(&gcx), vec![first_path]);

        std::fs::write(&jsonl_path, "{\"path\":\"second.rs\"}\n").unwrap();
        handle_jsonl_event(
            gcx.clone(),
            &jsonl_path_str,
            EventKind::Modify(ModifyKind::Data(DataChange::Content)),
        )
        .await;
        assert_eq!(tracked_jsonl_files(&gcx), vec![second_path]);
    }
}
