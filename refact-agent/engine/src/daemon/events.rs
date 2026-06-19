use std::collections::VecDeque;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::sync::{broadcast, Mutex};

const DEFAULT_RING_CAPACITY: usize = 4096;
const DEFAULT_BROADCAST_CAPACITY: usize = 1024;
const EVENTS_ROTATE_BYTES: u64 = 10 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DaemonEvent {
    #[serde(default)]
    pub seq: u64,
    pub ts_ms: u64,
    pub kind: String,
    pub project_id: Option<String>,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EventReplay {
    pub events: Vec<DaemonEvent>,
    pub gap: Option<EventReplayGap>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventReplayGap {
    pub requested_after_seq: u64,
    pub oldest_seq: u64,
    pub latest_seq: u64,
}

struct EventBusState {
    ring: VecDeque<DaemonEvent>,
    next_seq: u64,
}

#[derive(Clone)]
pub struct EventBus {
    state: Arc<Mutex<EventBusState>>,
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
        Self::new_with_capacities(
            events_path,
            capacity,
            capacity.max(16).min(DEFAULT_BROADCAST_CAPACITY),
        )
    }

    fn new_with_capacities(
        events_path: PathBuf,
        capacity: usize,
        broadcast_capacity: usize,
    ) -> Self {
        let rotated_path = events_path.with_file_name("events.jsonl.1");
        let capacity = capacity.max(1);
        let (tx, _) = broadcast::channel(broadcast_capacity.max(1));
        Self {
            state: Arc::new(Mutex::new(EventBusState {
                ring: VecDeque::with_capacity(capacity),
                next_seq: 0,
            })),
            tx,
            events_path,
            rotated_path,
            capacity,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<DaemonEvent> {
        self.tx.subscribe()
    }

    pub async fn snapshot(&self) -> Vec<DaemonEvent> {
        self.state.lock().await.ring.iter().cloned().collect()
    }

    pub async fn replay_after(&self, after_seq: u64) -> EventReplay {
        let state = self.state.lock().await;
        let oldest_seq = state.ring.front().map(|event| event.seq);
        let latest_seq = state
            .ring
            .back()
            .map(|event| event.seq)
            .unwrap_or(state.next_seq);
        let gap = oldest_seq
            .filter(|oldest_seq| {
                after_seq < latest_seq && after_seq.saturating_add(1) < *oldest_seq
            })
            .map(|oldest_seq| EventReplayGap {
                requested_after_seq: after_seq,
                oldest_seq,
                latest_seq,
            });
        let events = state
            .ring
            .iter()
            .filter(|event| event.seq > after_seq)
            .cloned()
            .collect();
        EventReplay { events, gap }
    }

    pub async fn emit(
        &self,
        kind: impl Into<String>,
        project_id: Option<String>,
        payload: Value,
    ) -> Result<DaemonEvent, String> {
        let mut event = DaemonEvent {
            seq: 0,
            ts_ms: crate::daemon::state::now_ms(),
            kind: kind.into(),
            project_id,
            payload: redact_value(payload),
        };
        {
            let mut state = self.state.lock().await;
            state.next_seq = state.next_seq.saturating_add(1);
            event.seq = state.next_seq;
            while state.ring.len() >= self.capacity {
                state.ring.pop_front();
            }
            state.ring.push_back(event.clone());
            let _ = self.tx.send(event.clone());
        }
        if let Err(error) = append_jsonl(&self.events_path, &self.rotated_path, &event).await {
            tracing::warn!("failed to persist daemon event {}: {error}", event.seq);
        }
        Ok(event)
    }
}

fn redact_value(value: Value) -> Value {
    match value {
        Value::String(value) => Value::String(crate::daemon::auth::redact_daemon_token(&value)),
        Value::Array(values) => Value::Array(values.into_iter().map(redact_value).collect()),
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, redact_value(value)))
                .collect::<Map<_, _>>(),
        ),
        other => other,
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
        assert_eq!(snapshot[0].seq, 2);
        assert_eq!(snapshot[1].seq, 3);
        assert_eq!(snapshot[0].kind, "two");
        assert_eq!(snapshot[1].kind, "three");
        assert_eq!(rx.recv().await.unwrap().kind, "one");
        let content = tokio::fs::read_to_string(path).await.unwrap();
        assert_eq!(content.lines().count(), 3);
    }

    #[tokio::test]
    async fn daemon_event_bus_redacts_token_bearing_payloads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let bus = EventBus::new(path.clone());
        let event = bus
            .emit(
                "proxy_worker_unreachable",
                Some("project".to_string()),
                json!({"error": "GET /p/project/v1?daemon_token=secret-token failed"}),
            )
            .await
            .unwrap();

        assert!(!event.payload.to_string().contains("secret-token"));
        assert!(event.payload.to_string().contains("<redacted>"));
        let content = tokio::fs::read_to_string(path).await.unwrap();
        assert!(!content.contains("secret-token"));
        assert!(content.contains("<redacted>"));
    }

    #[tokio::test]
    async fn daemon_event_bus_lagged_subscriber_can_replay_recent_events() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let bus = EventBus::new_with_capacities(path, 8, 1);
        let mut rx = bus.subscribe();

        for idx in 1..=4 {
            bus.emit(format!("event-{idx}"), None, json!({"n": idx}))
                .await
                .unwrap();
        }

        assert!(matches!(
            rx.recv().await,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_))
        ));
        let replay = bus.replay_after(0).await;
        assert!(replay.gap.is_none());
        assert_eq!(
            replay
                .events
                .iter()
                .map(|event| event.seq)
                .collect::<Vec<_>>(),
            vec![1, 2, 3, 4]
        );
    }

    #[tokio::test]
    async fn daemon_event_bus_replay_reports_gap_when_cursor_is_too_old() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events.jsonl");
        let bus = EventBus::new_with_capacity(path, 2);

        bus.emit("one", None, json!({})).await.unwrap();
        bus.emit("two", None, json!({})).await.unwrap();
        bus.emit("three", None, json!({})).await.unwrap();

        let replay = bus.replay_after(0).await;
        assert_eq!(
            replay.gap,
            Some(EventReplayGap {
                requested_after_seq: 0,
                oldest_seq: 2,
                latest_seq: 3,
            })
        );
        assert_eq!(
            replay
                .events
                .iter()
                .map(|event| event.kind.as_str())
                .collect::<Vec<_>>(),
            vec!["two", "three"]
        );
    }

    #[tokio::test]
    async fn daemon_event_bus_persistence_failure_still_broadcasts() {
        let dir = tempfile::tempdir().unwrap();
        let bus = EventBus::new(dir.path().to_path_buf());
        let mut rx = bus.subscribe();

        let event = bus.emit("live", None, json!({"ok": true})).await.unwrap();

        assert_eq!(event.seq, 1);
        assert_eq!(rx.recv().await.unwrap().kind, "live");
        assert_eq!(bus.snapshot().await[0].kind, "live");
    }
}
