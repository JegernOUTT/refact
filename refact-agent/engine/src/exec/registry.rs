use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, oneshot, Mutex, Notify};

use crate::exec::transcript::ExecTranscript;
use crate::exec::types::{
    current_timestamp_ms, ExecOutputChunk, ExecOutputStream, ExecProcessFilter, ExecProcessId,
    ExecProcessMeta, ExecProcessSnapshot, ExecReadResult, ExecServiceLookup, ExecStatus,
};

const REMOVE_KILL_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) enum ExecProcessCommand {
    Kill {
        response: oneshot::Sender<Result<ExecProcessSnapshot, String>>,
    },
    Finish {
        status: ExecStatus,
        response: oneshot::Sender<Result<ExecProcessSnapshot, String>>,
    },
}

pub(crate) struct ExecProcessRuntime {
    pub control_tx: mpsc::Sender<ExecProcessCommand>,
    pub terminal: Arc<Notify>,
}

impl Clone for ExecProcessRuntime {
    fn clone(&self) -> Self {
        Self {
            control_tx: self.control_tx.clone(),
            terminal: self.terminal.clone(),
        }
    }
}

struct ExecProcessRecord {
    snapshot: ExecProcessSnapshot,
    transcript: ExecTranscript,
    child: Option<tokio::process::Child>,
    runtime: Option<ExecProcessRuntime>,
}

impl ExecProcessRecord {
    fn new(meta: ExecProcessMeta, transcript_limit_bytes: usize) -> Self {
        let process_id = meta.process_id.clone();
        Self {
            snapshot: ExecProcessSnapshot::new(meta),
            transcript: ExecTranscript::new(process_id, transcript_limit_bytes),
            child: None,
            runtime: None,
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

    fn with_runtime(
        meta: ExecProcessMeta,
        transcript_limit_bytes: usize,
        runtime: ExecProcessRuntime,
    ) -> Self {
        let mut record = Self::new(meta, transcript_limit_bytes);
        record.runtime = Some(runtime);
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
    runtime: Option<ExecProcessRuntime>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecShutdownCleanupSummary {
    pub removed_count: usize,
    pub runtime_stop_attempts: usize,
    pub runtime_stopped_count: usize,
    pub runtime_failed_count: usize,
    pub runtime_timed_out_count: usize,
    pub child_stop_attempts: usize,
    pub child_stopped_count: usize,
    pub child_failed_count: usize,
    pub child_timed_out_count: usize,
}

#[derive(Clone, Copy)]
enum ExecCleanupTargetKind {
    Runtime,
    Child,
    NoChild,
}

enum ExecCleanupOutcome {
    NoChild,
    Stopped,
    Failed {
        message: String,
        child: Option<tokio::process::Child>,
    },
    TimedOut {
        message: String,
        child: Option<tokio::process::Child>,
    },
}

struct ExecRemoveTarget {
    process_id: ExecProcessId,
    runtime: ExecProcessRuntime,
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
        match records.get(&process_id) {
            Some(existing) if !existing.snapshot.status.is_terminal() => {
                return existing.snapshot.clone();
            }
            Some(_) | None => {}
        }
        records.insert(process_id, record);
        snapshot
    }

    pub async fn register_new(
        &self,
        meta: ExecProcessMeta,
        transcript_limit_bytes: usize,
    ) -> Result<ExecProcessSnapshot, String> {
        let process_id = meta.process_id.clone();
        let record = ExecProcessRecord::new(meta, transcript_limit_bytes);
        let snapshot = record.snapshot.clone();
        let mut records = self.records.lock().await;
        match records
            .get(&process_id)
            .map(|existing| existing.snapshot.status.is_terminal())
        {
            Some(false) => return Err(format!("process already exists: {process_id}")),
            Some(true) | None => {}
        }
        records.insert(process_id, record);
        Ok(snapshot)
    }

    pub(crate) async fn register_new_with_runtime(
        &self,
        meta: ExecProcessMeta,
        transcript_limit_bytes: usize,
        runtime: ExecProcessRuntime,
    ) -> Result<ExecProcessSnapshot, String> {
        let process_id = meta.process_id.clone();
        let record = ExecProcessRecord::with_runtime(meta, transcript_limit_bytes, runtime);
        let snapshot = record.snapshot.clone();
        let mut records = self.records.lock().await;
        match records
            .get(&process_id)
            .map(|existing| existing.snapshot.status.is_terminal())
        {
            Some(false) => return Err(format!("process already exists: {process_id}")),
            Some(true) | None => {}
        }
        records.insert(process_id, record);
        Ok(snapshot)
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

    #[cfg(test)]
    pub(crate) async fn attach_runtime(
        &self,
        process_id: &ExecProcessId,
        runtime: ExecProcessRuntime,
    ) -> Result<ExecProcessSnapshot, String> {
        let mut records = self.records.lock().await;
        let record = records
            .get_mut(process_id)
            .ok_or_else(|| format!("process not found: {process_id}"))?;
        record.runtime = Some(runtime);
        Ok(record.snapshot.clone())
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
        let snapshot = record.snapshot.clone();
        let terminal = if snapshot.status.is_terminal() {
            record
                .runtime
                .as_ref()
                .map(|runtime| runtime.terminal.clone())
        } else {
            None
        };
        drop(records);
        if let Some(terminal) = terminal {
            terminal.notify_waiters();
        }
        Ok(snapshot)
    }

    pub(crate) async fn complete_status(
        &self,
        process_id: &ExecProcessId,
        status: ExecStatus,
    ) -> Result<ExecProcessSnapshot, String> {
        self.set_status(process_id, status).await
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

    pub async fn kill(&self, process_id: &ExecProcessId) -> Result<ExecProcessSnapshot, String> {
        self.finish_process(process_id, None).await
    }

    pub(crate) async fn finish_with_status(
        &self,
        process_id: &ExecProcessId,
        status: ExecStatus,
    ) -> Result<ExecProcessSnapshot, String> {
        self.finish_process(process_id, Some(status)).await
    }

    async fn finish_process(
        &self,
        process_id: &ExecProcessId,
        status: Option<ExecStatus>,
    ) -> Result<ExecProcessSnapshot, String> {
        let control_tx = {
            let records = self.records.lock().await;
            let record = records
                .get(process_id)
                .ok_or_else(|| format!("process not found: {process_id}"))?;
            if record.snapshot.status.is_terminal() {
                return Ok(record.snapshot.clone());
            }
            record
                .runtime
                .as_ref()
                .map(|runtime| runtime.control_tx.clone())
                .ok_or_else(|| format!("process is not running: {process_id}"))?
        };
        let (response, rx) = oneshot::channel();
        let terminal_status = status.clone().unwrap_or(ExecStatus::Killed);
        let command = match status {
            Some(status) => ExecProcessCommand::Finish { status, response },
            None => ExecProcessCommand::Kill { response },
        };
        if control_tx.send(command).await.is_err() {
            return self.set_status(process_id, terminal_status).await;
        }
        tokio::select! {
            response = rx => match response {
                Ok(result) => result,
                Err(_) => self.wait(process_id).await,
            },
            result = self.wait(process_id) => result,
        }
    }

    pub async fn wait(&self, process_id: &ExecProcessId) -> Result<ExecProcessSnapshot, String> {
        loop {
            let mut records = self.records.lock().await;
            let record = records
                .get_mut(process_id)
                .ok_or_else(|| format!("process not found: {process_id}"))?;
            if record.snapshot.status.is_terminal() {
                return Ok(record.snapshot.clone());
            }
            if record
                .runtime
                .as_ref()
                .map(|runtime| runtime.control_tx.is_closed())
                .unwrap_or(false)
            {
                record.set_status(ExecStatus::Failed {
                    message: "process runtime stopped before terminal status".to_string(),
                });
                let snapshot = record.snapshot.clone();
                let terminal = record
                    .runtime
                    .as_ref()
                    .map(|runtime| runtime.terminal.clone());
                drop(records);
                if let Some(terminal) = terminal {
                    terminal.notify_waiters();
                }
                return Ok(snapshot);
            }
            let terminal = record
                .runtime
                .as_ref()
                .map(|runtime| runtime.terminal.clone())
                .ok_or_else(|| format!("process is not running: {process_id}"))?;
            let notified = terminal.notified();
            drop(records);
            notified.await;
        }
    }

    pub async fn remove(
        &self,
        process_id: &ExecProcessId,
    ) -> Result<Option<ExecProcessSnapshot>, String> {
        let Some(target) = self.remove_target(process_id).await else {
            return Ok(self.remove_record(process_id).await);
        };
        self.stop_remove_target(target, REMOVE_KILL_TIMEOUT).await?;
        Ok(self.remove_record(process_id).await)
    }

    pub async fn remove_by_owner(
        &self,
        filter: ExecProcessFilter,
    ) -> Result<Vec<ExecProcessSnapshot>, String> {
        let process_ids = {
            let records = self.records.lock().await;
            records
                .iter()
                .filter(|(_, record)| record.snapshot.meta.owner.matches_filter(&filter))
                .filter(|(_, record)| {
                    filter
                        .status
                        .map(|status| record.snapshot.status.kind() == status)
                        .unwrap_or(true)
                })
                .map(|(process_id, _)| process_id.clone())
                .collect::<Vec<_>>()
        };
        let mut removed = Vec::with_capacity(process_ids.len());
        for process_id in process_ids {
            if let Some(snapshot) = self.remove(&process_id).await? {
                removed.push(snapshot);
            }
        }
        Ok(removed)
    }

    async fn remove_target(&self, process_id: &ExecProcessId) -> Option<ExecRemoveTarget> {
        let records = self.records.lock().await;
        let record = records.get(process_id)?;
        if record.snapshot.status.is_terminal() {
            return None;
        }
        record.runtime.clone().map(|runtime| ExecRemoveTarget {
            process_id: process_id.clone(),
            runtime,
        })
    }

    async fn stop_remove_target(
        &self,
        target: ExecRemoveTarget,
        timeout: Duration,
    ) -> Result<ExecProcessSnapshot, String> {
        match tokio::time::timeout(timeout, self.kill(&target.process_id)).await {
            Ok(Ok(snapshot)) => Ok(snapshot),
            Ok(Err(message)) => {
                let _ = self
                    .mark_failed(&target.process_id, message.clone())
                    .await?;
                target.runtime.terminal.notify_waiters();
                Err(message)
            }
            Err(_) => {
                let message = format!(
                    "timed out while removing process after {:.3}s",
                    timeout.as_secs_f64()
                );
                let _ = self
                    .mark_failed(&target.process_id, message.clone())
                    .await?;
                target.runtime.terminal.notify_waiters();
                Err(message)
            }
        }
    }

    async fn remove_terminal_record(
        &self,
        process_id: &ExecProcessId,
    ) -> Option<ExecProcessSnapshot> {
        let (snapshot, terminal) = {
            let mut records = self.records.lock().await;
            let record = records.get(process_id)?;
            if !record.snapshot.status.is_terminal() {
                return None;
            }
            let record = records.remove(process_id)?;
            (
                record.snapshot,
                record.runtime.map(|runtime| runtime.terminal),
            )
        };
        if let Some(terminal) = terminal {
            terminal.notify_waiters();
        }
        Some(snapshot)
    }

    async fn remove_record(&self, process_id: &ExecProcessId) -> Option<ExecProcessSnapshot> {
        let (snapshot, terminal) = {
            let mut records = self.records.lock().await;
            let record = records.remove(process_id)?;
            (
                record.snapshot,
                record.runtime.map(|runtime| runtime.terminal),
            )
        };
        if let Some(terminal) = terminal {
            terminal.notify_waiters();
        }
        Some(snapshot)
    }

    pub async fn cleanup_shutdown(&self, timeout: Duration) -> ExecShutdownCleanupSummary {
        let targets = self.collect_active_targets().await;
        let mut summary = ExecShutdownCleanupSummary::default();
        let outcomes = futures::future::join_all(
            targets
                .into_iter()
                .map(|target| cleanup_target(self.clone(), target, timeout)),
        )
        .await;
        for (process_id, short_description, kind, outcome) in outcomes {
            match outcome {
                ExecCleanupOutcome::NoChild => {
                    if self.remove_terminal_record(&process_id).await.is_some() {
                        summary.removed_count += 1;
                    }
                }
                ExecCleanupOutcome::Stopped => {
                    match kind {
                        ExecCleanupTargetKind::Runtime => {
                            summary.runtime_stop_attempts += 1;
                            summary.runtime_stopped_count += 1;
                        }
                        ExecCleanupTargetKind::Child => {
                            summary.child_stop_attempts += 1;
                            summary.child_stopped_count += 1;
                        }
                        ExecCleanupTargetKind::NoChild => {}
                    }
                    if self.remove_terminal_record(&process_id).await.is_some() {
                        summary.removed_count += 1;
                    }
                }
                ExecCleanupOutcome::Failed { message, child } => {
                    match kind {
                        ExecCleanupTargetKind::Runtime => {
                            summary.runtime_stop_attempts += 1;
                            summary.runtime_failed_count += 1;
                        }
                        ExecCleanupTargetKind::Child => {
                            summary.child_stop_attempts += 1;
                            summary.child_failed_count += 1;
                        }
                        ExecCleanupTargetKind::NoChild => {}
                    }
                    self.keep_cleanup_failure(&process_id, message.clone(), child)
                        .await;
                    tracing::warn!(
                        "exec cleanup: failed to stop {} ({}): {}",
                        process_id,
                        short_description,
                        message
                    );
                }
                ExecCleanupOutcome::TimedOut { message, child } => {
                    match kind {
                        ExecCleanupTargetKind::Runtime => {
                            summary.runtime_stop_attempts += 1;
                            summary.runtime_timed_out_count += 1;
                        }
                        ExecCleanupTargetKind::Child => {
                            summary.child_stop_attempts += 1;
                            summary.child_timed_out_count += 1;
                        }
                        ExecCleanupTargetKind::NoChild => {}
                    }
                    self.keep_cleanup_failure(&process_id, message, child).await;
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

    async fn keep_cleanup_failure(
        &self,
        process_id: &ExecProcessId,
        message: String,
        child: Option<tokio::process::Child>,
    ) {
        let terminal = {
            let mut records = self.records.lock().await;
            let Some(record) = records.get_mut(process_id) else {
                return;
            };
            if record.child.is_none() {
                record.child = child;
            }
            if record.snapshot.meta.ended_at_ms.is_none() {
                record.snapshot.meta.ended_at_ms = Some(current_timestamp_ms());
            }
            record.snapshot.status = ExecStatus::Failed { message };
            record
                .runtime
                .as_ref()
                .map(|runtime| runtime.terminal.clone())
        };
        if let Some(terminal) = terminal {
            terminal.notify_waiters();
        }
    }

    async fn collect_active_targets(&self) -> Vec<ExecCleanupTarget> {
        let mut records = self.records.lock().await;
        records
            .values_mut()
            .filter(|record| !record.snapshot.status.is_terminal() || record.child.is_some())
            .map(|record| {
                let runtime = if record.snapshot.status.is_terminal() {
                    None
                } else {
                    record.runtime.clone()
                };
                if runtime.is_none() && !record.snapshot.status.is_terminal() {
                    record.set_status(ExecStatus::Killed);
                }
                ExecCleanupTarget {
                    snapshot: record.snapshot.clone(),
                    child: record.child.take(),
                    runtime,
                }
            })
            .collect()
    }
}

async fn cleanup_target(
    registry: ExecRegistry,
    target: ExecCleanupTarget,
    timeout: Duration,
) -> (
    ExecProcessId,
    String,
    ExecCleanupTargetKind,
    ExecCleanupOutcome,
) {
    let process_id = target.snapshot.meta.process_id.clone();
    let short_description = target.snapshot.meta.short_description.clone();
    if target.runtime.is_some() && !target.snapshot.status.is_terminal() {
        let outcome = match tokio::time::timeout(timeout, registry.kill(&process_id)).await {
            Ok(Ok(_)) => ExecCleanupOutcome::Stopped,
            Ok(Err(message)) => ExecCleanupOutcome::Failed {
                message,
                child: None,
            },
            Err(_) => ExecCleanupOutcome::TimedOut {
                message: format!(
                    "timed out while stopping runtime process after {:.3}s",
                    timeout.as_secs_f64()
                ),
                child: None,
            },
        };
        return (
            process_id,
            short_description,
            ExecCleanupTargetKind::Runtime,
            outcome,
        );
    }
    let Some(mut child) = target.child else {
        return (
            process_id,
            short_description,
            ExecCleanupTargetKind::NoChild,
            ExecCleanupOutcome::NoChild,
        );
    };

    if timeout.is_zero() {
        #[cfg(unix)]
        {
            if let Some(pid) = child.id() {
                let _ = unsafe { libc::kill(pid as i32, libc::SIGKILL) };
            }
        }
        #[cfg(windows)]
        {
            let _ = child.start_kill();
        }
        return (
            process_id,
            short_description,
            ExecCleanupTargetKind::Child,
            ExecCleanupOutcome::TimedOut {
                message: "timed out while stopping child process after 0.000s".to_string(),
                child: Some(child),
            },
        );
    }
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
    let (outcome, child) = match wait_result {
        Ok(Ok(_)) => (ExecCleanupOutcome::Stopped, None),
        Ok(Err(err)) => (
            ExecCleanupOutcome::Failed {
                message: err.to_string(),
                child: None,
            },
            None,
        ),
        Err(_) => (
            ExecCleanupOutcome::TimedOut {
                message: format!(
                    "timed out while stopping child process after {:.3}s",
                    timeout.as_secs_f64()
                ),
                child: None,
            },
            Some(child),
        ),
    };
    let outcome = match (outcome, kill_error) {
        (ExecCleanupOutcome::Stopped, _) => ExecCleanupOutcome::Stopped,
        (_, Some(message)) => ExecCleanupOutcome::Failed { message, child },
        (ExecCleanupOutcome::Failed { message, .. }, None) => {
            ExecCleanupOutcome::Failed { message, child }
        }
        (ExecCleanupOutcome::TimedOut { message, .. }, None) => {
            ExecCleanupOutcome::TimedOut { message, child }
        }
        (ExecCleanupOutcome::NoChild, None) => ExecCleanupOutcome::NoChild,
    };
    (
        process_id,
        short_description,
        ExecCleanupTargetKind::Child,
        outcome,
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::exec::transcript::DEFAULT_MAX_BYTES;
    use crate::exec::types::{ExecMode, ExecOwnerMeta, ExecStatusKind};

    fn meta(process_id: &str, mode: ExecMode, command: &str) -> ExecProcessMeta {
        ExecProcessMeta::new(mode, command.to_string())
            .with_process_id(ExecProcessId(process_id.to_string()))
    }

    fn long_running_command() -> String {
        if cfg!(target_os = "windows") {
            "[Console]::Out.Write('ready'); Start-Sleep -Seconds 30".to_string()
        } else {
            "printf ready; sleep 30".to_string()
        }
    }

    #[cfg(unix)]
    fn process_exists(process_id: u32) -> bool {
        unsafe { libc::kill(process_id as i32, 0) == 0 }
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
    async fn test_register_does_not_overwrite_active_record() {
        let registry = ExecRegistry::new();
        let first = registry
            .register(
                meta("exec_duplicate", ExecMode::Background, "sleep 1").with_chat_id("chat-a"),
                DEFAULT_MAX_BYTES,
            )
            .await;
        let process_id = first.meta.process_id.clone();
        registry.mark_started(&process_id).await.unwrap();

        let second = registry
            .register(
                meta("exec_duplicate", ExecMode::Background, "echo replacement")
                    .with_chat_id("chat-b"),
                DEFAULT_MAX_BYTES,
            )
            .await;
        let stored = registry.get(&process_id).await.unwrap();

        assert_eq!(second, stored);
        assert_eq!(stored.status, ExecStatus::Running);
        assert_eq!(stored.meta.command, "sleep 1");
        assert_eq!(stored.meta.owner.chat_id.as_deref(), Some("chat-a"));
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
    async fn test_wait_marks_closed_runtime_terminal() {
        let registry = ExecRegistry::new();
        let snapshot = registry
            .register(
                meta("exec_dead_runtime", ExecMode::Background, "sleep 1"),
                DEFAULT_MAX_BYTES,
            )
            .await;
        let (control_tx, control_rx) = mpsc::channel(1);
        drop(control_rx);
        registry
            .attach_runtime(
                &snapshot.meta.process_id,
                ExecProcessRuntime {
                    control_tx,
                    terminal: Arc::new(Notify::new()),
                },
            )
            .await
            .unwrap();
        registry
            .mark_started(&snapshot.meta.process_id)
            .await
            .unwrap();

        let waited = tokio::time::timeout(
            Duration::from_millis(100),
            registry.wait(&snapshot.meta.process_id),
        )
        .await
        .expect("wait should not hang")
        .unwrap();

        assert!(waited.status.is_terminal());
        assert_eq!(
            waited.status,
            ExecStatus::Failed {
                message: "process runtime stopped before terminal status".to_string()
            }
        );
    }

    #[tokio::test]
    async fn test_finish_process_marks_closed_runtime_killed() {
        let registry = ExecRegistry::new();
        let snapshot = registry
            .register(
                meta("exec_dead_runtime_kill", ExecMode::Background, "sleep 1"),
                DEFAULT_MAX_BYTES,
            )
            .await;
        let (control_tx, control_rx) = mpsc::channel(1);
        drop(control_rx);
        registry
            .attach_runtime(
                &snapshot.meta.process_id,
                ExecProcessRuntime {
                    control_tx,
                    terminal: Arc::new(Notify::new()),
                },
            )
            .await
            .unwrap();
        registry
            .mark_started(&snapshot.meta.process_id)
            .await
            .unwrap();

        let killed = tokio::time::timeout(
            Duration::from_millis(100),
            registry.kill(&snapshot.meta.process_id),
        )
        .await
        .expect("kill should not hang")
        .unwrap();

        assert_eq!(killed.status, ExecStatus::Killed);
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

        assert_eq!(registry.remove(&process_id).await.unwrap(), Some(snapshot));
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
            .await
            .unwrap();
        assert_eq!(removed.len(), 2);
        let remaining = registry.list(ExecProcessFilter::default()).await;
        assert_eq!(remaining.len(), 1);
        assert_eq!(
            remaining[0].meta.process_id,
            ExecProcessId("exec_keep".to_string())
        );
    }

    #[tokio::test]
    async fn remove_by_owner_kills_active_processes() {
        let registry = ExecRegistry::new();
        let first = registry
            .spawn(
                crate::exec::ExecSpawnRequest::background(long_running_command()).with_owner(
                    ExecOwnerMeta {
                        chat_id: Some("chat-drop".to_string()),
                        ..ExecOwnerMeta::default()
                    },
                ),
            )
            .await
            .unwrap();
        let second = registry
            .spawn(
                crate::exec::ExecSpawnRequest::background(long_running_command()).with_owner(
                    ExecOwnerMeta {
                        chat_id: Some("chat-drop".to_string()),
                        ..ExecOwnerMeta::default()
                    },
                ),
            )
            .await
            .unwrap();

        let removed = registry
            .remove_by_owner(ExecProcessFilter {
                chat_id: Some("chat-drop".to_string()),
                ..ExecProcessFilter::default()
            })
            .await
            .unwrap();

        assert_eq!(removed.len(), 2);
        assert!(removed
            .iter()
            .all(|snapshot| snapshot.status == ExecStatus::Killed));
        assert!(registry
            .get(&first.snapshot.meta.process_id)
            .await
            .is_none());
        assert!(registry
            .get(&second.snapshot.meta.process_id)
            .await
            .is_none());
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
        let mut command = if cfg!(target_os = "windows") {
            let mut command = tokio::process::Command::new("powershell.exe");
            command
                .arg("-NoProfile")
                .arg("-Command")
                .arg("Start-Sleep -Seconds 30");
            command
        } else {
            let mut command = tokio::process::Command::new("sh");
            command.arg("-c").arg("sleep 30");
            command
        };
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
    async fn cleanup_timeout_keeps_record() {
        let registry = ExecRegistry::new();
        let mut command = if cfg!(target_os = "windows") {
            let mut command = tokio::process::Command::new("powershell.exe");
            command.arg("-Command").arg("Start-Sleep -Seconds 30");
            command
        } else {
            let mut command = tokio::process::Command::new("sleep");
            command.arg("30");
            command
        };
        let child = command.spawn().expect("spawn child");
        let child_id = child.id().expect("child id");
        let snapshot = registry
            .register_with_child(
                meta(
                    &format!("exec_cleanup_timeout_pid_{child_id}"),
                    ExecMode::Background,
                    "sleep 30",
                ),
                DEFAULT_MAX_BYTES,
                child,
            )
            .await;
        registry
            .mark_started(&snapshot.meta.process_id)
            .await
            .unwrap();

        let summary = registry.cleanup_shutdown(Duration::ZERO).await;
        let retained = registry.get(&snapshot.meta.process_id).await.unwrap();

        assert_eq!(summary.removed_count, 0);
        assert_eq!(summary.child_stop_attempts, 1);
        assert_eq!(summary.child_stopped_count, 0);
        assert_eq!(summary.child_timed_out_count, 1);
        assert!(matches!(retained.status, ExecStatus::Failed { .. }));
        #[cfg(not(unix))]
        let _ = child_id;
        #[cfg(unix)]
        assert!(process_exists(child_id));
        let _ = registry.remove(&snapshot.meta.process_id).await;
    }

    #[tokio::test]
    async fn test_cleanup_shutdown_kills_runtime_background_process() {
        let registry = ExecRegistry::new();
        let result = registry
            .spawn(crate::exec::ExecSpawnRequest::background(
                long_running_command(),
            ))
            .await
            .unwrap();
        let process_id = result.snapshot.meta.process_id.clone();
        assert_eq!(result.snapshot.status, ExecStatus::Running);

        let summary = registry.cleanup_shutdown(Duration::from_secs(2)).await;

        assert_eq!(summary.removed_count, 1);
        assert_eq!(summary.runtime_stop_attempts, 1);
        assert_eq!(summary.runtime_stopped_count, 1);
        assert_eq!(summary.runtime_failed_count, 0);
        assert_eq!(summary.runtime_timed_out_count, 0);
        assert!(registry.get(&process_id).await.is_none());
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
