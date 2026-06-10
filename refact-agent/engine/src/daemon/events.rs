use std::collections::VecDeque;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{broadcast, Mutex};

const DEFAULT_RING_CAPACITY: usize = 4096;
const EVENTS_ROTATE_BYTES: u64 = 10 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DaemonEvent {
    pub ts_ms: u64,
    pub kind: String,
    pub project_id: Option<String>,
    pub payload: Value,
}

#[derive(Clone)]
pub struct EventBus {
    ring: Arc<Mutex<VecDeque<DaemonEvent>>>,
    tx: broadcast::Sender<DaemonEvent>,
    events_path: PathBuf,
    rotated_path: PathBuf,
    capacity: usize,
}

impl EventBus {
    pub fn new(events_path: PathBuf) -> Self {
        Self::new_with_capacity(events_path, DEFAULT_RING_CAPACITY)
    }

    pub fn new_with_capacity(events_path: PathBuf, capacity: usize) -> Self {
        let rotated_path = events_path.with_file_name("events.jsonl.1");
        let (tx, _) = broadcast::channel(capacity.max(16));
        Self {
            ring: Arc::new(Mutex::new(VecDeque::with_capacity(capacity))),
            tx,
            events_path,
            rotated_path,
            capacity: capacity.max(1),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<DaemonEvent> {
        self.tx.subscribe()
    }

    pub async fn snapshot(&self) -> Vec<DaemonEvent> {
        self.ring.lock().await.iter().cloned().collect()
    }

    pub async fn emit(
        &self,
        kind: impl Into<String>,
        project_id: Option<String>,
        payload: Value,
    ) -> Result<DaemonEvent, String> {
        let event = DaemonEvent {
            ts_ms: crate::daemon::state::now_ms(),
            kind: kind.into(),
            project_id,
            payload,
        };
        {
            let mut ring = self.ring.lock().await;
            while ring.len() >= self.capacity {
                ring.pop_front();
            }
            ring.push_back(event.clone());
        }
        append_jsonl(&self.events_path, &self.rotated_path, &event).await?;
        let _ = self.tx.send(event.clone());
        Ok(event)
    }
}

async fn append_jsonl(path: &Path, rotated_path: &Path, event: &DaemonEvent) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| format!("failed to create {}: {error}", parent.display()))?;
    }
    if should_rotate(path).await? {
        match tokio::fs::remove_file(rotated_path).await {
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "failed to remove {}: {error}",
                    rotated_path.display()
                ));
            }
        }
        tokio::fs::rename(path, rotated_path)
            .await
            .map_err(|error| format!("failed to rotate {}: {error}", path.display()))?;
    }
    let line = serde_json::to_string(event)
        .map_err(|error| format!("failed to encode daemon event: {error}"))?;
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
        .map_err(|error| format!("failed to open {}: {error}", path.display()))?;
    use tokio::io::AsyncWriteExt;
    file.write_all(line.as_bytes())
        .await
        .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
    file.write_all(b"\n")
        .await
        .map_err(|error| format!("failed to write {}: {error}", path.display()))?;
    Ok(())
}

async fn should_rotate(path: &Path) -> Result<bool, String> {
    match tokio::fs::metadata(path).await {
        Ok(metadata) => Ok(metadata.len() >= EVENTS_ROTATE_BYTES),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("failed to stat {}: {error}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn daemon_event_bus_ring_jsonl_and_broadcast() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let bus = EventBus::new_with_capacity(path.clone(), 2);
        let mut rx = bus.subscribe();
        bus.emit("one", None, json!({"n": 1})).await.unwrap();
        bus.emit("two", Some("p".to_string()), json!({"n": 2}))
            .await
            .unwrap();
        bus.emit("three", None, json!({"n": 3})).await.unwrap();

        let snapshot = bus.snapshot().await;
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0].kind, "two");
        assert_eq!(snapshot[1].kind, "three");
        assert_eq!(rx.recv().await.unwrap().kind, "one");
        let content = tokio::fs::read_to_string(path).await.unwrap();
        assert_eq!(content.lines().count(), 3);
    }
}
