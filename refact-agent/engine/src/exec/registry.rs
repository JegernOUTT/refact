use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;

use crate::exec::transcript::ExecTranscript;
use crate::exec::types::{
    current_timestamp_ms, ExecOutputChunk, ExecOutputStream, ExecProcessFilter, ExecProcessId,
    ExecProcessMeta, ExecProcessSnapshot, ExecReadResult, ExecServiceLookup, ExecStatus,
};

struct ExecProcessRecord {
    snapshot: ExecProcessSnapshot,
    transcript: ExecTranscript,
    child: Option<tokio::process::Child>,
}

impl ExecProcessRecord {
    fn new(meta: ExecProcessMeta, transcript_limit_bytes: usize) -> Self {
        let process_id = meta.process_id.clone();
        Self {
            snapshot: ExecProcessSnapshot::new(meta),
            transcript: ExecTranscript::new(process_id, transcript_limit_bytes),
            child: None,
        }
    }

    fn with_child(
        meta: ExecProcessMeta,
        transcript_limit_bytes: usize,
        child: tokio::process::Child,
    ) -> Self {
        let mut record = Self::new(meta, transcript_limit_bytes);
        record.child = Some(child);
        record
    }

    fn set_status(&mut self, status: ExecStatus) {
        if self.snapshot.status == status {
            return;
        }
        if self.snapshot.status.is_terminal() {
            return;
        }
        if matches!(status, ExecStatus::Running) && self.snapshot.meta.started_at_ms.is_none() {
            self.snapshot.meta.started_at_ms = Some(current_timestamp_ms());
        }
        if status.is_terminal() && self.snapshot.meta.ended_at_ms.is_none() {
            self.snapshot.meta.ended_at_ms = Some(current_timestamp_ms());
        }
        self.snapshot.status = status;
    }
}

struct ExecCleanupTarget {
    snapshot: ExecProcessSnapshot,
    child: Option<tokio::process::Child>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecShutdownCleanupSummary {
    pub removed_count: usize,
    pub child_stop_attempts: usize,
    pub child_stopped_count: usize,
    pub child_failed_count: usize,
    pub child_timed_out_count: usize,
}

enum ExecCleanupOutcome {
    NoChild,
    Stopped,
    Failed(String),
    TimedOut,
}

#[derive(Clone, Default)]
pub struct ExecRegistry {
    records: Arc<Mutex<HashMap<ExecProcessId, ExecProcessRecord>>>,
}

impl ExecRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn register(
        &self,
        meta: ExecProcessMeta,
        transcript_limit_bytes: usize,
    ) -> ExecProcessSnapshot {
        let process_id = meta.process_id.clone();
        let record = ExecProcessRecord::new(meta, transcript_limit_bytes);
        let snapshot = record.snapshot.clone();
        let mut records = self.records.lock().await;
        records.insert(process_id, record);
        snapshot
    }

    pub async fn register_with_child(
        &self,
        meta: ExecProcessMeta,
        transcript_limit_bytes: usize,
        child: tokio::process::Child,
    ) -> ExecProcessSnapshot {
        let process_id = meta.process_id.clone();
        let record = ExecProcessRecord::with_child(meta, transcript_limit_bytes, child);
        let snapshot = record.snapshot.clone();
        let mut records = self.records.lock().await;
        records.insert(process_id, record);
        snapshot
    }

    pub async fn get(&self, process_id: &ExecProcessId) -> Option<ExecProcessSnapshot> {
        let records = self.records.lock().await;
        records
            .get(process_id)
            .map(|record| record.snapshot.clone())
    }

    pub async fn list(&self, filter: ExecProcessFilter) -> Vec<ExecProcessSnapshot> {
        let records = self.records.lock().await;
        let mut snapshots = records
            .values()
            .filter(|record| record.snapshot.meta.owner.matches_filter(&filter))
            .filter(|record| {
                filter
                    .status
                    .map(|status| record.snapshot.status.kind() == status)
                    .unwrap_or(true)
            })
            .map(|record| record.snapshot.clone())
            .collect::<Vec<_>>();
        snapshots.sort_by(|a, b| a.meta.created_at_ms.cmp(&b.meta.created_at_ms));
        snapshots
    }

    pub async fn find_service(&self, lookup: ExecServiceLookup) -> Option<ExecProcessSnapshot> {
        let records = self.records.lock().await;
        records
            .values()
            .filter(|record| record.snapshot.meta.owner.matches_service_lookup(&lookup))
            .max_by_key(|record| record.snapshot.meta.created_at_ms)
            .map(|record| record.snapshot.clone())
    }

    pub async fn append_output(
        &self,
        process_id: &ExecProcessId,
        stream: ExecOutputStream,
        text: String,
    ) -> Result<ExecOutputChunk, String> {
        let mut records = self.records.lock().await;
        let record = records
            .get_mut(process_id)
            .ok_or_else(|| format!("process not found: {process_id}"))?;
        Ok(record.transcript.append_chunk(stream, text))
    }

    pub async fn read(
        &self,
        process_id: &ExecProcessId,
        since_seq: u64,
        limit: Option<usize>,
    ) -> ExecReadResult {
        let records = self.records.lock().await;
        records
            .get(process_id)
            .map(|record| record.transcript.read(since_seq, limit))
            .unwrap_or_else(|| ExecReadResult::not_found(process_id.clone(), since_seq))
    }

    pub async fn set_status(
        &self,
        process_id: &ExecProcessId,
        status: ExecStatus,
    ) -> Result<ExecProcessSnapshot, String> {
        let mut records = self.records.lock().await;
        let record = records
            .get_mut(process_id)
            .ok_or_else(|| format!("process not found: {process_id}"))?;
        record.set_status(status);
        Ok(record.snapshot.clone())
    }

    pub async fn mark_started(
        &self,
        process_id: &ExecProcessId,
    ) -> Result<ExecProcessSnapshot, String> {
        self.set_status(process_id, ExecStatus::Running).await
    }

    pub async fn mark_exited(
        &self,
        process_id: &ExecProcessId,
        exit_code: Option<i32>,
    ) -> Result<ExecProcessSnapshot, String> {
        self.set_status(process_id, ExecStatus::Exited { exit_code })
            .await
    }

    pub async fn mark_failed(
        &self,
        process_id: &ExecProcessId,
        message: String,
    ) -> Result<ExecProcessSnapshot, String> {
        self.set_status(process_id, ExecStatus::Failed { message })
            .await
    }

    pub async fn mark_killed(
        &self,
        process_id: &ExecProcessId,
    ) -> Result<ExecProcessSnapshot, String> {
        self.set_status(process_id, ExecStatus::Killed).await
    }

    pub async fn mark_timed_out(
        &self,
        process_id: &ExecProcessId,
    ) -> Result<ExecProcessSnapshot, String> {
        self.set_status(process_id, ExecStatus::TimedOut).await
    }

    pub async fn remove(&self, process_id: &ExecProcessId) -> Option<ExecProcessSnapshot> {
        let mut records = self.records.lock().await;
        records.remove(process_id).map(|record| record.snapshot)
    }

    pub async fn remove_by_owner(&self, filter: ExecProcessFilter) -> Vec<ExecProcessSnapshot> {
        let mut records = self.records.lock().await;
        let process_ids = records
            .iter()
            .filter(|(_, record)| record.snapshot.meta.owner.matches_filter(&filter))
            .filter(|(_, record)| {
                filter
                    .status
                    .map(|status| record.snapshot.status.kind() == status)
                    .unwrap_or(true)
            })
            .map(|(process_id, _)| process_id.clone())
            .collect::<Vec<_>>();
        process_ids
            .into_iter()
            .filter_map(|process_id| records.remove(&process_id).map(|record| record.snapshot))
            .collect()
    }

    pub async fn cleanup_shutdown(&self, timeout: Duration) -> ExecShutdownCleanupSummary {
        let targets = self.drain_active_targets().await;
        let mut summary = ExecShutdownCleanupSummary {
            removed_count: targets.len(),
            ..ExecShutdownCleanupSummary::default()
        };
        let outcomes = futures::future::join_all(
            targets
                .into_iter()
                .map(|target| cleanup_target(target, timeout)),
        )
        .await;
        for (process_id, short_description, outcome) in outcomes {
            match outcome {
                ExecCleanupOutcome::NoChild => {}
                ExecCleanupOutcome::Stopped => {
                    summary.child_stop_attempts += 1;
                    summary.child_stopped_count += 1;
                }
                ExecCleanupOutcome::Failed(message) => {
                    summary.child_stop_attempts += 1;
                    summary.child_failed_count += 1;
                    tracing::warn!(
                        "exec cleanup: failed to stop {} ({}): {}",
                        process_id,
                        short_description,
                        message
                    );
                }
                ExecCleanupOutcome::TimedOut => {
                    summary.child_stop_attempts += 1;
                    summary.child_timed_out_count += 1;
                    tracing::warn!(
                        "exec cleanup: process {} ({}) did not stop within {:?}",
                        process_id,
                        short_description,
                        timeout
                    );
                }
            }
        }
        summary
    }

    async fn drain_active_targets(&self) -> Vec<ExecCleanupTarget> {
        let mut records = self.records.lock().await;
        let process_ids = records
            .iter()
            .filter(|(_, record)| !record.snapshot.status.is_terminal() || record.child.is_some())
            .map(|(process_id, _)| process_id.clone())
            .collect::<Vec<_>>();
        process_ids
            .into_iter()
            .filter_map(|process_id| {
                let mut record = records.remove(&process_id)?;
                record.set_status(ExecStatus::Killed);
                Some(ExecCleanupTarget {
                    snapshot: record.snapshot,
                    child: record.child.take(),
                })
            })
            .collect()
    }
}

async fn cleanup_target(
    target: ExecCleanupTarget,
    timeout: Duration,
) -> (ExecProcessId, String, ExecCleanupOutcome) {
    let process_id = target.snapshot.meta.process_id.clone();
    let short_description = target.snapshot.meta.short_description.clone();
    let Some(mut child) = target.child else {
        return (process_id, short_description, ExecCleanupOutcome::NoChild);
    };

    let mut kill_error = None;
    #[cfg(unix)]
    {
        if let Some(pid) = child.id() {
            let result = unsafe { libc::kill(-(pid as i32), libc::SIGKILL) };
            if result != 0 {
                kill_error = Some(std::io::Error::last_os_error().to_string());
            }
        }
    }
    if let Err(err) = child.start_kill() {
        kill_error = Some(err.to_string());
    }
    let wait_result = tokio::time::timeout(timeout, child.wait()).await;
    let outcome = match wait_result {
        Ok(Ok(_)) => ExecCleanupOutcome::Stopped,
        Ok(Err(err)) => ExecCleanupOutcome::Failed(err.to_string()),
        Err(_) => ExecCleanupOutcome::TimedOut,
    };
    let outcome = match (outcome, kill_error) {
        (ExecCleanupOutcome::Stopped, _) => ExecCleanupOutcome::Stopped,
        (_, Some(message)) => ExecCleanupOutcome::Failed(message),
        (outcome, None) => outcome,
    };
    (process_id, short_description, outcome)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::exec::transcript::DEFAULT_MAX_BYTES;
    use crate::exec::types::{ExecMode, ExecStatusKind};

    fn meta(process_id: &str, mode: ExecMode, command: &str) -> ExecProcessMeta {
        ExecProcessMeta::new(mode, command.to_string())
            .with_process_id(ExecProcessId(process_id.to_string()))
    }

    #[tokio::test]
    async fn test_create_get_list() {
        let registry = ExecRegistry::new();
        let first = registry
            .register(
                meta("exec_one", ExecMode::Foreground, "echo one"),
                DEFAULT_MAX_BYTES,
            )
            .await;
        let second = registry
            .register(
                meta("exec_two", ExecMode::Background, "sleep 10"),
                DEFAULT_MAX_BYTES,
            )
            .await;

        assert_eq!(first.status, ExecStatus::Starting);
        assert_eq!(second.status, ExecStatus::Starting);
        assert_eq!(
            registry.get(&first.meta.process_id).await,
            Some(first.clone())
        );
        assert_eq!(
            registry
                .get(&ExecProcessId("exec_missing".to_string()))
                .await,
            None
        );

        let listed = registry.list(ExecProcessFilter::default()).await;
        assert_eq!(listed.len(), 2);
        assert!(listed.contains(&first));
        assert!(listed.contains(&second));
    }

    #[tokio::test]
    async fn test_list_filters_owner_and_status() {
        let registry = ExecRegistry::new();
        let first = meta("exec_one", ExecMode::Service, "server")
            .with_chat_id("chat-a")
            .with_service_name("api")
            .with_workspace(PathBuf::from("/workspace-a"));
        let second = meta("exec_two", ExecMode::Service, "server")
            .with_chat_id("chat-b")
            .with_service_name("api")
            .with_workspace(PathBuf::from("/workspace-b"));
        registry.register(first, DEFAULT_MAX_BYTES).await;
        let second_snapshot = registry.register(second, DEFAULT_MAX_BYTES).await;
        registry
            .mark_started(&second_snapshot.meta.process_id)
            .await
            .unwrap();

        let filtered = registry
            .list(ExecProcessFilter {
                chat_id: Some("chat-b".to_string()),
                tool_call_id: None,
                service_name: Some("api".to_string()),
                workspace: Some(PathBuf::from("/workspace-b")),
                status: Some(ExecStatusKind::Running),
            })
            .await;
        assert_eq!(filtered.len(), 1);
        assert_eq!(
            filtered[0].meta.process_id,
            ExecProcessId("exec_two".to_string())
        );
    }

    #[tokio::test]
    async fn test_output_append_and_read_cursor() {
        let registry = ExecRegistry::new();
        let snapshot = registry
            .register(meta("exec_out", ExecMode::Foreground, "echo hi"), 4096)
            .await;
        let process_id = snapshot.meta.process_id;

        let first = registry
            .append_output(&process_id, ExecOutputStream::Stdout, "hello".to_string())
            .await
            .unwrap();
        let second = registry
            .append_output(&process_id, ExecOutputStream::Stderr, "warn".to_string())
            .await
            .unwrap();
        assert_eq!(first.seq, 0);
        assert_eq!(second.seq, 1);

        let all = registry.read(&process_id, 0, None).await;
        assert!(all.found);
        assert_eq!(all.chunks.len(), 2);
        assert_eq!(all.next_seq, 2);
        assert_eq!(all.latest_seq, 2);

        let partial = registry.read(&process_id, 1, Some(1)).await;
        assert_eq!(partial.chunks, vec![second]);
        assert_eq!(partial.next_seq, 2);
    }

    #[tokio::test]
    async fn test_read_missing_process() {
        let registry = ExecRegistry::new();
        let result = registry
            .read(&ExecProcessId("exec_missing".to_string()), 7, None)
            .await;
        assert!(!result.found);
        assert_eq!(result.process_id, ExecProcessId("exec_missing".to_string()));
        assert_eq!(result.since_seq, 7);
    }

    #[tokio::test]
    async fn test_append_missing_process_is_error() {
        let registry = ExecRegistry::new();
        let err = registry
            .append_output(
                &ExecProcessId("exec_missing".to_string()),
                ExecOutputStream::Stdout,
                "hello".to_string(),
            )
            .await
            .unwrap_err();
        assert_eq!(err, "process not found: exec_missing");
    }

    #[tokio::test]
    async fn test_status_transition_timestamps() {
        let registry = ExecRegistry::new();
        let snapshot = registry
            .register(
                meta("exec_life", ExecMode::Background, "sleep 1"),
                DEFAULT_MAX_BYTES,
            )
            .await;
        let process_id = snapshot.meta.process_id;

        let running = registry.mark_started(&process_id).await.unwrap();
        assert_eq!(running.status, ExecStatus::Running);
        let started_at = running.meta.started_at_ms.expect("started timestamp");
        assert!(running.meta.ended_at_ms.is_none());

        let exited = registry.mark_exited(&process_id, Some(0)).await.unwrap();
        assert_eq!(exited.status, ExecStatus::Exited { exit_code: Some(0) });
        assert_eq!(exited.meta.started_at_ms, Some(started_at));
        assert!(exited.meta.ended_at_ms.is_some());
    }

    #[tokio::test]
    async fn test_terminal_status_transition_is_idempotent() {
        let registry = ExecRegistry::new();
        let snapshot = registry
            .register(
                meta("exec_race", ExecMode::Background, "sleep 1"),
                DEFAULT_MAX_BYTES,
            )
            .await;
        let process_id = snapshot.meta.process_id;

        let exited = registry.mark_exited(&process_id, Some(0)).await.unwrap();
        let ended_at = exited.meta.ended_at_ms;
        let killed = registry.mark_killed(&process_id).await.unwrap();
        let failed = registry
            .mark_failed(&process_id, "late failure".to_string())
            .await
            .unwrap();

        assert_eq!(killed.status, ExecStatus::Exited { exit_code: Some(0) });
        assert_eq!(failed.status, ExecStatus::Exited { exit_code: Some(0) });
        assert_eq!(failed.meta.ended_at_ms, ended_at);
    }

    #[tokio::test]
    async fn test_set_status_same_value_is_idempotent() {
        let registry = ExecRegistry::new();
        let snapshot = registry
            .register(
                meta("exec_same", ExecMode::Background, "sleep 1"),
                DEFAULT_MAX_BYTES,
            )
            .await;
        let process_id = snapshot.meta.process_id;

        let first = registry.mark_started(&process_id).await.unwrap();
        let second = registry.mark_started(&process_id).await.unwrap();
        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn test_service_name_lookup_scopes_by_owner_and_workspace() {
        let registry = ExecRegistry::new();
        let first = meta("exec_service_a", ExecMode::Service, "server")
            .with_chat_id("chat-a")
            .with_service_name("api")
            .with_workspace(PathBuf::from("/workspace-a"));
        let second = meta("exec_service_b", ExecMode::Service, "server")
            .with_chat_id("chat-b")
            .with_service_name("api")
            .with_workspace(PathBuf::from("/workspace-b"));
        registry.register(first, DEFAULT_MAX_BYTES).await;
        registry.register(second, DEFAULT_MAX_BYTES).await;

        let found = registry
            .find_service(
                ExecServiceLookup::new("api")
                    .with_chat_id("chat-b")
                    .with_workspace(PathBuf::from("/workspace-b")),
            )
            .await
            .expect("service found");
        assert_eq!(
            found.meta.process_id,
            ExecProcessId("exec_service_b".to_string())
        );

        let missing = registry
            .find_service(
                ExecServiceLookup::new("api")
                    .with_chat_id("chat-a")
                    .with_workspace(PathBuf::from("/workspace-b")),
            )
            .await;
        assert!(missing.is_none());
    }

    #[tokio::test]
    async fn test_remove_by_process_id() {
        let registry = ExecRegistry::new();
        let snapshot = registry
            .register(
                meta("exec_remove", ExecMode::Foreground, "true"),
                DEFAULT_MAX_BYTES,
            )
            .await;
        let process_id = snapshot.meta.process_id.clone();

        assert_eq!(registry.remove(&process_id).await, Some(snapshot));
        assert!(registry.get(&process_id).await.is_none());
    }

    #[tokio::test]
    async fn test_remove_by_owner() {
        let registry = ExecRegistry::new();
        registry
            .register(
                meta("exec_keep", ExecMode::Foreground, "true").with_chat_id("chat-keep"),
                DEFAULT_MAX_BYTES,
            )
            .await;
        registry
            .register(
                meta("exec_drop_one", ExecMode::Foreground, "true").with_chat_id("chat-drop"),
                DEFAULT_MAX_BYTES,
            )
            .await;
        registry
            .register(
                meta("exec_drop_two", ExecMode::Foreground, "true").with_chat_id("chat-drop"),
                DEFAULT_MAX_BYTES,
            )
            .await;

        let removed = registry
            .remove_by_owner(ExecProcessFilter {
                chat_id: Some("chat-drop".to_string()),
                ..ExecProcessFilter::default()
            })
            .await;
        assert_eq!(removed.len(), 2);
        let remaining = registry.list(ExecProcessFilter::default()).await;
        assert_eq!(remaining.len(), 1);
        assert_eq!(
            remaining[0].meta.process_id,
            ExecProcessId("exec_keep".to_string())
        );
    }

    #[tokio::test]
    async fn test_cleanup_shutdown_removes_active_state() {
        let registry = ExecRegistry::new();
        let starting = registry
            .register(
                meta("exec_cleanup_starting", ExecMode::Foreground, "echo hi"),
                DEFAULT_MAX_BYTES,
            )
            .await;
        let running = registry
            .register(
                meta("exec_cleanup_running", ExecMode::Service, "server"),
                DEFAULT_MAX_BYTES,
            )
            .await;
        registry
            .mark_started(&running.meta.process_id)
            .await
            .unwrap();
        let exited = registry
            .register(
                meta("exec_cleanup_exited", ExecMode::Background, "true"),
                DEFAULT_MAX_BYTES,
            )
            .await;
        registry
            .mark_exited(&exited.meta.process_id, Some(0))
            .await
            .unwrap();

        let summary = registry.cleanup_shutdown(Duration::from_millis(10)).await;

        assert_eq!(summary.removed_count, 2);
        assert_eq!(summary.child_stop_attempts, 0);
        assert_eq!(summary.child_failed_count, 0);
        assert_eq!(summary.child_timed_out_count, 0);
        assert!(registry.get(&starting.meta.process_id).await.is_none());
        assert!(registry.get(&running.meta.process_id).await.is_none());
        assert!(registry.get(&exited.meta.process_id).await.is_some());
    }

    #[tokio::test]
    async fn test_cleanup_shutdown_kills_registered_child() {
        let registry = ExecRegistry::new();
        let mut command = tokio::process::Command::new("sh");
        command.arg("-c").arg("sleep 30");
        #[cfg(unix)]
        unsafe {
            command.pre_exec(|| {
                libc::setpgid(0, 0);
                Ok(())
            });
        }
        let child = command.spawn().expect("spawn child");
        let snapshot = registry
            .register_with_child(
                meta("exec_cleanup_child", ExecMode::Background, "sleep 30"),
                DEFAULT_MAX_BYTES,
                child,
            )
            .await;
        registry
            .mark_started(&snapshot.meta.process_id)
            .await
            .unwrap();

        let summary = registry.cleanup_shutdown(Duration::from_secs(2)).await;

        assert_eq!(summary.removed_count, 1);
        assert_eq!(summary.child_stop_attempts, 1);
        assert_eq!(summary.child_stopped_count, 1);
        assert_eq!(summary.child_failed_count, 0);
        assert_eq!(summary.child_timed_out_count, 0);
        assert!(registry.get(&snapshot.meta.process_id).await.is_none());
    }

    #[tokio::test]
    async fn test_concurrent_append_read() {
        let registry = ExecRegistry::new();
        let snapshot = registry
            .register(
                meta("exec_concurrent", ExecMode::Background, "server"),
                4096,
            )
            .await;
        let process_id = snapshot.meta.process_id;

        let writer_registry = registry.clone();
        let writer_process_id = process_id.clone();
        let writer = tokio::spawn(async move {
            for i in 0..50 {
                writer_registry
                    .append_output(
                        &writer_process_id,
                        ExecOutputStream::Stdout,
                        format!("line {i}\n"),
                    )
                    .await
                    .unwrap();
            }
        });

        let reader_registry = registry.clone();
        let reader_process_id = process_id.clone();
        let reader = tokio::spawn(async move {
            let mut observed = 0;
            loop {
                let read = reader_registry.read(&reader_process_id, 0, None).await;
                observed = observed.max(read.chunks.len());
                if observed >= 50 {
                    break observed;
                }
                tokio::task::yield_now().await;
            }
        });

        writer.await.unwrap();
        let observed = reader.await.unwrap();
        assert_eq!(observed, 50);
        let final_read = registry.read(&process_id, 0, None).await;
        assert_eq!(final_read.chunks.len(), 50);
        assert_eq!(final_read.latest_seq, 50);
    }
}
