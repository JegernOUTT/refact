use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};

use tokio::sync::{Mutex as AMutex, MutexGuard};

use crate::chat::types::ChatSession;
use crate::global_context::GlobalContext;

async fn canonical_dir(path: &Path) -> Option<PathBuf> {
    let canonical = tokio::fs::canonicalize(path).await.ok()?;
    tokio::fs::metadata(&canonical)
        .await
        .is_ok_and(|metadata| metadata.is_dir())
        .then(|| dunce::simplified(&canonical).to_path_buf())
}

pub(crate) struct BuddyRootFilter {
    root: Option<PathBuf>,
    rootless_owned: bool,
    sources: StdMutex<HashMap<PathBuf, bool>>,
}

pub(crate) enum SessionClaim<'a> {
    Own(MutexGuard<'a, ChatSession>),
    NotOwn,
    Busy,
}

impl BuddyRootFilter {
    pub(crate) async fn new(gcx: Arc<GlobalContext>, root: &Path) -> Self {
        let root = canonical_dir(root).await;
        let workspace = crate::files_correction::get_unscoped_project_dirs(gcx).await;
        let rootless_owned = match (workspace.as_slice(), root.as_ref()) {
            ([only], Some(root)) => canonical_dir(only).await.as_ref() == Some(root),
            _ => false,
        };
        Self {
            root,
            rootless_owned,
            sources: StdMutex::new(HashMap::new()),
        }
    }

    pub(crate) async fn attributes(&self, source: Option<&Path>) -> bool {
        let Some(source) = source else {
            return self.rootless_owned;
        };
        let Some(root) = self.root.as_ref() else {
            return false;
        };
        if let Some(known) = self
            .sources
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(source)
        {
            return *known;
        }
        let owned = canonical_dir(source).await.as_ref() == Some(root);
        self.sources
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(source.to_path_buf(), owned);
        owned
    }

    pub(crate) async fn attributes_session(&self, session: &ChatSession) -> bool {
        self.attributes(crate::coordination::scope::session_source_root(session).as_deref())
            .await
    }

    pub(crate) async fn attributes_hint(
        &self,
        sessions: &crate::chat::SessionsMap,
        chat_id: &str,
    ) -> Option<bool> {
        let hint = sessions.1.source_root_hint(chat_id)?;
        Some(self.attributes(hint.as_deref()).await)
    }

    pub(crate) async fn claim<'a>(
        &self,
        sessions: &crate::chat::SessionsMap,
        chat_id: &str,
        session: &'a Arc<AMutex<ChatSession>>,
        wait: std::time::Duration,
    ) -> SessionClaim<'a> {
        let guard = match session.try_lock() {
            Ok(guard) => guard,
            Err(_) => {
                if self.attributes_hint(sessions, chat_id).await == Some(false) {
                    return SessionClaim::NotOwn;
                }
                if wait.is_zero() {
                    return SessionClaim::Busy;
                }
                match tokio::time::timeout(wait, session.lock()).await {
                    Ok(guard) => guard,
                    Err(_) => return SessionClaim::Busy,
                }
            }
        };
        if self.attributes_session(&guard).await {
            SessionClaim::Own(guard)
        } else {
            SessionClaim::NotOwn
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum KnownSource {
    Root(PathBuf),
    Rootless,
    Unknown,
}

pub(crate) async fn known_source(
    sessions: &crate::chat::SessionsMap,
    chat_id: &str,
) -> KnownSource {
    let session = sessions.read().await.get(chat_id).cloned();
    let observed = match session.as_ref().map(|session| session.try_lock()) {
        Some(Ok(session)) => Some(crate::coordination::scope::session_source_root(&session)),
        _ => sessions.1.source_root_hint(chat_id),
    };
    match observed {
        Some(Some(root)) => KnownSource::Root(root),
        Some(None) => KnownSource::Rootless,
        None => KnownSource::Unknown,
    }
}

const SINK_SOURCE_CACHE: usize = 256;

#[derive(Default)]
struct SinkCache {
    key: Option<(PathBuf, Vec<PathBuf>)>,
    root: Option<PathBuf>,
    rootless_owned: bool,
    sources: HashMap<PathBuf, Option<PathBuf>>,
}

#[derive(Default)]
pub(crate) struct SinkAttribution(AMutex<SinkCache>);

impl SinkAttribution {
    pub(crate) async fn admits(
        &self,
        gcx: &GlobalContext,
        buddy_root: &Path,
        chat_id: Option<&str>,
    ) -> bool {
        let Some(chat_id) = chat_id else {
            return true;
        };
        let source = known_source(&gcx.chat_sessions, chat_id).await;
        let folders = gcx
            .documents_state
            .workspace_folders
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let mut cache = self.0.lock().await;
        let key = (buddy_root.to_path_buf(), folders);
        if cache.key.as_ref() != Some(&key) {
            cache.root = canonical_dir(buddy_root).await;
            cache.rootless_owned = match (key.1.as_slice(), cache.root.as_ref()) {
                ([only], Some(root)) => canonical_dir(only).await.as_ref() == Some(root),
                _ => false,
            };
            cache.sources.clear();
            cache.key = Some(key);
        }
        match source {
            KnownSource::Unknown => false,
            KnownSource::Rootless => cache.rootless_owned,
            KnownSource::Root(source) => {
                let Some(root) = cache.root.clone() else {
                    return false;
                };
                let canonical = match cache.sources.get(&source) {
                    Some(canonical) => canonical.clone(),
                    None => {
                        let canonical = canonical_dir(&source).await;
                        if cache.sources.len() >= SINK_SOURCE_CACHE {
                            cache.sources.clear();
                        }
                        cache.sources.insert(source, canonical.clone());
                        canonical
                    }
                };
                canonical == Some(root)
            }
        }
    }
}
