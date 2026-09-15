//! Durable file provenance (M3).
//!
//! Who read and who wrote each file, persisted in the coordination store so
//! attribution survives a restart and a cross-process stale write can name the
//! other writer instead of always saying "external or unknown".
//!
//! **Ordering.** A commit enqueues its observation in memory — the edit hot
//! path never waits on the store — and the queue is drained by
//! [`flush`], which the background task calls periodically, which
//! `file_activity::begin_writes` calls before every write, and which every
//! attribution lookup calls *before* it reads. That is the whole guarantee:
//! a reader can never depend on a record that is not durable yet, because it
//! flushes first. A write is never reported as attributed on the strength of a
//! record still sitting in the queue.
//!
//! **Provenance is not a lease.** These records say what happened. They grant
//! nothing, they block nothing, and no writer waits on them.
//!
//! Durability is additive: with no usable store the observations are dropped
//! and counted, attribution degrades to the process-local registry, and the
//! loss is reported through the store warning channel.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;

use refact_privacy::{FileRecord, PrivacyAuditError, PrivacyAudited};
use refact_work_core::store::records::{AccessKind, ActivityObservationRecord};
use refact_work_core::{CoordinationStore, ProjectAccess, ProjectKey, RuntimeKind, StoreError, WorkKey};

use crate::app_state::AppState;
use crate::caps::BaseModelRecord;
use crate::global_context::GlobalContext;

use super::error::WorkError;

/// Observations buffered before a flush. Past this the oldest are dropped and
/// counted rather than letting an unreachable store grow memory without bound.
const MAX_PENDING: usize = 8192;

/// Rows written per store round trip; the queue lock is never held across one.
const FLUSH_BATCH: usize = 256;

const FLUSH_INTERVAL: Duration = Duration::from_millis(1_000);

/// Retention runs every this many flushes.
const PRUNE_EVERY: u32 = 60;

const SHUTDOWN_POLL: Duration = Duration::from_millis(200);

pub const EXTERNAL_OR_UNKNOWN: &str =
    " The writer is external or unknown: nothing in this project recorded that write.";

/// One observed read or write, as the edit path saw it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedActivity {
    /// Logical file identity from `file_activity::resolve_key`.
    pub file_key: String,
    /// Physical file identity when the platform exposes one.
    pub file_alias: Option<String>,
    pub path: String,
    pub actor: Option<String>,
    pub tool: String,
    pub access: AccessKind,
    pub digest_before: Option<String>,
    pub digest_after: Option<String>,
    pub at_ms: i64,
}

/// Who last wrote a file, or an honest refusal to guess.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WriterAttribution {
    /// A cooperating writer of this project recorded the write.
    Named {
        actor: Option<String>,
        tool: String,
        instance_id: String,
        at_ms: i64,
    },
    /// No record: an editor, a shell, a formatter, a process outside our
    /// cooperation — or a record retention already evicted.
    ExternalOrUnknown,
}

impl WriterAttribution {
    pub fn is_named(&self) -> bool {
        matches!(self, WriterAttribution::Named { .. })
    }
}

/// Model-facing sentence for a refusal. Nothing is invented for a missing
/// record, and an evicted record reads exactly like an external one.
pub fn render(attribution: &WriterAttribution) -> String {
    match attribution {
        WriterAttribution::Named {
            actor: Some(actor),
            tool,
            ..
        } => format!(" It was last written by {actor} using {tool}."),
        WriterAttribution::Named {
            actor: None,
            tool,
            instance_id,
            ..
        } => format!(
            " It was last written by engine instance {instance_id} using {tool}, \
             with no chat attribution."
        ),
        WriterAttribution::ExternalOrUnknown => EXTERNAL_OR_UNKNOWN.to_string(),
    }
}

/// One provenance row as a model may see it, after privacy filtering.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ProvenanceEntry {
    pub path: String,
    pub actor: Option<String>,
    pub tool: Option<String>,
    pub access: AccessKind,
    pub observed_at_ms: i64,
}

struct Sink {
    store: Arc<CoordinationStore>,
    access: ProjectAccess,
    instance_id: String,
    checkout: Option<String>,
}

impl Sink {
    fn record(&self, activity: &ObservedActivity, sequence: u64) -> ActivityObservationRecord {
        let actor = activity.actor.clone().unwrap_or_default();
        ActivityObservationRecord {
            project: self.access.project().clone(),
            record_id: format!(
                "{}:{:013}:{:012}",
                self.instance_id, activity.at_ms, sequence
            ),
            work: WorkKey::new(RuntimeKind::Trajectory, actor),
            path: activity.path.clone(),
            access: activity.access,
            content_hash: activity.digest_after.clone(),
            observed_at_ms: activity.at_ms,
            file_key: activity.file_key.clone(),
            file_alias: activity.file_alias.clone(),
            instance_id: self.instance_id.clone(),
            tool_call: Some(activity.tool.clone()),
            content_hash_before: activity.digest_before.clone(),
            checkout: self.checkout.clone(),
        }
    }
}

fn slot() -> &'static RwLock<Option<Arc<Sink>>> {
    static SLOT: OnceLock<RwLock<Option<Arc<Sink>>>> = OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(None))
}

fn sink() -> Option<Arc<Sink>> {
    slot()
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

fn pending() -> &'static Mutex<VecDeque<ObservedActivity>> {
    static SLOT: OnceLock<Mutex<VecDeque<ObservedActivity>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(VecDeque::new()))
}

static UNRECORDED: AtomicU64 = AtomicU64::new(0);
static SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Observations this process could not persist, so nothing downstream
/// describes their attribution as durable.
pub fn unrecorded_observations() -> u64 {
    UNRECORDED.load(Ordering::Relaxed)
}

/// Bind provenance to a store, a project and this engine instance.
pub fn install_sink(
    store: Arc<CoordinationStore>,
    project: ProjectKey,
    instance_id: impl Into<String>,
    checkout: Option<String>,
) {
    let access = store.authorize(project);
    *slot()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Arc::new(Sink {
        store,
        access,
        instance_id: instance_id.into(),
        checkout,
    }));
}

pub fn uninstall_sink() {
    *slot()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
}

fn instance_id() -> String {
    format!(
        "{}:{}",
        hostname::get()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        std::process::id()
    )
}

/// Open this engine's provenance sink. Coordination being off, or a store that
/// cannot be opened, leaves the sink absent: edits keep working and attribution
/// falls back to the process-local registry.
pub async fn install(app: &AppState) {
    let settings = crate::file_activity::settings();
    if !settings.enabled {
        return;
    }
    let path = refact_work_core::default_store_path(&app.paths.cache_dir);
    let project = super::outbox::project_key_for_root(app, None).await;
    let checkout = crate::files_correction::get_project_dirs(app.gcx.clone())
        .await
        .into_iter()
        .next()
        .map(|root| root.to_string_lossy().into_owned());
    match super::outbox::store_for_path(&path, &settings).await {
        Ok(store) => install_sink(store, project, instance_id(), checkout),
        Err(error) => super::outbox::note_durability_unavailable(&path, error.to_string()),
    }
}

/// Queue one observation. Sync and allocation-bounded: the edit path never
/// waits on the store here.
pub fn observe(activity: ObservedActivity) {
    let mut queue = pending()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    while queue.len() >= MAX_PENDING {
        queue.pop_front();
        UNRECORDED.fetch_add(1, Ordering::Relaxed);
    }
    queue.push_back(activity);
}

fn take_batch(limit: usize) -> Vec<ObservedActivity> {
    let mut queue = pending()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let take = queue.len().min(limit);
    queue.drain(..take).collect()
}

fn discard_pending() -> usize {
    let mut queue = pending()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let dropped = queue.len();
    queue.clear();
    dropped
}

/// Drain the queue into the store. The queue lock is taken to move a batch out
/// and released before any store I/O.
pub async fn flush() {
    let Some(sink) = sink() else {
        let dropped = discard_pending();
        if dropped > 0 {
            UNRECORDED.fetch_add(dropped as u64, Ordering::Relaxed);
        }
        return;
    };
    loop {
        let batch = take_batch(FLUSH_BATCH);
        if batch.is_empty() {
            return;
        }
        let taken = batch.len();
        for activity in batch {
            let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let record = sink.record(&activity, sequence);
            if let Err(error) = sink
                .store
                .put_activity_observation(&sink.access, &record)
                .await
            {
                UNRECORDED.fetch_add(taken as u64, Ordering::Relaxed);
                super::outbox::note_durability_unavailable(sink.store.path(), error.to_string());
                return;
            }
        }
    }
}

/// Enforce the retention cap. Eviction is deterministic and an evicted row
/// reads back as `ExternalOrUnknown`, never as an invented observation.
pub async fn retain(max_records: usize) -> Result<usize, StoreError> {
    let Some(sink) = sink() else {
        return Ok(0);
    };
    sink.store
        .prune_activity_observations(&sink.access, max_records)
        .await
}

/// Deletion control: forget provenance for one logical file, or for the whole
/// project when `file_key` is `None`.
pub async fn forget(file_key: Option<&str>) -> Result<usize, StoreError> {
    let Some(sink) = sink() else {
        return Ok(0);
    };
    flush().await;
    sink.store
        .delete_activity_observations(&sink.access, file_key)
        .await
}

/// Who last wrote this file, other than `actor`, at or after `since_ms`.
///
/// The pending queue is flushed first, so this never misses a record this
/// process already committed and never names one that is not durable.
pub async fn last_writer(file_key: &str, actor: Option<&str>, since_ms: i64) -> WriterAttribution {
    flush().await;
    let Some(sink) = sink() else {
        super::metrics::record_unknown_file_attribution("no_provenance_sink");
        return WriterAttribution::ExternalOrUnknown;
    };
    let found = sink
        .store
        .last_activity_for_file(
            &sink.access,
            file_key,
            AccessKind::Write,
            since_ms,
            actor.filter(|actor| !actor.is_empty()),
        )
        .await;
    match found {
        Ok(Some(record)) => {
            super::metrics::record_unknown_file_attribution("named");
            WriterAttribution::Named {
                actor: Some(record.work.id).filter(|id| !id.is_empty()),
                tool: record.tool_call.unwrap_or_else(|| "an unnamed tool".into()),
                instance_id: record.instance_id,
                at_ms: record.observed_at_ms,
            }
        }
        Ok(None) => {
            super::metrics::record_unknown_file_attribution("external_or_unknown");
            WriterAttribution::ExternalOrUnknown
        }
        Err(error) => {
            super::metrics::record_unknown_file_attribution("store_unavailable");
            super::outbox::note_durability_unavailable(sink.store.path(), error.to_string());
            WriterAttribution::ExternalOrUnknown
        }
    }
}

/// Persisted provenance for the given logical file keys, newest first.
pub async fn entries_for_files(file_keys: &[String]) -> Vec<ProvenanceEntry> {
    let Some(sink) = sink() else {
        return Vec::new();
    };
    flush().await;
    let mut entries = Vec::new();
    for file_key in file_keys {
        for kind in AccessKind::ALL {
            let found = sink
                .store
                .last_activity_for_file(&sink.access, file_key, kind, 0, None)
                .await;
            if let Ok(Some(record)) = found {
                entries.push(ProvenanceEntry {
                    path: record.path,
                    actor: Some(record.work.id).filter(|id| !id.is_empty()),
                    tool: record.tool_call,
                    access: record.access,
                    observed_at_ms: record.observed_at_ms,
                });
            }
        }
    }
    entries.sort_by_key(|entry| (std::cmp::Reverse(entry.observed_at_ms), entry.path.clone()));
    entries
}

struct ProvenanceAudit {
    records: Vec<(usize, FileRecord)>,
}

impl PrivacyAudited for ProvenanceAudit {
    fn privacy_records(&self) -> Result<Vec<(usize, FileRecord)>, PrivacyAuditError> {
        Ok(self.records.clone())
    }
}

/// Destination-model filtering, applied before provenance reaches a model.
/// An entry whose path the policy will not send to this model is dropped, and
/// an unusable policy or an unknown model clears nothing.
pub fn clear_for_model(
    gcx: &Arc<GlobalContext>,
    model: Option<&BaseModelRecord>,
    entries: Vec<ProvenanceEntry>,
) -> Result<Vec<ProvenanceEntry>, WorkError> {
    let Some(model) = model.filter(|model| !model.id.trim().is_empty()) else {
        return Ok(Vec::new());
    };
    let before = super::privacy_guard::policy_key(gcx, model)?;
    let visible: Vec<ProvenanceEntry> = entries
        .into_iter()
        .filter(|entry| {
            let paths = std::iter::once(PathBuf::from(&entry.path));
            let Ok(records) = crate::privacy::records::declared_file_records(gcx, paths) else {
                return false;
            };
            crate::privacy::destinations::clear_for_model(
                gcx,
                ProvenanceAudit {
                    records: records.into_iter().enumerate().collect(),
                },
                model,
            )
            .is_ok()
        })
        .collect();
    if super::privacy_guard::policy_key(gcx, model)? != before {
        return Err(WorkError::policy_unavailable());
    }
    Ok(visible)
}

async fn wait_for_shutdown(shutdown: Arc<AtomicBool>) {
    while !shutdown.load(Ordering::SeqCst) {
        tokio::time::sleep(SHUTDOWN_POLL).await;
    }
}

/// Owner of flushing and retention: drains the queue on a fixed cadence and
/// prunes to the configured cap until the shutdown flag is raised.
pub async fn file_provenance_background_task(app: AppState, shutdown: Arc<AtomicBool>) {
    if shutdown.load(Ordering::SeqCst) {
        return;
    }
    install(&app).await;
    if shutdown.load(Ordering::SeqCst) {
        return;
    }
    let mut flushes: u32 = 0;
    loop {
        tokio::select! {
            _ = tokio::time::sleep(FLUSH_INTERVAL) => {}
            _ = wait_for_shutdown(shutdown.clone()) => break,
        }
        if shutdown.load(Ordering::SeqCst) {
            break;
        }
        flush().await;
        flushes = flushes.saturating_add(1);
        if flushes >= PRUNE_EVERY {
            flushes = 0;
            let cap = crate::file_activity::settings().store_max_activity_observations;
            match retain(cap).await {
                Ok(evicted) if evicted > 0 => {
                    tracing::info!("file provenance retention evicted {evicted} observations")
                }
                Ok(_) => {}
                Err(error) => tracing::warn!("file provenance retention failed: {error}"),
            }
        }
    }
    flush().await;
    let unrecorded = unrecorded_observations();
    if unrecorded > 0 {
        tracing::warn!(
            "{unrecorded} file observations were never persisted; \
             their attribution did not survive this process"
        );
    }
}

#[cfg(test)]
pub(crate) fn reset_for_tests() {
    uninstall_sink();
    discard_pending();
    UNRECORDED.store(0, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_work_core::CoordinationSettings;
    use serial_test::serial;

    fn settings() -> CoordinationSettings {
        CoordinationSettings::default()
    }

    async fn store() -> Arc<CoordinationStore> {
        Arc::new(
            CoordinationStore::open_in_memory(&settings())
                .await
                .unwrap(),
        )
    }

    fn write(file_key: &str, actor: Option<&str>, tool: &str, at_ms: i64) -> ObservedActivity {
        ObservedActivity {
            file_key: file_key.to_string(),
            file_alias: Some("id:66:1".to_string()),
            path: file_key.to_string(),
            actor: actor.map(str::to_string),
            tool: tool.to_string(),
            access: AccessKind::Write,
            digest_before: Some("before".to_string()),
            digest_after: Some("after".to_string()),
            at_ms,
        }
    }

    #[serial(file_provenance)]
    #[tokio::test]
    async fn a_write_recorded_by_one_engine_is_named_by_another_after_a_restart() {
        reset_for_tests();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("coordination.sqlite");
        let key = "/project/src/lib.rs";

        let writer = Arc::new(CoordinationStore::open(&path, &settings()).await.unwrap());
        install_sink(
            writer.clone(),
            ProjectKey::new("alpha"),
            "engine-a",
            Some("/project".to_string()),
        );
        observe(write(key, Some("chat-a"), "patch", 1_000));
        flush().await;
        uninstall_sink();
        Arc::into_inner(writer).unwrap().close().await.unwrap();

        let reader = Arc::new(CoordinationStore::open(&path, &settings()).await.unwrap());
        install_sink(reader, ProjectKey::new("alpha"), "engine-b", None);
        let attribution = last_writer(key, Some("chat-b"), 0).await;
        reset_for_tests();

        assert_eq!(
            attribution,
            WriterAttribution::Named {
                actor: Some("chat-a".to_string()),
                tool: "patch".to_string(),
                instance_id: "engine-a".to_string(),
                at_ms: 1_000,
            }
        );
        assert!(render(&attribution).contains("chat-a"));
        assert!(render(&attribution).contains("patch"));
    }

    #[serial(file_provenance)]
    #[tokio::test]
    async fn a_write_nobody_recorded_stays_external_or_unknown() {
        reset_for_tests();
        install_sink(store().await, ProjectKey::new("alpha"), "engine-a", None);
        observe(write(
            "/project/src/other.rs",
            Some("chat-a"),
            "patch",
            1_000,
        ));

        let unrecorded = last_writer("/project/src/lib.rs", Some("chat-b"), 0).await;
        let own = last_writer("/project/src/other.rs", Some("chat-a"), 0).await;
        reset_for_tests();

        assert_eq!(unrecorded, WriterAttribution::ExternalOrUnknown);
        assert_eq!(
            render(&unrecorded),
            EXTERNAL_OR_UNKNOWN,
            "a shell, an editor or a formatter must never be given a name"
        );
        assert_eq!(
            own,
            WriterAttribution::ExternalOrUnknown,
            "a chat's own write is not another writer"
        );
    }

    #[serial(file_provenance)]
    #[tokio::test]
    async fn retention_evicts_deterministically_and_evicted_provenance_reads_unknown() {
        reset_for_tests();
        install_sink(store().await, ProjectKey::new("alpha"), "engine-a", None);
        for index in 0..5i64 {
            observe(write(
                &format!("/project/src/{index}.rs"),
                Some("chat-a"),
                "patch",
                index,
            ));
        }
        flush().await;

        let evicted = retain(2).await.unwrap();
        let oldest = last_writer("/project/src/0.rs", Some("chat-b"), 0).await;
        let newest = last_writer("/project/src/4.rs", Some("chat-b"), 0).await;
        let again = retain(2).await.unwrap();
        reset_for_tests();

        assert_eq!(evicted, 3);
        assert_eq!(again, 0, "eviction is stable once the cap is met");
        assert_eq!(
            oldest,
            WriterAttribution::ExternalOrUnknown,
            "evicted provenance must read unknown, never be invented"
        );
        assert!(newest.is_named());
    }

    #[serial(file_provenance)]
    #[tokio::test]
    async fn deletion_control_removes_provenance_for_a_path_and_for_a_project() {
        reset_for_tests();
        install_sink(store().await, ProjectKey::new("alpha"), "engine-a", None);
        observe(write("/project/a.rs", Some("chat-a"), "patch", 1));
        observe(write("/project/b.rs", Some("chat-a"), "patch", 2));
        flush().await;

        assert_eq!(forget(Some("/project/a.rs")).await.unwrap(), 1);
        let forgotten = last_writer("/project/a.rs", Some("chat-b"), 0).await;
        let kept = last_writer("/project/b.rs", Some("chat-b"), 0).await;
        assert_eq!(forget(None).await.unwrap(), 1);
        let after_project_delete = last_writer("/project/b.rs", Some("chat-b"), 0).await;
        observe(write("/project/a.rs", Some("chat-a"), "patch", 3));
        flush().await;
        let recorded_again = last_writer("/project/a.rs", Some("chat-b"), 0).await;
        reset_for_tests();

        assert_eq!(forgotten, WriterAttribution::ExternalOrUnknown);
        assert!(kept.is_named());
        assert_eq!(
            after_project_delete,
            WriterAttribution::ExternalOrUnknown,
            "deleted provenance stays gone"
        );
        assert!(recorded_again.is_named());
    }

    #[serial(file_provenance)]
    #[tokio::test]
    async fn provenance_of_one_project_is_never_read_by_another() {
        reset_for_tests();
        let shared = store().await;
        install_sink(shared.clone(), ProjectKey::new("alpha"), "engine-a", None);
        observe(write("/project/shared.rs", Some("chat-alpha"), "patch", 1));
        flush().await;
        let mine = last_writer("/project/shared.rs", Some("chat-b"), 0).await;

        install_sink(shared, ProjectKey::new("beta"), "engine-a", None);
        let theirs = last_writer("/project/shared.rs", Some("chat-b"), 0).await;
        reset_for_tests();

        assert!(mine.is_named());
        assert_eq!(theirs, WriterAttribution::ExternalOrUnknown);
    }

    #[serial(file_provenance)]
    #[tokio::test]
    async fn without_a_store_edits_keep_working_and_the_loss_is_reported() {
        reset_for_tests();
        observe(write("/project/a.rs", Some("chat-a"), "patch", 1));
        flush().await;
        let attribution = last_writer("/project/a.rs", Some("chat-b"), 0).await;
        let unrecorded = unrecorded_observations();
        reset_for_tests();

        assert_eq!(attribution, WriterAttribution::ExternalOrUnknown);
        assert_eq!(
            unrecorded, 1,
            "an observation that was never persisted has to be counted as lost"
        );
    }

    #[serial(file_provenance)]
    #[tokio::test]
    async fn provenance_is_filtered_for_the_destination_model_before_a_model_sees_it() {
        reset_for_tests();
        let dir = tempfile::tempdir().unwrap();
        let guarded = dir.path().join("guarded.rs");
        std::fs::write(&guarded, "secret").unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        gcx.privacy_policy_load.write().unwrap().policy = Arc::new(refact_privacy::PrivacyPolicy {
            zones: vec![refact_privacy::Zone {
                name: "guarded".into(),
                patterns: vec![guarded.to_string_lossy().into_owned()],
                send_to: vec!["trusted".into()],
                on_shell_read: refact_privacy::ShellBehavior::Withhold,
            }],
            ..Default::default()
        });
        install_sink(store().await, ProjectKey::new("alpha"), "engine-a", None);
        let key = guarded.to_string_lossy().into_owned();
        observe(ObservedActivity {
            path: key.clone(),
            ..write(&key, Some("chat-a"), "patch", 1)
        });
        flush().await;
        let entries = entries_for_files(&[key]).await;

        let trusted = clear_for_model(
            &gcx,
            Some(&BaseModelRecord {
                id: "trusted/model".into(),
                ..Default::default()
            }),
            entries.clone(),
        )
        .unwrap();
        let other = clear_for_model(
            &gcx,
            Some(&BaseModelRecord {
                id: "other/model".into(),
                ..Default::default()
            }),
            entries.clone(),
        )
        .unwrap();
        let unknown_model = clear_for_model(&gcx, None, entries.clone()).unwrap();
        reset_for_tests();

        assert_eq!(entries.len(), 1);
        assert_eq!(trusted.len(), 1);
        assert_eq!(trusted[0].actor.as_deref(), Some("chat-a"));
        assert!(
            other.is_empty(),
            "a model the zone does not send to must not see the path or the writer"
        );
        assert!(unknown_model.is_empty());
    }
}
