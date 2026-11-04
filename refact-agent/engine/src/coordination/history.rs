use std::path::Path;

use crate::app_state::AppState;
use crate::chat::trajectory_index::{TrajectoryIndexEntry, TrajectoryIndexSource, WorkReferenceState};
use refact_work_core::WorkKey;
use serde::{Deserialize, Serialize};

use super::roots::{self, ProjectRoots};
use super::types::{Disclosure, Freshness, WorkKindTag, WorkMetadata, WorkRow, WorkStateTag};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkIndexMetadata {
    pub records: Vec<refact_privacy::FileRecord>,
    pub provenance_complete: bool,
    pub source_revision: u64,
    pub measured_tokens: Option<u64>,
}

impl WorkIndexMetadata {
    pub fn from_values(messages: &[serde_json::Value], source_revision: u64) -> Self {
        let settings = crate::file_activity::settings();
        let mut provenance = super::provenance::WorkProvenance::default();
        let mut measured_tokens: Option<u64> = None;
        for value in messages {
            let Ok(message) =
                serde_json::from_value::<crate::call_validation::ChatMessage>(value.clone())
            else {
                return Self {
                    records: vec![],
                    provenance_complete: false,
                    source_revision,
                    measured_tokens: None,
                };
            };
            provenance.observe(&message, messages.len(), &settings);
            if let Some(usage) = message.usage {
                measured_tokens = Some(
                    measured_tokens
                        .unwrap_or(0)
                        .saturating_add(usage.total_tokens as u64),
                );
            }
        }
        let (records, provenance_complete) = provenance.snapshot(messages.len(), &settings);
        Self {
            records,
            provenance_complete,
            source_revision,
            measured_tokens,
        }
    }

    #[cfg(test)]
    pub fn from_messages(
        messages: &[crate::call_validation::ChatMessage],
        source_revision: u64,
    ) -> Self {
        let settings = crate::file_activity::settings();
        let provenance = super::provenance::WorkProvenance::from_messages(messages, &settings);
        let (records, provenance_complete) = provenance.snapshot(messages.len(), &settings);
        Self {
            records,
            provenance_complete,
            source_revision,
            measured_tokens: messages
                .iter()
                .filter_map(|message| message.usage.as_ref())
                .map(|usage| usage.total_tokens as u64)
                .reduce(u64::saturating_add),
        }
    }
}

fn timestamp(value: &str) -> Option<u64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|time| time.timestamp_millis().max(0) as u64)
}

pub(super) fn row_from_entry(entry: &TrajectoryIndexEntry, roots: &ProjectRoots) -> WorkRow {
    let (kind, task_id, card_id) = match &entry.source {
        TrajectoryIndexSource::Task {
            task_id,
            role,
            card_id,
            ..
        } => (
            if role == "planner" {
                WorkKindTag::TaskPlanner
            } else if role == "linked" {
                WorkKindTag::Chat
            } else {
                WorkKindTag::BoardAgent
            },
            Some(task_id.clone()),
            card_id.clone(),
        ),
        _ => (
            if entry.parent_id.is_some() {
                WorkKindTag::Subagent
            } else {
                WorkKindTag::Chat
            },
            None,
            None,
        ),
    };
    let updated_at_ms = timestamp(&entry.updated_at).unwrap_or(0);
    let mut root = entry.worktree.as_ref().map_or_else(
        || roots.project_root(),
        |meta| {
            roots.get(&meta.id).cloned().unwrap_or_else(|| {
                let mut root = roots::worktree_root_ref(meta);
                root.path = None;
                root
            })
        },
    );
    if root.project != roots.project {
        root.path = None;
    }
    let work = entry.work.as_ref();
    let settings = crate::file_activity::settings();
    let provenance_complete = false;
    WorkRow {
        key: WorkKey::trajectory(&entry.id),
        metadata: WorkMetadata {
            task_id,
            card_id,
            freshness: if updated_at_ms == 0 {
                Freshness::Unknown
            } else if super::service::now_ms().saturating_sub(updated_at_ms)
                <= settings.recent_window_secs.saturating_mul(1000)
            {
                Freshness::Recent
            } else {
                Freshness::Stale
            },
            last_activity_at: timestamp(&entry.updated_at),
            measured_tokens: work.and_then(|work| work.measured_tokens),
            source_revision: work.map_or(0, |work| work.source_revision),
            ..Default::default()
        },
        kind,
        state: WorkStateTag::Unknown,
        attention: false,
        title: Some(entry.title.clone()).filter(|title| !title.is_empty()),
        summary: None,
        root,
        parent_chat_id: entry.parent_id.clone(),
        root_chat_id: entry.root_chat_id.clone(),
        chat_id: Some(entry.id.clone()),
        model: Some(entry.model.clone()).filter(|model| !model.is_empty()),
        current_tool: None,
        declared_files: vec![],
        observed_files: vec![],
        queue_size: 0,
        step_count: 0,
        open_questions: 0,
        updated_at_ms,
        stale: true,
        disclosure: Disclosure::Full,
        source_records: vec![],
        provenance_complete,
    }
}

pub(super) async fn indexed_rows(
    app: &AppState,
    roots: &ProjectRoots,
    degraded: &mut Vec<String>,
    resolve: bool,
) -> Vec<WorkRow> {
    let directory = roots.source_workspace_root.join(".refact/trajectories");
    let directory = match tokio::fs::canonicalize(&directory)
        .await
        .map(|path| dunce::simplified(&path).to_path_buf())
    {
        Ok(directory) if directory.starts_with(&roots.source_workspace_root) => directory,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return vec![],
        _ => {
            degraded.push("historical index unavailable".into());
            return vec![];
        }
    };
    let index = match app
        .gcx
        .trajectory_index_coordinator
        .work_snapshot(&directory)
        .await
    {
        Ok(index) => index,
        Err(_) => {
            degraded.push("historical index unavailable".into());
            return vec![];
        }
    };
    let mut entries: Vec<_> = index
        .entries
        .iter()
        .filter(|entry| entry.displayable_chat)
        .filter(|entry| {
            crate::chat::trajectory_index::index_entry_file_name_is_valid(&entry.file_name)
        })
        .filter(|entry| {
            entry.worktree.as_ref().is_none_or(|meta| {
                roots::project_identity(Path::new(&meta.source_workspace_root)) == roots.project
            })
        })
        .collect();
    entries.sort_by_key(|entry| std::cmp::Reverse(timestamp(&entry.updated_at)));
    let mut rows = Vec::with_capacity(entries.len());
    let mut evidence_bytes = 0usize;
    for (position, entry) in entries.into_iter().enumerate() {
        let row = if resolve && position < MAX_RESOLVED_ROWS {
            resolve_entry(&directory, entry, roots)
                .await
                .unwrap_or_else(|| row_from_entry(entry, roots))
        } else {
            row_from_entry(entry, roots)
        };
        let mut row = row;
        evidence_bytes = evidence_bytes.saturating_add(row.source_records.iter().fold(
            0usize,
            |bytes, record| {
                bytes
                    .saturating_add(record.path.len())
                    .saturating_add(record.zone.len())
                    .saturating_add(std::mem::size_of::<refact_privacy::FileRecord>())
            },
        ));
        if evidence_bytes > CACHE_BYTES {
            row.source_records.clear();
            row.provenance_complete = false;
        }
        rows.push(row);
    }
    rows
}

const MAX_RESOLVED_ROWS: usize = 32;
const MAX_SOURCE_BYTES: u64 = 16 * 1024 * 1024;
const CACHE_BYTES: usize = 16 * 1024 * 1024;
const RACY_WINDOW_NS: i128 = 2_000_000_000;
const MAX_EVIDENCE_AGE_NS: i128 = 600_000_000_000;

trait EvidenceClock: Send + 'static {
    fn now_ns(&self) -> Option<i128>;
    fn racy_window_ns(&self) -> i128 {
        RACY_WINDOW_NS
    }
    fn max_age_ns(&self) -> i128 {
        MAX_EVIDENCE_AGE_NS
    }
    fn stat(&self, metadata: &std::fs::Metadata) -> Option<SourceStat> {
        source_stat(metadata)
    }
}
struct SystemEvidenceClock;
impl EvidenceClock for SystemEvidenceClock {
    fn now_ns(&self) -> Option<i128> {
        Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_nanos() as i128,
        )
    }
}

// Only fully hashed and verified evidence enters this cache. The stat is a
// reuse fence, never standalone evidence. Non-Unix platforms disable reuse.
#[derive(Clone, Debug, PartialEq, Eq)]
struct SourceStat {
    dev: u64,
    ino: u64,
    len: u64,
    mtime_ns: i128,
    ctime_ns: i128,
}
fn source_stat(metadata: &std::fs::Metadata) -> Option<SourceStat> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(SourceStat {
            dev: metadata.dev(),
            ino: metadata.ino(),
            len: metadata.len(),
            mtime_ns: metadata.mtime() as i128 * 1_000_000_000 + metadata.mtime_nsec() as i128,
            ctime_ns: metadata.ctime() as i128 * 1_000_000_000 + metadata.ctime_nsec() as i128,
        })
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        None
    }
}
type EvidenceKey = (std::path::PathBuf, String, [u8; 32]);
type CachedEvidence = (
    TrajectoryIndexEntry,
    WorkIndexMetadata,
    usize,
    Option<SourceStat>,
    i128, // Wall-clock nanoseconds when the fully verified evidence was cached.
);
type EvidenceCache = std::collections::VecDeque<(EvidenceKey, CachedEvidence)>;
static EVIDENCE_CACHE: std::sync::LazyLock<std::sync::Mutex<EvidenceCache>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(Default::default()));
fn evidence_cache() -> std::sync::MutexGuard<'static, EvidenceCache> {
    EVIDENCE_CACHE.lock().unwrap_or_else(|poisoned| {
        tracing::warn!("Recovering poisoned trajectory evidence cache mutex");
        poisoned.into_inner()
    })
}
static RESOLVER_PERMITS: std::sync::LazyLock<std::sync::Arc<tokio::sync::Semaphore>> =
    std::sync::LazyLock::new(|| std::sync::Arc::new(tokio::sync::Semaphore::new(4)));
#[cfg(test)]
static SOURCE_READS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<std::path::PathBuf, usize>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(Default::default()));

async fn resolve_entry(
    directory: &Path,
    entry: &TrajectoryIndexEntry,
    roots: &ProjectRoots,
) -> Option<WorkRow> {
    resolve_entry_with_clock(directory, entry, roots, SystemEvidenceClock).await
}

async fn resolve_entry_with_clock(
    directory: &Path,
    entry: &TrajectoryIndexEntry,
    roots: &ProjectRoots,
    clock: impl EvidenceClock,
) -> Option<WorkRow> {
    let permit = RESOLVER_PERMITS.clone().acquire_owned().await.ok()?;
    let directory = directory.to_path_buf();
    let entry = entry.clone();
    let (fresh, work) = tokio::task::spawn_blocking(move || {
        let _permit = permit; // Bound blocking readers even on caller cancellation.
        use std::io::Read;
        use crate::chat::trajectory_index::{entry_from_owned_trajectory_value, source_sha256};
        if !crate::chat::trajectory_index::index_entry_file_name_is_valid(&entry.file_name) {
            return None;
        }
        if let Some(reference) = &entry.work_ref {
            if reference.version != 1
                || !matches!(
                    reference.state,
                    WorkReferenceState::Unresolved | WorkReferenceState::SourceBound
                )
                || (reference.state == WorkReferenceState::SourceBound
                    && reference.source_sha256.is_none())
            {
                return None;
            }
        }
        let path = directory.join(&entry.file_name);
        if path.file_stem()?.to_str()? != entry.id {
            return None;
        }
        let canonical = super::scope::canonical_candidate(&path).ok()?;
        if !canonical.starts_with(&directory) {
            return None;
        }
        let metadata = std::fs::symlink_metadata(&path).ok()?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() > MAX_SOURCE_BYTES
        {
            return None;
        }
        let stat = clock.stat(&metadata);
        let now_ns = clock.now_ns();
        let bound = entry.work_ref.as_ref().and_then(|r| r.source_sha256);
        let cached = evidence_cache()
            .iter()
            .find(
                |((cached_path, id, hash), (_, _, _, cached_stat, cached_at))| {
                    cached_path == &canonical
                    && id == &entry.id
                    && stat.is_some()
                    && cached_stat == &stat
                    // Identical stats can hide same-tick, same-length writes.
                    // A racy snapshot stays untrusted even after time passes:
                    // only a later re-hash can establish a safe cache time.
                    && stat.as_ref().is_some_and(|stat| {
                        stat.mtime_ns < *cached_at - clock.racy_window_ns()
                            && stat.ctime_ns < *cached_at - clock.racy_window_ns()
                    })
                    && now_ns.is_some_and(|now| {
                        now >= *cached_at && now - *cached_at <= clock.max_age_ns()
                    })
                    && bound.is_none_or(|bound| bound == *hash)
                },
            )
            .map(|(_, evidence)| evidence.clone());
        let (fresh, work) = if let Some((fresh, work, _, _, _)) = cached {
            (fresh, work)
        } else {
            let mut options = std::fs::OpenOptions::new();
            options.read(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
            }
            let mut file = options.open(&path).ok()?;
            let opened = file.metadata().ok()?;
            if !opened.is_file() || clock.stat(&opened) != stat {
                return None;
            }
            let mut bytes = Vec::new();
            #[cfg(test)]
            {
                *SOURCE_READS
                    .lock()
                    .unwrap()
                    .entry(canonical.clone())
                    .or_default() += 1;
            }
            (&mut file)
                .take(MAX_SOURCE_BYTES + 1)
                .read_to_end(&mut bytes)
                .ok()?;
            if bytes.len() as u64 > MAX_SOURCE_BYTES {
                return None;
            }
            let hash = source_sha256(&bytes);
            if bound.is_some_and(|bound| bound != hash) {
                return None;
            }
            let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
            let identity = crate::chat::types::TrajectorySourceIdentity::from_json(&value).ok()?;
            if TrajectoryIndexSource::from(&identity) != entry.source {
                return None;
            }
            let revision = value.get("version").and_then(|v| v.as_u64()).unwrap_or(0);
            let work = WorkIndexMetadata::from_values(value.get("messages")?.as_array()?, revision);
            if !work.provenance_complete {
                return None;
            }
            let fresh =
                entry_from_owned_trajectory_value(&directory, &path, value, None, Some(hash))
                    .ok()?;
            if fresh.id != entry.id || !fresh.displayable_chat {
                return None;
            }
            // Fence both the opened snapshot and current pathname after projection.
            // Changed metadata invalidates the result without a second body read.
            if clock.stat(&file.metadata().ok()?) != stat
                || clock.stat(&std::fs::symlink_metadata(&path).ok()?) != stat
                || super::scope::canonical_candidate(&path).ok()? != canonical
            {
                return None;
            }
            let size = serde_json::to_vec(&(&fresh, &work))
                .ok()?
                .len()
                .saturating_add(
                    work.records
                        .len()
                        .saturating_mul(std::mem::size_of::<refact_privacy::FileRecord>()),
                );
            if let Some(cached_at) = clock.now_ns().filter(|_| size <= CACHE_BYTES) {
                let mut cache = evidence_cache();
                while cache
                    .iter()
                    .map(|(_, (_, _, size, _, _))| *size)
                    .sum::<usize>()
                    + size
                    > CACHE_BYTES
                {
                    cache.pop_front()?;
                }
                let key = (canonical, entry.id.clone(), hash);
                cache.retain(|(old, _)| old != &key);
                cache.push_back((key, (fresh.clone(), work.clone(), size, stat, cached_at)));
            }
            (fresh, work)
        };
        if fresh.source != entry.source || fresh.id != entry.id || !fresh.displayable_chat {
            return None;
        }
        if entry
            .work_ref
            .as_ref()
            .is_some_and(|r| r.source_sha256.is_some() && r.revision != work.source_revision)
        {
            return None;
        }
        Some((fresh, work))
    })
    .await
    .ok()??;
    let mut row = row_from_entry(&fresh, roots);
    if row.root.project != roots.project {
        return None;
    }
    row.source_records = work.records;
    row.provenance_complete = work.provenance_complete;
    row.metadata.source_revision = work.source_revision;
    row.metadata.measured_tokens = work.measured_tokens;
    Some(row)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::coordination::privacy_guard;
    use serde_json::json;
    use std::sync::Arc;

    async fn fixture(
        messages: Vec<serde_json::Value>,
    ) -> (
        tempfile::TempDir,
        std::sync::Arc<crate::global_context::GlobalContext>,
        ProjectRoots,
        TrajectoryIndexEntry,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let roots = roots::load_project_roots(gcx.clone(), dir.path().into()).await;
        let value = json!({"id":"lazy", "title":"Private original", "created_at":"2026-01-01T00:00:00Z", "updated_at":"2026-01-02T00:00:00Z", "model":"trusted/model", "mode":"agent", "tool_use":"agent", "version":7, "messages":messages});
        let path = dir.path().join("lazy.json");
        let bytes = serde_json::to_vec(&value).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let entry = crate::chat::trajectory_index::entry_from_trajectory_value(
            dir.path(),
            &path,
            &value,
            None,
            Some(crate::chat::trajectory_index::source_sha256(&bytes)),
        )
        .unwrap();
        assert!(entry.displayable_chat);
        (dir, gcx, roots, entry)
    }

    #[tokio::test]
    async fn lazy_guarded_evidence_is_exact_and_hash_fenced_even_after_cache_hit() {
        let record = json!({"path":"/guarded/file", "zone":"guarded", "attribution":"observed"});
        let (dir, gcx, roots, entry) = fixture(vec![
            json!({"role":"tool", "content":"private", "privacy":{"files":[record.clone()]}}),
        ])
        .await;
        gcx.privacy_policy_load.write().unwrap().policy = Arc::new(refact_privacy::PrivacyPolicy {
            zones: vec![refact_privacy::Zone {
                name: "guarded".into(),
                patterns: vec!["/guarded/**".into()],
                send_to: vec!["trusted".into()],
                on_shell_read: refact_privacy::ShellBehavior::Withhold,
            }],
            ..Default::default()
        });
        #[derive(Clone)]
        struct FixtureClock {
            now: i128,
            tick: i128,
        }
        impl EvidenceClock for FixtureClock {
            fn now_ns(&self) -> Option<i128> {
                Some(self.now)
            }
            fn stat(&self, metadata: &std::fs::Metadata) -> Option<SourceStat> {
                let mut stat = source_stat(metadata)?;
                // Emulate a filesystem tick deterministically; real in-place IO,
                // device, inode and length are unchanged. No sleep or ctime forging.
                stat.mtime_ns = self.tick;
                stat.ctime_ns = self.tick;
                Some(stat)
            }
        }
        let clock = FixtureClock {
            now: 10_000_000_000,
            tick: 10_000_000_000,
        };
        let row = resolve_entry_with_clock(dir.path(), &entry, &roots, clock.clone())
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&row.source_records).unwrap(),
            json!([record])
        );
        assert_eq!(row.title.as_deref(), Some("Private original"));
        let trusted = crate::caps::BaseModelRecord {
            id: "trusted/model".into(),
            ..Default::default()
        };
        let other = crate::caps::BaseModelRecord {
            id: "other/model".into(),
            ..Default::default()
        };
        assert_eq!(
            privacy_guard::clear_rows(&gcx, vec![row.clone()], &trusted)
                .unwrap()
                .len(),
            1
        );
        assert!(privacy_guard::clear_rows(&gcx, vec![row], &other)
            .unwrap()
            .is_empty());
        let path = dir.path().join("lazy.json");
        let canonical = std::fs::canonicalize(&path).unwrap();
        let original = std::fs::read(&path).unwrap();
        let changed = String::from_utf8(original.clone())
            .unwrap()
            .replace("Private original", "Changed original");
        assert_eq!(changed.len(), original.len());
        assert_ne!(changed.as_bytes(), original);
        let before_stat = clock.stat(&std::fs::metadata(&path).unwrap());
        // An immediate same-length in-place rewrite within the emulated tick
        // has identical stats, but must not reuse the just-cached evidence.
        std::fs::write(&path, changed).unwrap();
        assert_eq!(clock.stat(&std::fs::metadata(&path).unwrap()), before_stat);
        assert!(
            resolve_entry_with_clock(dir.path(), &entry, &roots, clock.clone())
                .await
                .is_none(),
            "changed bytes at identical revision and stat must withhold cached evidence"
        );
        // Restore the same source and advance the injected clock, not the wall
        // clock. The racy entry must first be re-hashed before safe warm reuse.
        std::fs::write(&path, original).unwrap();
        let safe_clock = FixtureClock {
            now: clock.now + RACY_WINDOW_NS + 1,
            ..clock
        };
        assert!(
            resolve_entry_with_clock(dir.path(), &entry, &roots, safe_clock.clone())
                .await
                .is_some()
        );
        let before = SOURCE_READS.lock().unwrap()[&canonical];
        assert!(
            resolve_entry_with_clock(dir.path(), &entry, &roots, safe_clock)
                .await
                .is_some()
        );
        assert_eq!(
            SOURCE_READS.lock().unwrap()[&canonical],
            before,
            "unchanged verified source outside the racy window must reuse evidence without reading or hashing"
        );
    }

    #[tokio::test]
    async fn lazy_verified_empty_is_distinct_from_missing_source() {
        let (dir, _, roots, entry) = fixture(vec![]).await;
        let row = resolve_entry(dir.path(), &entry, &roots).await.unwrap();
        assert!(row.provenance_complete);
        assert!(row.source_records.is_empty());
        std::fs::remove_file(dir.path().join("lazy.json")).unwrap();
        assert!(resolve_entry(dir.path(), &entry, &roots).await.is_none());
    }

    #[tokio::test]
    async fn lazy_malformed_source_never_becomes_verified_empty() {
        let (dir, _, roots, entry) = fixture(vec![json!(false)]).await;
        assert!(resolve_entry(dir.path(), &entry, &roots).await.is_none());
    }

    #[tokio::test]
    async fn lazy_unknown_reference_is_not_clearance() {
        let (dir, _, roots, mut entry) = fixture(vec![]).await;
        let mut wire = serde_json::to_value(&entry).unwrap();
        wire["work_ref"] = json!({"version":999,"state":"future","source_sha256":"changed-shape"});
        entry =
            serde_json::from_value(wire).expect("future reference must not brick index decoding");
        assert!(resolve_entry(dir.path(), &entry, &roots).await.is_none());
    }

    #[tokio::test]
    async fn lazy_unresolved_identity_mismatch_is_withheld() {
        let (dir, _, roots, mut entry) = fixture(vec![]).await;
        entry.work_ref.as_mut().unwrap().source_sha256 = None;
        entry.work_ref.as_mut().unwrap().state = WorkReferenceState::Unresolved;
        entry.id = "other".into();
        assert!(resolve_entry(dir.path(), &entry, &roots).await.is_none());
    }

    #[tokio::test]
    async fn lazy_source_size_limit_is_fail_closed() {
        let (dir, _, roots, entry) = fixture(vec![]).await;
        std::fs::OpenOptions::new()
            .write(true)
            .open(dir.path().join("lazy.json"))
            .unwrap()
            .set_len(MAX_SOURCE_BYTES + 1)
            .unwrap();
        assert!(resolve_entry(dir.path(), &entry, &roots).await.is_none());
    }

    #[test]
    fn one_malformed_index_message_invalidates_all_provenance() {
        let values = vec![
            serde_json::to_value(crate::call_validation::ChatMessage::default()).unwrap(),
            json!(false),
        ];
        let work = WorkIndexMetadata::from_values(&values, 9);
        assert!(!work.provenance_complete);
        assert!(work.records.is_empty());
        assert_eq!(work.measured_tokens, None);
        assert_eq!(work.source_revision, 9);
    }

    #[tokio::test]
    async fn legacy_or_denied_history_never_discloses_to_a_model() {
        let project = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let roots = roots::load_project_roots(gcx.clone(), project.path().into()).await;
        let mut entry: TrajectoryIndexEntry = serde_json::from_value(json!({
            "id":"legacy", "file_name":"legacy.json", "created_at":"2026-01-01T00:00:00Z",
            "updated_at":"2026-01-02T00:00:00Z", "title":"Legacy private title", "model":"trusted/model",
            "mode":"AGENT", "message_count":1
        })).unwrap();
        let model = crate::caps::BaseModelRecord {
            id: "other/model".into(),
            ..Default::default()
        };
        assert!(
            privacy_guard::clear_rows(&gcx, vec![row_from_entry(&entry, &roots)], &model)
                .unwrap()
                .is_empty()
        );
        let mut message = crate::call_validation::ChatMessage::new("tool".into(), "private".into());
        message.extra.insert(
            "privacy".into(),
            json!({"files":[{
                "path":"/guarded/file", "zone":"guarded", "attribution":"declared"
            }]}),
        );
        entry.work = Some(
            serde_json::from_value(
                serde_json::to_value(WorkIndexMetadata::from_messages(&[message], 1)).unwrap(),
            )
            .unwrap(),
        );
        gcx.privacy_policy_load.write().unwrap().policy = Arc::new(refact_privacy::PrivacyPolicy {
            zones: vec![refact_privacy::Zone {
                name: "guarded".into(),
                patterns: vec!["/guarded/**".into()],
                send_to: vec!["trusted".into()],
                on_shell_read: refact_privacy::ShellBehavior::Withhold,
            }],
            ..Default::default()
        });
        assert!(
            privacy_guard::clear_rows(&gcx, vec![row_from_entry(&entry, &roots)], &model)
                .unwrap()
                .is_empty()
        );
    }
}
