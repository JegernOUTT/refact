//! Daemon-owned observations, never goal or permission authority.
//!
//! A worker projection is authenticated against the current supervisor instance.
//! Retained mandate identifiers are references only: they cannot restore LIVE
//! grants, answer questions, approve tools, resume goals, or authorize a wake.
use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::daemon::projects::ProjectEntry;
use crate::daemon::state::write_daemon_state_json_atomic;

const MAX_PROJECTS: usize = 256;
const MAX_BYTES: u64 = 1_048_576;
pub(crate) const RECONCILE_BATCH: usize = 16;
const FRESH_MS: u64 = 90_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedIdentity {
    pub pid: u32,
    pub generation: u64,
    /// One-way digest, not a credential accepted by worker_identity_matches.
    pub instance_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupervisionRef {
    pub mandate_id: String,
    pub owner_epoch: u64,
    pub chat_id: String,
    pub worker_agent_id: Option<String>,
}

/// Complete bounded snapshot, not an event or command delivery request.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerProjection {
    pub project_id: String,
    pub canonical_root: PathBuf,
    pub pid: u32,
    pub instance_token: String,
    pub cursor: u64,
    pub supervision: Vec<SupervisionRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GapReason {
    DaemonRestart,
    WorkerUnavailable,
    WorkerReplaced,
    SummaryStale,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfflineGap {
    pub since_ms: u64,
    pub observed_at_ms: u64,
    pub recovered_at_ms: Option<u64>,
    pub reason: GapReason,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectObservation {
    pub project_id: String,
    pub canonical_root: PathBuf,
    pub worker: ObservedIdentity,
    pub last_seen_ms: u64,
    pub cursor: u64,
    pub supervision: Vec<SupervisionRef>,
    pub offline_gap: Option<OfflineGap>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Disk {
    projects: BTreeMap<String, ProjectObservation>,
}

pub(crate) struct ReconciliationStore {
    path: PathBuf,
    disk: Disk,
    after: Option<String>,
}

impl ReconciliationStore {
    pub(crate) async fn load(path: PathBuf, now: u64) -> Result<Self, String> {
        use tokio::io::AsyncReadExt;
        let disk = match tokio::fs::File::open(&path).await {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(MAX_BYTES + 1)
                    .read_to_end(&mut bytes)
                    .await
                    .map_err(|e| e.to_string())?;
                if bytes.len() as u64 > MAX_BYTES {
                    return Err("reconciliation file exceeds bound".into());
                }
                let disk: Disk = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                if disk.projects.len() > MAX_PROJECTS {
                    return Err("reconciliation project capacity exceeded".into());
                }
                disk
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Disk::default(),
            Err(e) => return Err(e.to_string()),
        };
        let mut store = Self {
            path,
            disk,
            after: None,
        };
        for row in store.disk.projects.values_mut() {
            open_gap(row, GapReason::DaemonRestart, now);
        }
        // Startup observes only. In particular it never adopts retained PIDs.
        if !store.disk.projects.is_empty() {
            store.save().await?;
        }
        Ok(store)
    }

    async fn save(&self) -> Result<(), String> {
        if serde_json::to_vec_pretty(&self.disk)
            .map_err(|e| e.to_string())?
            .len() as u64
            > MAX_BYTES
        {
            return Err("reconciliation file exceeds bound".into());
        }
        write_daemon_state_json_atomic(&self.path, &self.disk, "Buddy observations").await
    }

    pub(crate) fn get(&self, id: &str) -> Option<ProjectObservation> {
        self.disk.projects.get(id).cloned()
    }

    pub(crate) fn batch(&mut self) -> Vec<String> {
        let ids: Vec<_> = self
            .disk
            .projects
            .keys()
            .filter(|id| self.after.as_ref().is_none_or(|after| *id > after))
            .take(RECONCILE_BATCH)
            .cloned()
            .collect();
        self.after = if ids.len() == RECONCILE_BATCH {
            ids.last().cloned()
        } else {
            None
        };
        ids
    }

    pub(crate) async fn accept(
        &mut self,
        entry: &ProjectEntry,
        identity: ObservedIdentity,
        report: WorkerProjection,
        now: u64,
    ) -> Result<bool, String> {
        if entry.id != report.project_id
            || entry.root != report.canonical_root
            || report.supervision.len() > 64
            || report.cursor == 0
            || report.supervision.iter().any(|r| {
                !valid_id(&r.mandate_id)
                    || !valid_id(&r.chat_id)
                    || r.owner_epoch == 0
                    || r.worker_agent_id.as_ref().is_some_and(|id| !valid_id(id))
            })
            || report.supervision.iter().enumerate().any(|(i, r)| {
                report.supervision[..i]
                    .iter()
                    .any(|other| other.mandate_id == r.mandate_id)
            })
        {
            return Err("invalid bounded project projection".into());
        }
        let old = self.get(&entry.id);
        if old.is_none() && self.disk.projects.len() >= MAX_PROJECTS {
            return Err("reconciliation project capacity exceeded".into());
        }
        if let Some(old) = old.as_ref() {
            if old.canonical_root != entry.root {
                return Err("retained root mismatch".into());
            }
            if old.worker == identity && report.cursor <= old.cursor {
                if report.cursor == old.cursor && report.supervision == old.supervision {
                    return Ok(false);
                }
                return Err("stale or conflicting projection cursor".into());
            }
        }
        let mut row = ProjectObservation {
            project_id: entry.id.clone(),
            canonical_root: entry.root.clone(),
            worker: identity,
            last_seen_ms: now,
            cursor: report.cursor,
            supervision: report.supervision,
            offline_gap: old.as_ref().and_then(|r| r.offline_gap.clone()),
        };
        if old.as_ref().is_some_and(|old| old.worker != row.worker) {
            row.offline_gap = Some(OfflineGap {
                since_ms: old.as_ref().unwrap().last_seen_ms,
                observed_at_ms: now,
                recovered_at_ms: None,
                reason: GapReason::WorkerReplaced,
            });
        }
        if let Some(gap) = row.offline_gap.as_mut() {
            gap.recovered_at_ms = Some(now);
        }
        self.disk.projects.insert(entry.id.clone(), row);
        if let Err(e) = self.save().await {
            match old {
                Some(old) => {
                    self.disk.projects.insert(entry.id.clone(), old);
                }
                None => {
                    self.disk.projects.remove(&entry.id);
                }
            }
            return Err(e);
        }
        Ok(true)
    }

    pub(crate) async fn reconcile(
        &mut self,
        id: &str,
        entry: Option<&ProjectEntry>,
        identity: Option<&ObservedIdentity>,
        now: u64,
    ) -> Result<(), String> {
        let Some(old) = self.get(id) else {
            return Ok(());
        };
        if entry.is_none_or(|entry| entry.root != old.canonical_root) {
            self.disk.projects.remove(id);
        } else {
            let reason = match identity {
                None => Some(GapReason::WorkerUnavailable),
                Some(identity) if identity != &old.worker => Some(GapReason::WorkerReplaced),
                _ if now.saturating_sub(old.last_seen_ms) >= FRESH_MS => {
                    Some(GapReason::SummaryStale)
                }
                _ => None,
            };
            if let Some(reason) = reason {
                open_gap(self.disk.projects.get_mut(id).unwrap(), reason, now);
            }
        }
        if self.get(id).as_ref() != Some(&old) {
            if let Err(error) = self.save().await {
                self.disk.projects.insert(id.into(), old);
                return Err(error);
            }
        }
        Ok(())
    }
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && !id.chars().any(char::is_control)
}

fn open_gap(row: &mut ProjectObservation, reason: GapReason, now: u64) {
    if row
        .offline_gap
        .as_ref()
        .is_some_and(|gap| gap.recovered_at_ms.is_none())
    {
        return;
    }
    row.offline_gap = Some(OfflineGap {
        since_ms: row.last_seen_ms,
        observed_at_ms: now,
        recovered_at_ms: None,
        reason,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(root: PathBuf, id: &str) -> ProjectEntry {
        ProjectEntry {
            id: id.into(),
            root,
            slug: id.into(),
            pinned: false,
            automation_suspended: false,
            last_active_ms: 0,
            settings: Default::default(),
        }
    }

    fn identity(generation: u64) -> ObservedIdentity {
        ObservedIdentity {
            pid: 42,
            generation,
            instance_digest: format!("digest-{generation}"),
        }
    }

    fn report(entry: &ProjectEntry, cursor: u64) -> WorkerProjection {
        WorkerProjection {
            project_id: entry.id.clone(),
            canonical_root: entry.root.clone(),
            pid: 42,
            instance_token: "never-persist-this-token".into(),
            cursor,
            supervision: vec![SupervisionRef {
                mandate_id: "m".into(),
                owner_epoch: 1,
                chat_id: "c".into(),
                worker_agent_id: Some("a".into()),
            }],
        }
    }

    #[tokio::test]
    async fn two_roots_restart_preserves_cursors_not_authority() {
        let scratch = tempfile::tempdir().unwrap();
        let path = scratch.path().join("observations.json");
        let a = entry(scratch.path().join("a"), "a");
        let b = entry(scratch.path().join("b"), "b");
        let mut store = ReconciliationStore::load(path.clone(), 1).await.unwrap();
        store
            .accept(&a, identity(1), report(&a, 7), 10)
            .await
            .unwrap();
        store
            .accept(&b, identity(2), report(&b, 3), 11)
            .await
            .unwrap();
        let bytes = tokio::fs::read(&path).await.unwrap();
        assert!(!String::from_utf8(bytes)
            .unwrap()
            .contains("never-persist-this-token"));
        let mut restored = ReconciliationStore::load(path, 20).await.unwrap();
        assert_eq!(restored.get("a").unwrap().cursor, 7);
        assert_eq!(restored.get("b").unwrap().cursor, 3);
        assert_eq!(
            restored.get("a").unwrap().offline_gap.unwrap().reason,
            GapReason::DaemonRestart
        );
        // Repeated offline checks preserve the original gap instead of emitting storms.
        let old = restored.get("a").unwrap();
        restored.reconcile("a", Some(&a), None, 30).await.unwrap();
        assert_eq!(restored.get("a").unwrap(), old);
        restored
            .accept(&a, identity(3), report(&a, 1), 40)
            .await
            .unwrap();
        assert_eq!(
            restored
                .get("a")
                .unwrap()
                .offline_gap
                .unwrap()
                .recovered_at_ms,
            Some(40)
        );
        assert_eq!(restored.get("b").unwrap().cursor, 3);
        assert!(restored
            .get("b")
            .unwrap()
            .offline_gap
            .unwrap()
            .recovered_at_ms
            .is_none());
    }

    #[tokio::test]
    async fn duplicate_cursor_does_not_refresh_last_seen_or_accept_conflict() {
        let scratch = tempfile::tempdir().unwrap();
        let a = entry(scratch.path().join("a"), "a");
        let mut store = ReconciliationStore::load(scratch.path().join("observations.json"), 1)
            .await
            .unwrap();
        assert!(store
            .accept(&a, identity(1), report(&a, 5), 10)
            .await
            .unwrap());
        assert!(!store
            .accept(&a, identity(1), report(&a, 5), 20)
            .await
            .unwrap());
        assert_eq!(store.get("a").unwrap().last_seen_ms, 10);
        let mut conflicting = report(&a, 5);
        conflicting.supervision.clear();
        assert!(store
            .accept(&a, identity(1), conflicting, 21)
            .await
            .is_err());
        assert!(store
            .accept(&a, identity(1), report(&a, 4), 22)
            .await
            .is_err());
        store
            .reconcile("a", Some(&a), Some(&identity(1)), FRESH_MS + 10)
            .await
            .unwrap();
        assert_eq!(
            store.get("a").unwrap().offline_gap.unwrap().reason,
            GapReason::SummaryStale
        );
    }

    #[tokio::test]
    async fn foreign_root_and_command_or_consent_payloads_are_rejected() {
        let scratch = tempfile::tempdir().unwrap();
        let a = entry(scratch.path().join("a"), "a");
        let b = entry(scratch.path().join("b"), "b");
        let mut store = ReconciliationStore::load(scratch.path().join("observations.json"), 1)
            .await
            .unwrap();
        assert!(store
            .accept(&a, identity(1), report(&b, 1), 2)
            .await
            .is_err());
        for field in ["goal", "tool_consent", "approve", "grant", "resume", "wake"] {
            let mut value = serde_json::to_value(report(&a, 1)).unwrap();
            value[field] = serde_json::json!({"allowed": true});
            assert!(
                serde_json::from_value::<WorkerProjection>(value).is_err(),
                "{field}"
            );
        }
        assert!(store.get("a").is_none());
    }

    #[tokio::test]
    async fn forget_and_failed_persistence_do_not_change_other_roots() {
        let scratch = tempfile::tempdir().unwrap();
        let a = entry(scratch.path().join("a"), "a");
        let mut store = ReconciliationStore::load(scratch.path().join("observations.json"), 1)
            .await
            .unwrap();
        store
            .accept(&a, identity(1), report(&a, 1), 2)
            .await
            .unwrap();
        let before = store.get("a").unwrap();
        let blocked = scratch.path().join("blocked");
        tokio::fs::write(&blocked, b"file").await.unwrap();
        store.path = blocked.join("observations.json");
        assert!(store
            .accept(&a, identity(1), report(&a, 2), 3)
            .await
            .is_err());
        assert_eq!(store.get("a").unwrap(), before);
        assert!(store.reconcile("a", None, None, 4).await.is_err());
        assert_eq!(store.get("a").unwrap(), before);
        store.path = scratch.path().join("observations.json");
        store.reconcile("a", None, None, 5).await.unwrap();
        assert!(store.get("a").is_none());
    }

    #[tokio::test]
    async fn startup_and_tick_work_are_bounded() {
        let scratch = tempfile::tempdir().unwrap();
        let path = scratch.path().join("observations.json");
        let mut store = ReconciliationStore::load(path.clone(), 1).await.unwrap();
        for i in 0..(RECONCILE_BATCH + 1) {
            let a = entry(scratch.path().join(i.to_string()), &format!("{i:03}"));
            store
                .accept(&a, identity(1), report(&a, 1), 2)
                .await
                .unwrap();
        }
        let first = store.batch();
        let second = store.batch();
        assert_eq!(first.len(), RECONCILE_BATCH);
        assert_eq!(second.len(), 1);
        assert!(!first.contains(&second[0]));
        assert_eq!(store.batch(), first);
        tokio::fs::write(&path, vec![b' '; MAX_BYTES as usize + 1])
            .await
            .unwrap();
        assert!(ReconciliationStore::load(path, 3).await.is_err());
    }
}
