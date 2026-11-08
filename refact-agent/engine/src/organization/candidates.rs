use std::collections::{BTreeMap, HashSet};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use crate::{app_state::AppState, coordination::scope::WorkCaller};
use super::{
    sources::{self, Source},
    store::{self, Store},
    types::*,
};
pub use super::sources::Candidate;

const MAX_CANDIDATES: usize = 12;

#[derive(Debug, Serialize)]
pub struct CandidatePage {
    pub revision: u64,
    pub candidates: Vec<Candidate>,
    pub next_cursor: Option<String>,
    pub partial: bool,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReviewDecision {
    Assign,
    Keep,
    Defer,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewOutcome {
    pub item: Item,
    pub fingerprint: String,
    pub decision: ReviewDecision,
    #[serde(default)]
    pub topic_name: Option<String>,
    #[serde(default)]
    pub tag_names: Vec<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewRequest {
    pub expected_revision: u64,
    pub request_id: String,
    pub outcomes: Vec<ReviewOutcome>,
    #[serde(default)]
    pub operations: Vec<Op>,
}

fn schema(conn: &Connection) -> Result<(), Error> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS organization_scan(principal TEXT PRIMARY KEY, after_key TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS organization_reviewed(item TEXT PRIMARY KEY, fingerprint TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS organization_issued(principal TEXT NOT NULL,item TEXT NOT NULL,fingerprint TEXT NOT NULL,source TEXT NOT NULL,records TEXT NOT NULL,policy TEXT NOT NULL,PRIMARY KEY(principal,item));
        CREATE TABLE IF NOT EXISTS organization_review_requests(id TEXT PRIMARY KEY,payload TEXT NOT NULL,response TEXT NOT NULL);")?;
    Ok(())
}

fn cycle(sources: Vec<Source>, after: &str, limit: usize) -> Vec<Source> {
    let split = sources.partition_point(|s| s.item.key().as_str() <= after);
    sources[split..]
        .iter()
        .chain(sources[..split].iter())
        .take(limit)
        .cloned()
        .collect()
}

pub async fn list_candidates(
    app: &AppState,
    caller: &WorkCaller,
    limit: usize,
) -> Result<CandidatePage, Error> {
    let normalized = super::service::project_caller(app, caller).await?;
    let caller = &normalized;
    if !(1..=MAX_CANDIDATES).contains(&limit) {
        return Err(Error::invalid("candidate limit must be 1..12"));
    }
    let model = caller.model.clone().ok_or_else(Error::denied)?;
    let policy = crate::coordination::privacy_guard::policy_key(&app.gcx, &model)
        .map_err(|_| Error::denied())?;
    let all = sources::catalog(app, caller).await?;
    let read_root = caller.source_root.clone();
    let read_principal = caller.principal.clone();
    let (after, revision) = super::service::blocking(move || {
        let store = Store::open(&read_root)?;
        schema(&store.conn)?;
        let after = store
            .conn
            .query_row(
                "SELECT after_key FROM organization_scan WHERE principal=?1",
                [&read_principal],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .unwrap_or_default();
        Ok((after, store.revision()?))
    })
    .await?;
    let selected = cycle(all, &after, limit);
    let next = selected.last().map(|s| s.item.key());
    let mut candidates = Vec::new();
    let mut issued = Vec::new();
    for source in &selected {
        if let Ok(candidate) = sources::resolve(
            app.gcx.clone(),
            caller.source_root.clone(),
            source.clone(),
            model.clone(),
        )
        .await
        {
            issued.push((source.clone(), candidate.clone()));
            candidates.push(candidate);
        }
    }
    if crate::coordination::privacy_guard::policy_key(&app.gcx, &model)
        .map_err(|_| Error::denied())?
        != policy
    {
        return Err(Error::denied());
    }
    let write_root = caller.source_root.clone();
    let write_principal = caller.principal.clone();
    let write_next = next.clone();
    let gcx = app.gcx.clone();
    let cancellation = std::sync::Arc::new(std::sync::atomic::AtomicU8::new(0));
    let guard = CancelReviewOnDrop(cancellation.clone());
    let reviewed = super::service::blocking(move || {
        ensure_active(&cancellation, &gcx.shutdown_flag)?;
    let mut store = Store::open(&write_root)?;
    schema(&store.conn)?;
    let tx = store
        .conn
        .transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut reviewed = HashSet::new();
    for (source, candidate) in issued {
        let done: Option<String> = tx
            .query_row(
                "SELECT fingerprint FROM organization_reviewed WHERE item=?1",
                [candidate.item.key()],
                |r| r.get(0),
            )
            .optional()?;
        if done.as_deref() == Some(&candidate.fingerprint) {
            reviewed.insert(candidate.item.key());
            continue;
        }
        tx.execute("INSERT INTO organization_issued VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(principal,item) DO UPDATE SET fingerprint=excluded.fingerprint,source=excluded.source,records=excluded.records,policy=excluded.policy", params![write_principal,candidate.item.key(),candidate.fingerprint,serde_json::to_string(&source)?,serde_json::to_string(&candidate.source_records)?,policy])?;
    }
    if let Some(next) = &write_next {
        // This is an attempted-key checkpoint, never a reviewed/coverage mark.
        // Advancing over denied/deferred sources prevents oldest-item starvation.
        tx.execute("INSERT INTO organization_scan VALUES(?1,?2) ON CONFLICT(principal) DO UPDATE SET after_key=excluded.after_key", params![write_principal,next])?;
    }
    admit_commit(&cancellation, &gcx.shutdown_flag)?;
    tx.commit()?;
    Ok(reviewed)
    }).await?;
    drop(guard);
    candidates.retain(|c| !reviewed.contains(&c.item.key()));
    Ok(CandidatePage {
        revision,
        candidates,
        next_cursor: next,
        partial: true,
    })
}

fn bounded_records(records: &mut Vec<refact_privacy::FileRecord>) -> Result<(), Error> {
    let mut seen = HashSet::new();
    records.retain(|record| seen.insert(record.clone()));
    records.sort_by_key(|record| {
        (
            record.path.clone(),
            record.zone.clone(),
            format!("{:?}", record.attribution),
        )
    });
    let bytes = records
        .iter()
        .try_fold(0usize, |total, record| {
            total
                .checked_add(record.path.len())
                .and_then(|value| value.checked_add(record.zone.len()))
                .and_then(|value| value.checked_add(64))
        })
        .ok_or_else(Error::denied)?;
    if bytes > 128 * 1024 || serde_json::to_vec(records)?.len() > 128 * 1024 {
        return Err(Error::denied());
    }
    Ok(())
}

fn validate_request(request: &ReviewRequest) -> Result<(), Error> {
    if request.request_id.is_empty()
        || request.request_id.len() > 128
        || request.outcomes.is_empty()
        || request.outcomes.len() > MAX_CANDIDATES
        || request.operations.len() > 100
        || serde_json::to_vec(request)?.len() > 128 * 1024
    {
        return Err(Error::invalid("invalid review bounds"));
    }
    let mut seen = HashSet::new();
    for outcome in &request.outcomes {
        if !seen.insert(outcome.item.key())
            || outcome.fingerprint.len() != 64
            || !outcome.fingerprint.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(Error::invalid("invalid or duplicate review source"));
        }
    }
    if !request.operations.is_empty() {
        return Err(Error::invalid("review uses semantic names, not operations"));
    }
    for outcome in &request.outcomes {
        if outcome.tag_names.len() > 16
            || (outcome.decision != ReviewDecision::Assign
                && (outcome.topic_name.is_some() || !outcome.tag_names.is_empty()))
        {
            return Err(Error::invalid("only assign outcomes may name taxonomy"));
        }
    }
    Ok(())
}

pub async fn commit_review(
    app: &AppState,
    caller: &WorkCaller,
    request: ReviewRequest,
) -> Result<ApplyResponse, Error> {
    commit_review_with_context(app, caller, request, Vec::new()).await
}

pub(crate) async fn commit_review_with_context(
    app: &AppState,
    caller: &WorkCaller,
    request: ReviewRequest,
    mut context_records: Vec<refact_privacy::FileRecord>,
) -> Result<ApplyResponse, Error> {
    let normalized = super::service::project_caller(app, caller).await?;
    let caller = &normalized;
    validate_request(&request)?;
    bounded_records(&mut context_records)?;
    let model = caller.model.clone().ok_or_else(Error::denied)?;
    let policy = crate::coordination::privacy_guard::policy_key(&app.gcx, &model)
        .map_err(|_| Error::denied())?;
    let payload =
        serde_json::to_string(&(&caller.principal, &model.id, &request, &context_records))?;
    let read_root = caller.source_root.clone();
    let read_principal = caller.principal.clone();
    let read_request = request.clone();
    let read_payload = payload.clone();
    let read_policy = policy.clone();
    let (issued, replayed) = super::service::blocking(move || {
    let mut issued = BTreeMap::new();
        let store = Store::open(&read_root)?;
        schema(&store.conn)?;
        if let Some(response) = replay(&store.conn, &read_request.request_id, &read_payload)? {
            return Ok((BTreeMap::new(), Some(response)));
        }
        for outcome in &read_request.outcomes {
            let row: Option<(String, String, String)> = store.conn.query_row("SELECT fingerprint,source,policy FROM organization_issued WHERE principal=?1 AND item=?2", params![read_principal,outcome.item.key()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
            let Some((fingerprint, body, issued_policy)) = row else {
                return Err(Error::denied());
            };
            if fingerprint != outcome.fingerprint || issued_policy != read_policy {
                return Err(Error::conflict(
                    "candidate changed; request candidates again",
                ));
            }
            let source: Source = serde_json::from_str(&body)?;
            if source.item != outcome.item {
                return Err(Error::denied());
            }
            issued.insert(outcome.item.key(), source);
        }
    Ok((issued, None))
    }).await?;
    if let Some(response) = replayed {
        return Ok(response);
    }
    let mut records = context_records;
    let root = caller.source_root.clone();
    let principal = caller.principal.clone();
    let gcx = app.gcx.clone();
    let cancellation = std::sync::Arc::new(std::sync::atomic::AtomicU8::new(0));
    let guard = CancelReviewOnDrop(cancellation.clone());
    let result = super::service::blocking(move || {
        ensure_active(&cancellation, &gcx.shutdown_flag)?;
        if crate::coordination::privacy_guard::policy_key(&gcx, &model).map_err(|_| Error::denied())? != policy { return Err(Error::denied()); }
        let mut store = Store::open(&root)?;
        schema(&store.conn)?;
        let mut stamps = BTreeMap::new();
        for outcome in &request.outcomes {
            let source = issued.get(&outcome.item.key()).ok_or_else(Error::denied)?;
            let stamp = sources::source_stamp(source)?;
            let fresh = sources::project(&gcx, &root, source, &model)?;
            if fresh.fingerprint != outcome.fingerprint || !sources::source_unchanged(source, &stamp) { return Err(Error::conflict("source changed before review commit")); }
            records.extend(fresh.source_records);
            stamps.insert(outcome.item.key(), stamp);
        }
        bounded_records(&mut records)?;
        let paths = records.iter().map(|record| {
            let path = std::path::PathBuf::from(&record.path);
            if path.is_absolute() { path } else { root.join(path) }
        });
        let refreshed = crate::privacy::records::declared_file_records(&gcx, paths)
            .map_err(|_| Error::denied())?;
        records.extend(refreshed);
        bounded_records(&mut records)?;
        struct Audit(Vec<refact_privacy::FileRecord>);
        impl refact_privacy::PrivacyAudited for Audit {
            fn privacy_records(&self) -> Result<Vec<(usize, refact_privacy::FileRecord)>, refact_privacy::PrivacyAuditError> {
                Ok(self.0.iter().cloned().enumerate().collect())
            }
        }
        crate::privacy::destinations::clear_for_model(&gcx, Audit(records.clone()), &model)
            .map_err(|_| Error::denied())?;
        ensure_active(&cancellation, &gcx.shutdown_flag)?;
        let tx = store.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(response) = replay(&tx, &request.request_id, &payload)? { return Ok(response); }
        if store::revision(&tx)? != request.expected_revision { return Err(Error::conflict("organization revision changed")); }
        for outcome in &request.outcomes {
            let source = issued.get(&outcome.item.key()).ok_or_else(Error::denied)?;
            let stamp = stamps.get(&outcome.item.key()).ok_or_else(Error::denied)?;
            if !sources::source_unchanged(source, stamp) { return Err(Error::conflict("source changed while waiting to commit")); }
            let current: Option<String> = tx.query_row("SELECT fingerprint FROM organization_issued WHERE principal=?1 AND item=?2", params![principal,outcome.item.key()], |r| r.get(0)).optional()?;
            if current.as_deref() != Some(&outcome.fingerprint) { return Err(Error::conflict("candidate replaced")); }
        }
        ensure_active(&cancellation, &gcx.shutdown_flag)?;
        let mut results = Vec::new();
        for outcome in &request.outcomes {
            if outcome.decision != ReviewDecision::Assign { continue; }
            let topic_id = outcome.topic_name.as_ref().map(|name| resolve_name(&tx, TaxonomyKind::Topics, name)).transpose()?;
            let tag_ids = outcome.tag_names.iter().map(|name| resolve_name(&tx, TaxonomyKind::Tags, name)).collect::<Result<Vec<_>,_>>()?;
            results.extend(store::apply_ops(&tx, &[Op::AssignItem { item: outcome.item.clone(), topic_id, tag_ids, topic_locked: None, tags_locked: None }], Authority::Model)?);
        }
        for outcome in &request.outcomes {
            if outcome.decision != ReviewDecision::Defer {
                tx.execute("INSERT INTO organization_reviewed VALUES(?1,?2) ON CONFLICT(item) DO UPDATE SET fingerprint=excluded.fingerprint", params![outcome.item.key(),outcome.fingerprint])?;
            }
        }
        store::merge_provenance(&tx, &records)?;
        let revision = request.expected_revision.checked_add(1).filter(|v| *v <= i64::MAX as u64).ok_or_else(|| Error::conflict("revision exhausted"))?;
        let response = ApplyResponse { revision, results };
        tx.execute("UPDATE meta SET revision=?1 WHERE id=1", [revision])?;
        tx.execute("INSERT INTO organization_review_requests VALUES(?1,?2,?3)", params![request.request_id,payload,serde_json::to_string(&response)?])?;
        if crate::coordination::privacy_guard::policy_key(&gcx, &model).map_err(|_| Error::denied())? != policy { return Err(Error::denied()); }
        for outcome in &request.outcomes {
            let source = issued.get(&outcome.item.key()).ok_or_else(Error::denied)?;
            let stamp = stamps.get(&outcome.item.key()).ok_or_else(Error::denied)?;
            if !sources::source_unchanged(source, stamp) { return Err(Error::conflict("source changed before commit admission")); }
            #[cfg(not(unix))]
            if !sources::fingerprint_matches(&root, source, &outcome.fingerprint) {
                return Err(Error::conflict("source content changed before commit admission"));
            }
        }
        admit_commit(&cancellation, &gcx.shutdown_flag)?;
        tx.commit()?;
        Ok(response)
    }).await;
    drop(guard);
    result
}

fn replay(conn: &Connection, id: &str, payload: &str) -> Result<Option<ApplyResponse>, Error> {
    let saved: Option<(String, String)> = conn
        .query_row(
            "SELECT payload,response FROM organization_review_requests WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    match saved {
        Some((old, response)) if old == payload => Ok(Some(serde_json::from_str(&response)?)),
        Some(_) => Err(Error::conflict("review request id reused")),
        None => Ok(None),
    }
}

fn resolve_name(conn: &Connection, kind: TaxonomyKind, name: &str) -> Result<String, Error> {
    let normalized = name.split_whitespace().collect::<Vec<_>>().join(" ");
    let found: Option<String> = conn
        .query_row(
            "SELECT id FROM taxonomy WHERE kind=?1 AND name_key=?2 AND archived=0",
            params![kind.key(), normalized.to_lowercase()],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(id) = found {
        return Ok(id);
    }
    let op = match kind {
        TaxonomyKind::Topics => Op::CreateTopic {
            name: normalized,
        },
        TaxonomyKind::Tags => Op::CreateTag {
            name: normalized,
        },
    };
    store::apply_ops(conn, &[op], Authority::Model)?
        .into_iter()
        .next()
        .and_then(|r| r.id)
        .ok_or_else(|| Error::storage("missing created taxonomy id"))
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::caps::BaseModelRecord;
    async fn fixture(count: usize) -> (tempfile::TempDir, AppState, WorkCaller) {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().into()];
        for i in 0..count {
            let task = dir.path().join(format!(".refact/tasks/task-{i:02}"));
            std::fs::create_dir_all(&task).unwrap();
            std::fs::write(
                task.join("meta.yaml"),
                format!("id: task-{i:02}\ntitle: Synthetic title {i}\n"),
            )
            .unwrap();
        }
        let caller = WorkCaller {
            source_root: dir.path().into(),
            checkout_root: dir.path().into(),
            root_chat_id: None,
            principal: "fixture".into(),
            model: Some(BaseModelRecord {
                id: "trusted/model".into(),
                ..Default::default()
            }),
        };
        (dir, AppState::from_gcx(gcx).await, caller)
    }
    fn request(page: &CandidatePage, decision: ReviewDecision, id: &str) -> ReviewRequest {
        ReviewRequest {
            expected_revision: page.revision,
            request_id: id.into(),
            operations: vec![],
            outcomes: page
                .candidates
                .iter()
                .map(|c| ReviewOutcome {
                    item: c.item.clone(),
                    fingerprint: c.fingerprint.clone(),
                    decision,
                    topic_name: None,
                    tag_names: vec![],
                })
                .collect(),
        }
    }
    fn reviewed(root: &std::path::Path) -> usize {
        Store::open(root)
            .unwrap()
            .conn
            .query_row("SELECT COUNT(*) FROM organization_reviewed", [], |r| {
                r.get(0)
            })
            .unwrap()
    }
    #[tokio::test]
    async fn actual_pipeline_cycles_old_sources_and_defer_is_not_reviewed() {
        let (dir, app, caller) = fixture(15).await;
        let first = list_candidates(&app, &caller, 12).await.unwrap();
        assert_eq!(first.candidates.len(), 12);
        commit_review(
            &app,
            &caller,
            request(&first, ReviewDecision::Defer, "defer"),
        )
        .await
        .unwrap();
        assert_eq!(reviewed(dir.path()), 0);
        let second = list_candidates(&app, &caller, 12).await.unwrap();
        let visited: HashSet<_> = first
            .candidates
            .iter()
            .chain(&second.candidates)
            .map(|c| c.item.key())
            .collect();
        assert_eq!(
            visited.len(),
            15,
            "old items cannot starve behind defer or page cap"
        );
        let request = request(&second, ReviewDecision::Keep, "keep");
        let response = commit_review(&app, &caller, request.clone()).await.unwrap();
        assert_eq!(reviewed(dir.path()), 12);
        assert_eq!(
            commit_review(&app, &caller, request).await.unwrap(),
            response
        );
    }
    #[tokio::test]
    async fn actual_pipeline_refuses_replacement_without_poisoning_review() {
        let (dir, app, caller) = fixture(1).await;
        let page = list_candidates(&app, &caller, 12).await.unwrap();
        let path = dir.path().join(".refact/tasks/task-00/meta.yaml");
        std::fs::write(&path, "id: task-00\ntitle: Changed title\n").unwrap();
        assert!(
            commit_review(&app, &caller, request(&page, ReviewDecision::Keep, "stale"))
                .await
                .is_err()
        );
        assert_eq!(reviewed(dir.path()), 0);
        let next = list_candidates(&app, &caller, 12).await.unwrap();
        assert_eq!(next.candidates[0].title.as_deref(), Some("Changed title"));
        assert_ne!(
            page.candidates[0].fingerprint,
            next.candidates[0].fingerprint
        );
        commit_review(&app, &caller, request(&next, ReviewDecision::Keep, "fresh"))
            .await
            .unwrap();
        assert!(list_candidates(&app, &caller, 12)
            .await
            .unwrap()
            .candidates
            .is_empty());
    }
    #[tokio::test]
    async fn actual_pipeline_denial_leaks_no_title_and_assign_is_atomic() {
        let (dir, app, caller) = fixture(1).await;
        app.gcx.privacy_policy_load.write().unwrap().policy =
            std::sync::Arc::new(refact_privacy::PrivacyPolicy {
                blocked: vec!["**/meta.yaml".into()],
                ..Default::default()
            });
        let denied = list_candidates(&app, &caller, 12).await.unwrap();
        assert!(denied.candidates.is_empty());
        assert!(!serde_json::to_string(&denied)
            .unwrap()
            .contains("Synthetic"));
        app.gcx.privacy_policy_load.write().unwrap().policy = Default::default();
        let page = list_candidates(&app, &caller, 12).await.unwrap();
        let mut assign = request(&page, ReviewDecision::Assign, "assign");
        assign.outcomes[0].topic_name = Some("Synthetic topic".into());
        assign.outcomes[0].tag_names = vec!["Fixture".into()];
        commit_review(&app, &caller, assign).await.unwrap();
        let store = Store::open(dir.path()).unwrap();
        let a = store::assignment(&store.conn, &page.candidates[0].item).unwrap();
        assert!(a.topic_id.is_some());
        assert_eq!(a.tag_ids.len(), 1);
        assert_eq!(reviewed(dir.path()), 1);
        assert!(!store::provenance(&store.conn).unwrap().is_empty());
    }

    #[tokio::test]
    async fn derived_taxonomy_keeps_additional_model_context_provenance() {
        let (dir, app, caller) = fixture(1).await;
        let guarded = dir.path().join("private-context.txt");
        std::fs::write(&guarded, "guarded context").unwrap();
        app.gcx.privacy_policy_load.write().unwrap().policy =
            std::sync::Arc::new(refact_privacy::PrivacyPolicy {
                zones: vec![refact_privacy::Zone {
                    name: "guarded".into(),
                    patterns: vec![guarded.to_string_lossy().into_owned()],
                    send_to: vec!["trusted".into()],
                    on_shell_read: refact_privacy::ShellBehavior::Withhold,
                }],
                ..Default::default()
            });
        let records = crate::privacy::records::declared_file_records(&app.gcx, [guarded]).unwrap();
        let page = list_candidates(&app, &caller, 1).await.unwrap();
        let mut assign = request(&page, ReviewDecision::Assign, "context-derived");
        assign.outcomes[0].topic_name = Some("Derived from model context".into());
        commit_review_with_context(&app, &caller, assign, records.clone())
            .await
            .unwrap();
        assert!(store::provenance(&Store::open(dir.path()).unwrap().conn)
            .unwrap()
            .iter()
            .any(|record| records.contains(record)));
        let other = WorkCaller {
            model: Some(BaseModelRecord {
                id: "other/model".into(),
                ..Default::default()
            }),
            ..caller
        };
        assert!(
            super::super::service::taxonomy(&app, &other, TaxonomyKind::Topics, 200, None)
                .await
                .is_err()
        );
    }
}

pub fn status(caller: &WorkCaller) -> Result<serde_json::Value, Error> {
    let store = Store::open(&caller.source_root)?;
    schema(&store.conn)?;
    let reviewed: u64 =
        store
            .conn
            .query_row("SELECT COUNT(*) FROM organization_reviewed", [], |r| {
                r.get(0)
            })?;
    let issued: u64 = store.conn.query_row(
        "SELECT COUNT(*) FROM organization_issued WHERE principal=?1",
        [&caller.principal],
        |r| r.get(0),
    )?;
    let attempted_checkpoint: Option<String> = store
        .conn
        .query_row(
            "SELECT after_key FROM organization_scan WHERE principal=?1",
            [&caller.principal],
            |r| r.get(0),
        )
        .optional()?;
    Ok(
        serde_json::json!({"revision":store.revision()?,"reviewed_fingerprints":reviewed,"issued_candidates":issued,"attempted_checkpoint":attempted_checkpoint,"partial":true,"coverage_complete":false,"coverage":"bounded_cyclic_scan; reviewed counts successful source fingerprints, not current-source coverage"}),
    )
}

pub(crate) struct CancelReviewOnDrop(pub(crate) std::sync::Arc<std::sync::atomic::AtomicU8>);
impl Drop for CancelReviewOnDrop {
    fn drop(&mut self) {
        let _ = self.0.compare_exchange(
            0,
            1,
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
        );
    }
}
pub(crate) fn ensure_active(
    cancel: &std::sync::atomic::AtomicU8,
    shutdown: &std::sync::atomic::AtomicBool,
) -> Result<(), Error> {
    if cancel.load(std::sync::atomic::Ordering::SeqCst) != 0
        || shutdown.load(std::sync::atomic::Ordering::SeqCst)
    {
        return Err(Error::conflict("review cancelled or engine shutting down"));
    }
    Ok(())
}
pub(crate) fn admit_commit(
    cancel: &std::sync::atomic::AtomicU8,
    shutdown: &std::sync::atomic::AtomicBool,
) -> Result<(), Error> {
    ensure_active(cancel, shutdown)?;
    // This CAS is the admission linearization point: cancellation before it
    // rolls back; cancellation after it cannot revoke an already admitted commit.
    cancel
        .compare_exchange(
            0,
            2,
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
        )
        .map_err(|_| Error::conflict("review cancelled before commit"))?;
    if shutdown.load(std::sync::atomic::Ordering::SeqCst) {
        return Err(Error::conflict("engine shutting down"));
    }
    Ok(())
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    #[tokio::test]
    async fn dropped_owner_cancels_waiting_blocking_transaction_before_admission() {
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicU8::new(0));
        let guard = CancelReviewOnDrop(cancel.clone());
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.sqlite");
        let check = path.clone();
        let worker = tokio::task::spawn_blocking(move || {
            let mut conn = Connection::open(path).unwrap();
            conn.execute_batch("CREATE TABLE fixture(value INTEGER);")
                .unwrap();
            let tx = conn.transaction().unwrap();
            tx.execute("INSERT INTO fixture VALUES(1)", []).unwrap();
            started_tx.send(()).unwrap();
            resume_rx.recv().unwrap();
            if admit_commit(&cancel, &std::sync::atomic::AtomicBool::new(false)).is_ok() {
                tx.commit().unwrap();
            }
        });
        started_rx.await.unwrap();
        drop(guard);
        resume_tx.send(()).unwrap();
        worker.await.unwrap();
        let count: usize = Connection::open(check)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM fixture", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            count, 0,
            "detached worker must roll back after owner cancellation"
        );
    }
    #[test]
    fn shutdown_prevents_commit_admission() {
        assert!(admit_commit(
            &std::sync::atomic::AtomicU8::new(0),
            &std::sync::atomic::AtomicBool::new(true)
        )
        .is_err());
    }
}
