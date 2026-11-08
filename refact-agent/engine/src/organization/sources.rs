use std::{
    collections::BTreeMap,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock},
};

use refact_privacy::{FileRecord, PrivacyAuditError, PrivacyAudited};
use serde::{Deserialize, Serialize};
use crate::{
    app_state::AppState,
    caps::BaseModelRecord,
    chat::trajectory_index::{TrajectoryIndexEntry, TrajectoryIndexSource, WorkReferenceState},
    coordination::scope::WorkCaller,
    global_context::SharedGlobalContext,
};
use super::types::{Error, Item, ItemKind};

const MAX_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_RECORD_BYTES: usize = 64 * 1024;
static READERS: LazyLock<Arc<tokio::sync::Semaphore>> =
    LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(4)));

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Source {
    pub item: Item,
    pub path: PathBuf,
    pub entry: Option<TrajectoryIndexEntry>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Candidate {
    pub item: Item,
    pub fingerprint: String,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub user_excerpts: Vec<String>,
    pub partial: bool,
    pub source_records: Vec<FileRecord>,
}

struct Audit(Vec<FileRecord>);
impl PrivacyAudited for Audit {
    fn privacy_records(&self) -> Result<Vec<(usize, FileRecord)>, PrivacyAuditError> {
        Ok(self.0.iter().cloned().enumerate().collect())
    }
}

pub(crate) fn canonical_item(entry: &TrajectoryIndexEntry) -> Option<Item> {
    match &entry.source {
        TrajectoryIndexSource::Task {
            task_id, ..
        } => Some(Item {
            kind: ItemKind::Task,
            id: task_id.clone(),
        }),
        TrajectoryIndexSource::Normal => {
            if entry.parent_id.is_some() {
                return None;
            }
            let id = entry
                .root_chat_id
                .as_ref()
                .filter(|id| !id.is_empty())
                .unwrap_or(&entry.id);
            if id != &entry.id {
                return None;
            }
            Some(Item {
                kind: ItemKind::Chat,
                id: id.clone(),
            })
        }
        _ => None,
    }
}

pub(crate) async fn catalog(app: &AppState, caller: &WorkCaller) -> Result<Vec<Source>, Error> {
    let root = crate::coordination::scope::canonical_candidate(&caller.source_root)
        .map_err(|_| Error::denied())?;
    let directory = root.join(".refact/trajectories");
    let mut sources = BTreeMap::new();
    if tokio::fs::try_exists(&directory)
        .await
        .map_err(Error::storage)?
    {
        let canonical = tokio::fs::canonicalize(&directory)
            .await
            .map_err(Error::storage)?;
        if !canonical.starts_with(&root) {
            return Err(Error::denied());
        }
        let index = app
            .gcx
            .trajectory_index_coordinator
            .work_snapshot(&canonical)
            .await
            .map_err(Error::storage)?;
        for entry in &index.entries {
            if !entry.displayable_chat
                || !crate::chat::trajectory_index::index_entry_file_name_is_valid(&entry.file_name)
            {
                continue;
            }
            if entry.worktree.as_ref().is_some_and(|w| {
                crate::coordination::roots::project_identity(Path::new(&w.source_workspace_root))
                    != crate::coordination::roots::project_identity(&root)
            }) {
                continue;
            }
            let Some(item) = canonical_item(entry) else {
                continue;
            };
            if item.kind == ItemKind::Task {
                continue;
            }
            sources.insert(
                item.key(),
                Source {
                    item,
                    path: canonical.join(&entry.file_name),
                    entry: Some(entry.clone()),
                },
            );
        }
    }
    let tasks = root.join(".refact/tasks");
    if tokio::fs::try_exists(&tasks)
        .await
        .map_err(Error::storage)?
    {
        let canonical = tokio::fs::canonicalize(&tasks)
            .await
            .map_err(Error::storage)?;
        if !canonical.starts_with(&root) {
            return Err(Error::denied());
        }
    }
    if let Ok(mut entries) = tokio::fs::read_dir(&tasks).await {
        while let Some(entry) = entries.next_entry().await.map_err(Error::storage)? {
            if !entry.file_type().await.map_err(Error::storage)?.is_dir() {
                continue;
            }
            let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if crate::tasks::storage::validate_task_id(&id).is_err() {
                continue;
            }
            let item = Item {
                kind: ItemKind::Task,
                id,
            };
            sources.insert(
                item.key(),
                Source {
                    item,
                    path: entry.path().join("meta.yaml"),
                    entry: None,
                },
            );
        }
    }
    Ok(sources.into_values().collect())
}

fn stat_equal(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        (
            a.dev(),
            a.ino(),
            a.len(),
            a.mtime(),
            a.mtime_nsec(),
            a.ctime(),
            a.ctime_nsec(),
        ) == (
            b.dev(),
            b.ino(),
            b.len(),
            b.mtime(),
            b.mtime_nsec(),
            b.ctime(),
            b.ctime_nsec(),
        )
    }
    #[cfg(not(unix))]
    {
        a.len() == b.len()
            && a.modified().ok() == b.modified().ok()
            && a.created().ok() == b.created().ok()
    }
}

pub(crate) struct SourceStamp {
    canonical: PathBuf,
    metadata: std::fs::Metadata,
}

pub(crate) fn source_stamp(source: &Source) -> Result<SourceStamp, Error> {
    let canonical = crate::coordination::scope::canonical_candidate(&source.path)
        .map_err(|_| Error::denied())?;
    let metadata = std::fs::symlink_metadata(&source.path).map_err(|_| Error::denied())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(Error::denied());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(Error::denied());
        }
    }
    Ok(SourceStamp {
        canonical,
        metadata,
    })
}

pub(crate) fn source_unchanged(source: &Source, stamp: &SourceStamp) -> bool {
    source_stamp(source).is_ok_and(|fresh| {
        fresh.canonical == stamp.canonical && stat_equal(&fresh.metadata, &stamp.metadata)
    })
}

fn read_source(root: &Path, source: &Source) -> Result<(serde_json::Value, String), Error> {
    let canonical = crate::coordination::scope::canonical_candidate(&source.path)
        .map_err(|_| Error::denied())?;
    if !canonical.starts_with(root) {
        return Err(Error::denied());
    }
    let before = std::fs::symlink_metadata(&source.path).map_err(|_| Error::denied())?;
    if !before.is_file() || before.file_type().is_symlink() || before.len() > MAX_SOURCE_BYTES {
        return Err(Error::denied());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.nlink() != 1 {
            return Err(Error::denied());
        }
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let mut file = options.open(&source.path).map_err(|_| Error::denied())?;
    if !stat_equal(&before, &file.metadata().map_err(|_| Error::denied())?) {
        return Err(Error::denied());
    }
    let (value, hash) = if let Some(entry) = &source.entry {
        if source.path.file_stem().and_then(|s| s.to_str()) != Some(entry.id.as_str()) {
            return Err(Error::denied());
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
                return Err(Error::denied());
            }
        }
        let mut reader = HashReader {
            inner: (&mut file).take(MAX_SOURCE_BYTES + 1),
            hash: sha2::Sha256::default(),
            bytes: 0,
        };
        let mut de = serde_json::Deserializer::from_reader(std::io::BufReader::new(&mut reader));
        let value = serde::de::DeserializeSeed::deserialize(ProjectionSeed, &mut de)
            .map_err(|_| Error::denied())?;
        de.end().map_err(|_| Error::denied())?;
        drop(de);
        if reader.bytes > MAX_SOURCE_BYTES {
            return Err(Error::denied());
        }
        use sha2::Digest;
        let hash: [u8; 32] = reader.hash.finalize().into();
        if value.get("parent_id").and_then(|v| v.as_str()) != entry.parent_id.as_deref()
            || value.get("root_chat_id").and_then(|v| v.as_str()) != entry.root_chat_id.as_deref()
            || value
                .get("link_type")
                .and_then(|v| v.as_str())
                .is_some_and(|s| s.starts_with("internal:"))
        {
            return Err(Error::denied());
        }
        let identity = crate::chat::types::TrajectorySourceIdentity::from_json(&value)
            .map_err(|_| Error::denied())?;
        if value.get("id").and_then(|v| v.as_str()) != Some(entry.id.as_str())
            || TrajectoryIndexSource::from(&identity) != entry.source
            || canonical_item(entry).as_ref() != Some(&source.item)
        {
            return Err(Error::denied());
        }
        (value, hash)
    } else {
        let mut bytes = Vec::new();
        (&mut file)
            .take(2 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| Error::denied())?;
        if bytes.len() > 2 * 1024 * 1024 {
            return Err(Error::denied());
        }
        let value: serde_json::Value =
            serde_yaml::from_slice(&bytes).map_err(|_| Error::denied())?;
        if value.get("id").and_then(|v| v.as_str()) != Some(source.item.id.as_str()) {
            return Err(Error::denied());
        }
        (value, crate::chat::trajectory_index::source_sha256(&bytes))
    };
    let after = std::fs::symlink_metadata(&source.path).map_err(|_| Error::denied())?;
    if !stat_equal(&before, &file.metadata().map_err(|_| Error::denied())?)
        || !stat_equal(&before, &after)
        || crate::coordination::scope::canonical_candidate(&source.path)
            .map_err(|_| Error::denied())?
            != canonical
    {
        return Err(Error::denied());
    }
    Ok((value, hash.iter().map(|b| format!("{b:02x}")).collect()))
}

pub(crate) fn project(
    gcx: &SharedGlobalContext,
    root: &Path,
    source: &Source,
    model: &BaseModelRecord,
) -> Result<Candidate, Error> {
    let policy =
        crate::coordination::privacy_guard::policy_key(gcx, model).map_err(|_| Error::denied())?;
    let (value, fingerprint) = read_source(root, source)?;
    let mut records = crate::privacy::records::declared_file_records(gcx, [source.path.clone()])
        .map_err(|_| Error::denied())?;
    let mut excerpts = Vec::new();
    if source.entry.is_some() {
        let messages = value
            .get("messages")
            .and_then(|v| v.as_array())
            .ok_or_else(Error::denied)?;
        let work = crate::coordination::history::WorkIndexMetadata::from_values(
            messages,
            value.get("version").and_then(|v| v.as_u64()).unwrap_or(0),
        );
        if !work.provenance_complete {
            return Err(Error::denied());
        }
        records.extend(work.records);
        if let Some(all_records) = value.get("organization_records") {
            records.extend(
                serde_json::from_value::<Vec<FileRecord>>(all_records.clone())
                    .map_err(|_| Error::denied())?,
            );
        }
        for message in messages
            .iter()
            .filter(|m| m.get("role").and_then(|v| v.as_str()) == Some("user"))
            .take(3)
        {
            if let Some(text) = message.get("content").and_then(|v| v.as_str()) {
                excerpts.push(text.chars().take(512).collect());
            }
        }
    }
    // Reclassify recorded paths under the current policy as well as preserving
    // their original labels: policy edits may only narrow this disclosure.
    let paths = records
        .iter()
        .map(|r| {
            let p = PathBuf::from(&r.path);
            if p.is_absolute() {
                p
            } else {
                root.join(p)
            }
        })
        .collect::<Vec<_>>();
    records.extend(
        crate::privacy::records::declared_file_records(gcx, paths).map_err(|_| Error::denied())?,
    );
    let mut seen = std::collections::HashSet::new();
    records.retain(|r| seen.insert(r.clone()));
    if serde_json::to_vec(&records)?.len() > MAX_RECORD_BYTES {
        return Err(Error::denied());
    }
    crate::privacy::destinations::clear_for_model(gcx, Audit(records.clone()), model)
        .map_err(|_| Error::denied())?;
    if crate::coordination::privacy_guard::policy_key(gcx, model).map_err(|_| Error::denied())?
        != policy
    {
        return Err(Error::denied());
    }
    Ok(Candidate {
        item: source.item.clone(),
        fingerprint,
        title: value
            .get("title")
            .or_else(|| value.get("name"))
            .and_then(|v| v.as_str())
            .map(|s| s.chars().take(256).collect()),
        summary: None,
        user_excerpts: excerpts,
        partial: true,
        source_records: records,
    })
}

pub(crate) async fn resolve(
    gcx: SharedGlobalContext,
    root: PathBuf,
    source: Source,
    model: BaseModelRecord,
) -> Result<Candidate, Error> {
    let permit = READERS
        .clone()
        .acquire_owned()
        .await
        .map_err(Error::storage)?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        project(&gcx, &root, &source, &model)
    })
    .await
    .map_err(Error::storage)?
}

pub(crate) fn fingerprint_matches(root: &Path, source: &Source, expected: &str) -> bool {
    read_source(root, source).is_ok_and(|(_, fingerprint)| fingerprint == expected)
}

struct HashReader<R> {
    inner: R,
    hash: sha2::Sha256,
    bytes: u64,
}
impl<R: Read> Read for HashReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buffer)?;
        use sha2::Digest;
        self.hash.update(&buffer[..n]);
        self.bytes += n as u64;
        Ok(n)
    }
}
struct ProjectionSeed;
impl<'de> serde::de::DeserializeSeed<'de> for ProjectionSeed {
    type Value = serde_json::Value;
    fn deserialize<D: serde::Deserializer<'de>>(self, de: D) -> Result<Self::Value, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = serde_json::Value;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(f, "trajectory object")
            }
            fn visit_map<M: serde::de::MapAccess<'de>>(
                self,
                mut map: M,
            ) -> Result<Self::Value, M::Error> {
                let mut out = serde_json::Map::new();
                let mut records = Vec::new();
                while let Some(key) = map.next_key::<String>()? {
                    if key == "messages" {
                        let projection = map.next_value_seed(MessagesSeed)?;
                        out.insert(key, serde_json::Value::Array(projection.0));
                        records = projection.1;
                    } else if matches!(
                        key.as_str(),
                        "id" | "title"
                            | "task_meta"
                            | "buddy_meta"
                            | "root_chat_id"
                            | "parent_id"
                            | "link_type"
                            | "version"
                    ) {
                        out.insert(key, map.next_value()?);
                    } else {
                        map.next_value::<serde::de::IgnoredAny>()?;
                    }
                }
                out.insert(
                    "organization_records".into(),
                    serde_json::to_value(records).map_err(serde::de::Error::custom)?,
                );
                Ok(serde_json::Value::Object(out))
            }
        }
        de.deserialize_map(Visitor)
    }
}
struct MessagesSeed;
impl<'de> serde::de::DeserializeSeed<'de> for MessagesSeed {
    type Value = (Vec<serde_json::Value>, Vec<FileRecord>);
    fn deserialize<D: serde::Deserializer<'de>>(self, de: D) -> Result<Self::Value, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = (Vec<serde_json::Value>, Vec<FileRecord>);
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                write!(f, "messages array")
            }
            fn visit_seq<S: serde::de::SeqAccess<'de>>(
                self,
                mut seq: S,
            ) -> Result<Self::Value, S::Error> {
                let settings = crate::file_activity::settings();
                let mut provenance = crate::coordination::provenance::WorkProvenance::default();
                let mut count = 0;
                let mut excerpts = Vec::new();
                while let Some(message) =
                    seq.next_element::<crate::call_validation::ChatMessage>()?
                {
                    count += 1;
                    provenance.observe(&message, count, &settings);
                    if message.role == "user" && excerpts.len() < 3 {
                        let text: String = message
                            .content
                            .content_text_only()
                            .chars()
                            .take(512)
                            .collect();
                        excerpts.push(serde_json::json!({"role":"user","content":text}));
                    }
                }
                let (records, complete) = provenance.snapshot(count, &settings);
                if !complete {
                    return Err(serde::de::Error::custom("incomplete source provenance"));
                }
                Ok((excerpts, records))
            }
        }
        de.deserialize_seq(Visitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[cfg(unix)]
    #[tokio::test]
    async fn hardlinked_task_source_cannot_disclose_an_external_alias() {
        let root = tempfile::tempdir().unwrap();
        let private = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let external = private.path().join("meta.yaml");
        std::fs::write(&external, "id: alias-task\nname: Confidential topic\n").unwrap();
        gcx.privacy_policy_load.write().unwrap().policy = Arc::new(refact_privacy::PrivacyPolicy {
            blocked: vec![external.to_string_lossy().into_owned()],
            ..Default::default()
        });
        let directory = root.path().join(".refact/tasks/alias-task");
        std::fs::create_dir_all(&directory).unwrap();
        let alias = directory.join("meta.yaml");
        std::fs::hard_link(&external, &alias).unwrap();
        let source = Source {
            item: Item {
                kind: ItemKind::Task,
                id: "alias-task".into(),
            },
            path: alias.clone(),
            entry: None,
        };
        let model = BaseModelRecord {
            id: "trusted/model".into(),
            ..Default::default()
        };
        assert!(project(&gcx, root.path(), &source, &model).is_err());
        std::fs::remove_file(&alias).unwrap();
        std::fs::write(&alias, "id: alias-task\nname: Public topic\n").unwrap();
        let visible = project(&gcx, root.path(), &source, &model).unwrap();
        assert_eq!(visible.title.as_deref(), Some("Public topic"));
    }
    #[tokio::test]
    async fn streaming_large_source_carries_late_provenance_and_rejects_changed_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let path = dir.path().join("synthetic.json");
        let value = json!({"id":"synthetic","title":"Private fixture title","created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-02T00:00:00Z","model":"trusted/model","mode":"agent","tool_use":"agent","version":1,"messages":[{"role":"user","content":"Build a fixture parser"},{"role":"assistant","content":"x".repeat(3*1024*1024)},{"role":"tool","content":"late guarded result","privacy":{"files":[{"path":"/synthetic/guarded","zone":"guarded","attribution":"declared"}]}}]});
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
        let source = Source {
            item: Item {
                kind: ItemKind::Chat,
                id: "synthetic".into(),
            },
            path: path.clone(),
            entry: Some(entry),
        };
        gcx.privacy_policy_load.write().unwrap().policy = Arc::new(refact_privacy::PrivacyPolicy {
            zones: vec![refact_privacy::Zone {
                name: "guarded".into(),
                patterns: vec!["/synthetic/guarded".into()],
                send_to: vec!["trusted".into()],
                on_shell_read: refact_privacy::ShellBehavior::Withhold,
            }],
            ..Default::default()
        });
        let model = BaseModelRecord {
            id: "trusted/model".into(),
            ..Default::default()
        };
        let candidate = resolve(gcx.clone(), dir.path().into(), source.clone(), model)
            .await
            .unwrap();
        assert_eq!(candidate.user_excerpts, vec!["Build a fixture parser"]);
        assert_eq!(candidate.summary, None);
        assert!(candidate.partial);
        assert!(candidate
            .source_records
            .iter()
            .any(|r| r.path == "/synthetic/guarded"));
        assert_eq!(
            candidate.fingerprint,
            crate::chat::trajectory_index::source_sha256(&bytes)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        assert!(
            resolve(
                gcx,
                dir.path().into(),
                source.clone(),
                BaseModelRecord {
                    id: "other/model".into(),
                    ..Default::default()
                }
            )
            .await
            .is_err(),
            "late provenance must gate title and excerpt together"
        );
        std::fs::write(
            &path,
            serde_json::to_vec(&json!({"id":"synthetic","title":"replacement","messages":[]}))
                .unwrap(),
        )
        .unwrap();
        assert!(!fingerprint_matches(
            dir.path(),
            &source,
            &candidate.fingerprint
        ));
    }
}

pub async fn authorize(
    app: &AppState,
    caller: &WorkCaller,
    items: &[Item],
) -> Result<Vec<FileRecord>, Error> {
    if items.len() > 200 {
        return Err(Error::invalid("at most 200 organization items"));
    }
    let catalog = catalog(app, caller).await?;
    let by_key: BTreeMap<_, _> = catalog.into_iter().map(|s| (s.item.key(), s)).collect();
    let mut records = Vec::new();
    let policy = caller
        .model
        .as_ref()
        .map(|m| {
            crate::coordination::privacy_guard::policy_key(&app.gcx, m).map_err(|_| Error::denied())
        })
        .transpose()?;
    for item in items {
        let source = by_key
            .get(&item.key())
            .filter(|s| s.item == *item)
            .ok_or_else(Error::denied)?;
        if let Some(model) = &caller.model {
            records.extend(
                resolve(
                    app.gcx.clone(),
                    caller.source_root.clone(),
                    source.clone(),
                    model.clone(),
                )
                .await?
                .source_records,
            );
        } else {
            // Local operators receive no source text through this helper. The
            // complete catalog establishes identity without reading transcripts.
            records.extend(
                crate::privacy::records::declared_file_records(&app.gcx, [source.path.clone()])
                    .map_err(|_| Error::denied())?,
            );
        }
    }
    if let Some(model) = &caller.model {
        if Some(
            crate::coordination::privacy_guard::policy_key(&app.gcx, model)
                .map_err(|_| Error::denied())?,
        ) != policy
        {
            return Err(Error::denied());
        }
    }
    let mut seen = std::collections::HashSet::new();
    records.retain(|r| seen.insert(r.clone()));
    Ok(records)
}
