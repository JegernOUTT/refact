use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::files_correction::{registered_worktree_path_mappings, WorktreePathMappings};
use crate::global_context::GlobalContext;

const MAX_ENTRIES: usize = 4096;
const MAX_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct PrivacyKey {
    policy: Arc<refact_privacy::PrivacyPolicy>,
    workspaces: Vec<PathBuf>,
    base: PathBuf,
    epoch: u64,
    registry_revision: refact_worktrees::service::RegistryRevision,
}

impl PartialEq for PrivacyKey {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.policy, &other.policy)
            && self.workspaces == other.workspaces
            && self.base == other.base
            && self.epoch == other.epoch
            && self.registry_revision == other.registry_revision
    }
}
impl Eq for PrivacyKey {}

impl PrivacyKey {
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
}

pub fn privacy_key(gcx: &GlobalContext) -> Result<PrivacyKey, String> {
    let load = gcx
        .privacy_policy_load
        .read()
        .map_err(|_| "privacy policy unavailable")?;
    if load.error.is_some() {
        return Err("privacy policy unavailable".into());
    }
    Ok(PrivacyKey {
        policy: load.policy.clone(),
        workspaces: gcx
            .documents_state
            .workspace_folders
            .lock()
            .map_err(|_| "privacy workspace unavailable")?
            .clone(),
        base: std::env::current_dir().map_err(|_| "privacy base unavailable")?,
        epoch: gcx.privacy_classification.epoch.load(Ordering::SeqCst),
        registry_revision: refact_worktrees::service::registry_revision(&gcx.cache_dir),
    })
}

pub fn invalidate(gcx: &GlobalContext) {
    gcx.privacy_classification
        .epoch
        .fetch_add(1, Ordering::SeqCst);
}

#[derive(Default)]
pub struct ClassificationCache {
    epoch: AtomicU64,
    state: Mutex<Option<Arc<Snapshot>>>,
    initialize: Mutex<()>,
    #[cfg(test)]
    pub(crate) compiles: AtomicU64,
    #[cfg(test)]
    pub(crate) classifications: AtomicU64,
}

pub(crate) struct Snapshot {
    pub(crate) key: PrivacyKey,
    pub(crate) compiled: Arc<refact_privacy::CompiledPolicy>,
    fingerprint: String,
    pub(crate) mappings: WorktreePathMappings,
    entries: Mutex<Entries>,
}

#[derive(Clone)]
pub(crate) struct StaticClassification {
    pub(crate) candidates: Vec<PathBuf>,
    pub(crate) zone: refact_privacy::Zone,
}

#[derive(Default)]
struct Entries {
    paths: HashMap<PathBuf, (StaticClassification, u64, usize)>,
    clock: u64,
    bytes: usize,
}

pub(crate) fn snapshot(gcx: &GlobalContext) -> Result<Arc<Snapshot>, String> {
    let key = privacy_key(gcx)?;
    {
        let state = gcx
            .privacy_classification
            .state
            .lock()
            .map_err(|_| "privacy cache unavailable")?;
        if let Some(found) = state.as_ref().filter(|found| found.key == key) {
            return Ok(found.clone());
        }
    }
    let _initialize = gcx
        .privacy_classification
        .initialize
        .lock()
        .map_err(|_| "privacy cache unavailable")?;
    let key = privacy_key(gcx)?;
    {
        let state = gcx
            .privacy_classification
            .state
            .lock()
            .map_err(|_| "privacy cache unavailable")?;
        if let Some(found) = state.as_ref().filter(|found| found.key == key) {
            return Ok(found.clone());
        }
    }
    let previous = gcx
        .privacy_classification
        .state
        .lock()
        .map_err(|_| "privacy cache unavailable")?
        .clone();
    let (compiled, fingerprint) =
        if let Some(previous) = previous.filter(|old| Arc::ptr_eq(&old.key.policy, &key.policy)) {
            (previous.compiled.clone(), previous.fingerprint.clone())
        } else {
            let compiled = Arc::new(
                key.policy
                    .compile()
                    .map_err(|error| format!("privacy policy failed to compile: {error}"))?,
            );
            #[cfg(test)]
            gcx.privacy_classification
                .compiles
                .fetch_add(1, Ordering::SeqCst);
            let policy = serde_json::to_string(key.policy.as_ref())
                .map_err(|_| "privacy policy unavailable")?;
            (compiled, crate::file_activity::digest_of(&policy))
        };
    let found = Arc::new(Snapshot {
        key: key.clone(),
        compiled,
        fingerprint,
        mappings: registered_worktree_path_mappings(gcx.cache_dir.as_path()),
        entries: Mutex::new(Entries::default()),
    });
    if privacy_key(gcx)? != key {
        return Err("privacy context changed".into());
    }
    *gcx.privacy_classification
        .state
        .lock()
        .map_err(|_| "privacy cache unavailable")? = Some(found.clone());
    Ok(found)
}

pub fn policy_fingerprint(gcx: &GlobalContext) -> Result<String, String> {
    Ok(snapshot(gcx)?.fingerprint.clone())
}

impl Snapshot {
    #[cfg(test)]
    fn usage(&self) -> (usize, usize) {
        let entries = self.entries.lock().unwrap();
        (entries.paths.len(), entries.bytes)
    }

    pub(crate) fn finish(&self, gcx: &GlobalContext) -> Result<(), String> {
        if privacy_key(gcx)? != self.key {
            return Err("privacy context changed".into());
        }
        Ok(())
    }

    pub(crate) fn classify(&self, gcx: &Arc<GlobalContext>, path: &Path) -> StaticClassification {
        {
            let mut entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
            entries.clock += 1;
            let clock = entries.clock;
            if let Some((value, stamp, _)) = entries.paths.get_mut(path) {
                *stamp = clock;
                return value.clone();
            }
        }
        #[cfg(test)]
        gcx.privacy_classification
            .classifications
            .fetch_add(1, Ordering::SeqCst);
        let candidates = super::records::record_path_candidates(gcx, path, &self.mappings);
        let zone = self.compiled.strictest_zone_for_paths_with_roots(
            &candidates,
            super::records::privacy_roots(gcx, &self.mappings),
        );
        let value = StaticClassification {
            candidates,
            zone,
        };
        let bytes = path.as_os_str().len()
            + value
                .candidates
                .iter()
                .map(|p| p.as_os_str().len())
                .sum::<usize>()
            + value.zone.name.len()
            + value
                .zone
                .patterns
                .iter()
                .chain(value.zone.send_to.iter())
                .map(String::len)
                .sum::<usize>();
        if bytes <= MAX_BYTES && self.finish(gcx).is_ok() {
            let mut entries = self.entries.lock().unwrap_or_else(|p| p.into_inner());
            if !entries.paths.contains_key(path) {
                while entries.paths.len() >= MAX_ENTRIES || entries.bytes + bytes > MAX_BYTES {
                    let Some(oldest) = entries
                        .paths
                        .iter()
                        .min_by_key(|(_, (_, stamp, _))| stamp)
                        .map(|(path, _)| path.clone())
                    else {
                        break;
                    };
                    if let Some((_, _, size)) = entries.paths.remove(&oldest) {
                        entries.bytes -= size;
                    }
                }
                entries.clock += 1;
                let clock = entries.clock;
                entries.bytes += bytes;
                entries
                    .paths
                    .insert(path.to_path_buf(), (value.clone(), clock, bytes));
            }
        }
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::privacy::records::{declared_file_record, declared_file_records};

    #[tokio::test]
    async fn repeated_operations_share_static_work_and_logical_changes_invalidate() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let path = gcx.cache_dir.join("file.txt");
        std::fs::write(&path, "fixture").unwrap();
        let first = declared_file_record(&gcx, &path).unwrap();
        for _ in 0..100 {
            assert_eq!(declared_file_record(&gcx, &path).unwrap(), first);
            assert_eq!(
                declared_file_records(&gcx, [path.clone()]).unwrap(),
                [first.clone()]
            );
        }
        assert_eq!(
            gcx.privacy_classification.compiles.load(Ordering::SeqCst),
            1
        );
        assert_eq!(
            gcx.privacy_classification
                .classifications
                .load(Ordering::SeqCst),
            1
        );
        let old = snapshot(&gcx).unwrap();
        invalidate(&gcx);
        assert!(old.finish(&gcx).is_err());
        declared_file_record(&gcx, &path).unwrap();
        assert_eq!(
            gcx.privacy_classification.compiles.load(Ordering::SeqCst),
            1
        );
        assert_eq!(
            gcx.privacy_classification
                .classifications
                .load(Ordering::SeqCst),
            2
        );
        gcx.documents_state
            .workspace_folders
            .lock()
            .unwrap()
            .push(gcx.cache_dir.clone());
        declared_file_record(&gcx, &path).unwrap();
        assert_eq!(
            gcx.privacy_classification
                .classifications
                .load(Ordering::SeqCst),
            3
        );
        let policy = gcx.privacy_policy_load.read().unwrap().policy.clone();
        gcx.privacy_policy_load.write().unwrap().policy = Arc::new((*policy).clone());
        declared_file_record(&gcx, &path).unwrap();
        assert_eq!(
            gcx.privacy_classification.compiles.load(Ordering::SeqCst),
            2
        );
        gcx.privacy_policy_load.write().unwrap().error = Some("fixture".into());
        assert!(declared_file_record(&gcx, &path).is_err());
    }

    #[tokio::test]
    async fn registry_publications_only_invalidate_the_relevant_cache_directory() {
        use refact_worktrees::service::WorktreeService;
        use refact_worktrees::types::{WorktreeMeta, WorktreeRegistryRecord};

        let gcx = crate::global_context::tests::make_test_gcx().await;
        let unrelated = crate::global_context::tests::make_test_gcx().await;
        let sources = tempfile::tempdir().unwrap();
        let source = sources.path().join("source");
        let other_source = sources.path().join("other-source");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&other_source).unwrap();
        let service = WorktreeService::new(gcx.cache_dir.clone(), source.clone()).unwrap();
        let other_service =
            WorktreeService::new(unrelated.cache_dir.clone(), other_source.clone()).unwrap();
        let root = service.worktree_path_for_id("mapping").unwrap();
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("secret.txt");
        std::fs::write(&path, "fixture").unwrap();
        let source = service.source_workspace_root().to_path_buf();
        std::fs::write(source.join("secret.txt"), "fixture").unwrap();
        gcx.privacy_policy_load.write().unwrap().policy = Arc::new(refact_privacy::PrivacyPolicy {
            blocked: vec![source.join("secret.txt").to_string_lossy().into_owned()],
            ..Default::default()
        });
        let initial = declared_file_record(&gcx, &path).unwrap();
        assert_ne!(initial.zone, "blocked");
        let old = snapshot(&gcx).unwrap();
        let old_mappings = registered_worktree_path_mappings(&gcx.cache_dir);
        assert!(old_mappings.is_empty());
        let expected = other_service.load_registry().await.unwrap();
        other_service
            .compare_and_save_registry(&expected, &expected)
            .await
            .unwrap();
        old.finish(&gcx).unwrap();
        assert!(Arc::ptr_eq(&old, &snapshot(&gcx).unwrap()));
        assert!(Arc::ptr_eq(
            &old_mappings,
            &registered_worktree_path_mappings(&gcx.cache_dir)
        ));
        assert_eq!(declared_file_record(&gcx, &path).unwrap(), initial);
        assert_eq!(
            gcx.privacy_classification
                .classifications
                .load(Ordering::SeqCst),
            1
        );

        let same_cache_other_project =
            WorktreeService::new(gcx.cache_dir.clone(), other_source).unwrap();
        let expected = same_cache_other_project.load_registry().await.unwrap();
        same_cache_other_project
            .compare_and_save_registry(&expected, &expected)
            .await
            .unwrap();
        assert_eq!(old.finish(&gcx).unwrap_err(), "privacy context changed");
        let old = snapshot(&gcx).unwrap();

        let expected = service.load_registry().await.unwrap();
        let mut registry = expected.clone();
        registry.records.push(WorktreeRegistryRecord {
            creator_chat_id: None,
            meta: WorktreeMeta {
                id: "mapping".into(),
                kind: "chat".into(),
                root: root.clone(),
                source_workspace_root: source.clone(),
                repo_root: source,
                branch: None,
                base_branch: None,
                base_commit: None,
                task_id: None,
                card_id: None,
                agent_id: None,
                enforce: true,
            },
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
            last_seen_at: None,
            references: Vec::new(),
            last_known_status: None,
        });
        service
            .compare_and_save_registry(&expected, &registry)
            .await
            .unwrap();
        assert_eq!(old.finish(&gcx).unwrap_err(), "privacy context changed");
        let fresh = snapshot(&gcx).unwrap();
        assert!(!Arc::ptr_eq(&old, &fresh));
        assert_eq!(fresh.mappings.len(), 1);
        assert_eq!(fresh.mappings[0].root, root);
        assert_eq!(declared_file_record(&gcx, &path).unwrap().zone, "blocked");
        fresh.finish(&gcx).unwrap();
        assert_eq!(
            gcx.privacy_classification
                .classifications
                .load(Ordering::SeqCst),
            2
        );
        assert_eq!(
            gcx.privacy_classification.compiles.load(Ordering::SeqCst),
            1
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn warm_alias_observation_is_intentionally_best_effort() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let public = gcx.cache_dir.join("public.txt");
        let secret = gcx.cache_dir.join("secret.txt");
        let alias = gcx.cache_dir.join("alias.txt");
        std::fs::write(&public, "public").unwrap();
        std::fs::write(&secret, "secret").unwrap();
        std::os::unix::fs::symlink(&public, &alias).unwrap();
        gcx.privacy_policy_load.write().unwrap().policy = Arc::new(refact_privacy::PrivacyPolicy {
            blocked: vec!["secret.txt".into()],
            ..Default::default()
        });
        assert_ne!(declared_file_record(&gcx, &alias).unwrap().zone, "blocked");
        std::fs::remove_file(&alias).unwrap();
        std::os::unix::fs::symlink(&secret, &alias).unwrap();
        assert_ne!(declared_file_record(&gcx, &alias).unwrap().zone, "blocked");
        invalidate(&gcx);
        assert_eq!(declared_file_record(&gcx, &alias).unwrap().zone, "blocked");
    }

    #[tokio::test]
    async fn cache_bounds_entries_and_retained_path_bytes() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let snapshot = snapshot(&gcx).unwrap();
        for index in 0..MAX_ENTRIES + 2 {
            snapshot.classify(&gcx, &gcx.cache_dir.join(format!("{index}.txt")));
        }
        let (entries, bytes) = snapshot.usage();
        assert!(entries <= MAX_ENTRIES);
        assert!(bytes <= MAX_BYTES);
        let huge = PathBuf::from("x".repeat(MAX_BYTES + 1));
        snapshot.classify(&gcx, &huge);
        assert_eq!(snapshot.usage(), (entries, bytes));
    }
}
