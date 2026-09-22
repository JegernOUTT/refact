//! User-authored worker-start authority only. Never a goal/mandate/consent database.
use std::{collections::BTreeMap, path::PathBuf};
use serde::{Deserialize, Serialize};
use crate::daemon::{projects::ProjectEntry, state::write_daemon_state_json_atomic};

const MAX_GRANTS: usize = 256;
const MAX_BYTES: u64 = 1_048_576;
const MAX_LIFETIME_MS: u64 = 7 * 24 * 60 * 60 * 1000;
pub(crate) const MIN_INTERVAL_MS: u64 = 60_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WakeTriggerKind {
    ObservationTick,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WakeGrantRequest {
    pub canonical_root: PathBuf,
    pub trigger_kind: WakeTriggerKind,
    pub trigger_id: String,
    pub expires_at_ms: u64,
    pub max_starts: u32,
    pub interval_ms: u64,
    /// Explicit narrow acknowledgment; not generic execution consent.
    pub wake_only: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WakeGrant {
    pub grant_id: String,
    pub project_id: String,
    /// Comes only from authenticated configured Basic credentials, never request text.
    pub authored_by: String,
    pub request: WakeGrantRequest,
    pub issued_at_ms: u64,
    pub remaining_starts: u32,
    pub revoked: bool,
    pub last_slot: u64,
    pub last_attempt_ms: Option<u64>,
    pub last_outcome: Option<String>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Disk {
    grants: BTreeMap<String, WakeGrant>,
}

pub(crate) struct WakeStore {
    path: PathBuf,
    disk: Disk,
    started_at_ms: u64,
    after: Option<String>,
    failed: bool,
}

impl WakeStore {
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
                    return Err("wake store exceeds bound".into());
                }
                let disk: Disk = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                if disk.grants.len() > MAX_GRANTS
                    || disk.grants.iter().any(|(id, g)| {
                        id != &g.project_id
                            || !valid_request(&g.request, g.issued_at_ms)
                            || g.remaining_starts > g.request.max_starts
                    })
                {
                    return Err("invalid wake store".into());
                }
                disk
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Disk::default(),
            Err(e) => return Err(e.to_string()),
        };
        Ok(Self {
            path,
            disk,
            started_at_ms: now,
            after: None,
            failed: false,
        })
    }

    async fn save(&mut self) -> Result<(), String> {
        // Fail closed after any uncertain persistence result, without refunding budgets.
        if self.failed {
            return Err("wake store requires daemon restart after persistence failure".into());
        }
        if serde_json::to_vec(&self.disk)
            .map_err(|e| e.to_string())?
            .len() as u64
            > MAX_BYTES / 2
        {
            self.failed = true;
            return Err("wake store exceeds persistence bound".into());
        }
        let result =
            write_daemon_state_json_atomic(&self.path, &self.disk, "wake-only grants").await;
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    pub(crate) fn all(&self) -> Vec<WakeGrant> {
        self.disk.grants.values().cloned().collect()
    }

    pub(crate) fn get(&self, id: &str) -> Option<WakeGrant> {
        self.disk.grants.get(id).cloned()
    }

    pub(crate) async fn issue(
        &mut self,
        entry: &ProjectEntry,
        request: WakeGrantRequest,
        authored_by: String,
        now: u64,
    ) -> Result<WakeGrant, String> {
        if !valid_request(&request, now)
            || request.canonical_root != entry.root
            || tokio::fs::canonicalize(&entry.root)
                .await
                .map_err(|e| e.to_string())?
                != entry.root
        {
            return Err("invalid finite canonical-project wake-only grant".into());
        }
        // No replace/retry budget replenishment: revoke first, then explicitly issue anew.
        if self.get(&entry.id).is_some_and(|g| !g.revoked) {
            return Err("revoke existing grant before issuing another".into());
        }
        if !self.disk.grants.contains_key(&entry.id) && self.disk.grants.len() >= MAX_GRANTS {
            return Err("wake grant capacity exceeded".into());
        }
        let grant = WakeGrant {
            grant_id: uuid::Uuid::new_v4().to_string(),
            project_id: entry.id.clone(),
            authored_by,
            remaining_starts: request.max_starts,
            request,
            issued_at_ms: now,
            revoked: false,
            last_slot: 0,
            last_attempt_ms: None,
            last_outcome: None,
        };
        self.disk.grants.insert(entry.id.clone(), grant.clone());
        self.save().await?;
        Ok(grant)
    }

    pub(crate) async fn revoke(&mut self, id: &str) -> Result<(), String> {
        if let Some(g) = self.disk.grants.get_mut(id) {
            g.revoked = true;
        }
        self.save().await
    }

    pub(crate) fn batch(&mut self, now: u64) -> Vec<WakeGrant> {
        let rows: Vec<_> = self
            .disk
            .grants
            .iter()
            .filter(|(id, _)| self.after.as_ref().is_none_or(|after| *id > after))
            .take(16)
            .map(|(_, g)| g.clone())
            .collect();
        self.after = if rows.len() == 16 {
            rows.last().map(|g| g.project_id.clone())
        } else {
            None
        };
        rows.into_iter().filter(|g| self.due(g, now)).collect()
    }

    fn due(&self, g: &WakeGrant, now: u64) -> bool {
        if self.failed
            || g.revoked
            || g.remaining_starts == 0
            || now >= g.request.expires_at_ms
            || now < g.issued_at_ms
        {
            return false;
        }
        let slot = (now - g.issued_at_ms) / g.request.interval_ms;
        let due_at = g
            .issued_at_ms
            .saturating_add(slot.saturating_mul(g.request.interval_ms));
        slot > g.last_slot
            && due_at > self.started_at_ms
            && now.saturating_sub(due_at) < 30_000
            && g.last_attempt_ms
                .is_none_or(|last| now >= last.saturating_add(g.request.interval_ms))
    }

    pub(crate) async fn consume(
        &mut self,
        entry: &ProjectEntry,
        expected: &WakeGrant,
        now: u64,
    ) -> Result<bool, String> {
        let Some(g) = self.get(&entry.id) else {
            return Ok(false);
        };
        if g.grant_id != expected.grant_id
            || g.request != expected.request
            || g.request.canonical_root != entry.root
            || !self.due(&g, now)
        {
            return Ok(false);
        }
        let row = self.disk.grants.get_mut(&entry.id).unwrap();
        row.last_slot = (now - row.issued_at_ms) / row.request.interval_ms;
        row.last_attempt_ms = Some(now);
        row.remaining_starts -= 1;
        row.last_outcome = Some("consumed_start_unconfirmed".into());
        self.save().await?;
        Ok(true)
    }

    pub(crate) async fn outcome(
        &mut self,
        id: &str,
        grant_id: &str,
        outcome: String,
    ) -> Result<(), String> {
        if let Some(g) = self
            .disk
            .grants
            .get_mut(id)
            .filter(|g| g.grant_id == grant_id)
        {
            g.last_outcome = Some(outcome);
        }
        self.save().await
    }
}

fn valid_request(r: &WakeGrantRequest, now: u64) -> bool {
    r.wake_only
        && r.canonical_root.is_absolute()
        && !r.trigger_id.is_empty()
        && r.trigger_id.len() <= 128
        && r.trigger_id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
        && r.expires_at_ms > now
        && r.expires_at_ms - now <= MAX_LIFETIME_MS
        && (1..=16).contains(&r.max_starts)
        && (MIN_INTERVAL_MS..=MAX_LIFETIME_MS).contains(&r.interval_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(root: PathBuf) -> WakeGrantRequest {
        WakeGrantRequest {
            canonical_root: root,
            trigger_kind: WakeTriggerKind::ObservationTick,
            trigger_id: "user-observation-timer".into(),
            expires_at_ms: 1_000_000,
            max_starts: 2,
            interval_ms: 60_000,
            wake_only: true,
        }
    }
    #[tokio::test]
    async fn finite_canonical_grant_dedup_restart_expiry_and_no_catchup() {
        let dir = tempfile::tempdir().unwrap();
        let entry = ProjectEntry {
            id: "p".into(),
            slug: "p".into(),
            root: dir.path().canonicalize().unwrap(),
            pinned: false,
            last_active_ms: 0,
            settings: Default::default(),
        };
        let path = dir.path().join("wake.json");
        let mut store = WakeStore::load(path.clone(), 1).await.unwrap();
        assert!(store.batch(60_001).is_empty());
        let g = store
            .issue(
                &entry,
                request(entry.root.clone()),
                "fixture-user".into(),
                1,
            )
            .await
            .unwrap();
        assert!(store
            .issue(
                &entry,
                request(entry.root.clone()),
                "fixture-user".into(),
                2
            )
            .await
            .is_err());
        let mut wrong = entry.clone();
        wrong.root = dir.path().join("wrong");
        assert!(!store.consume(&wrong, &g, 60_001).await.unwrap());
        assert!(store.consume(&entry, &g, 60_001).await.unwrap());
        assert!(!store.consume(&entry, &g, 60_002).await.unwrap());
        let mut restored = WakeStore::load(path, 60_003).await.unwrap();
        assert!(restored.batch(60_004).is_empty());
        assert_eq!(restored.get("p").unwrap().remaining_starts, 1);
        assert!(restored.batch(115_001).is_empty()); // no late catchup
        assert!(restored.consume(&entry, &g, 120_001).await.unwrap());
        assert!(restored.batch(180_001).is_empty()); // no replenishment
        restored.revoke("p").await.unwrap();
        let g = restored
            .issue(
                &entry,
                request(entry.root.clone()),
                "fixture-user".into(),
                200_000,
            )
            .await
            .unwrap();
        assert!(!restored.consume(&entry, &g, 1_000_000).await.unwrap());
        restored.revoke("p").await.unwrap();
        assert!(!restored.consume(&entry, &g, 260_000).await.unwrap());
        for key in [
            "goal",
            "model",
            "tool_consent",
            "instance_token",
            "mandate_id",
        ] {
            let mut v = serde_json::to_value(request(entry.root.clone())).unwrap();
            v[key] = serde_json::json!(true);
            assert!(serde_json::from_value::<WakeGrantRequest>(v).is_err());
        }
    }
}
