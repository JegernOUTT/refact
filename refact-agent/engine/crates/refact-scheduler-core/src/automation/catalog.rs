//! Capability-rooted storage. User supplied paths never become storage paths.
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use super::types::*;

pub(crate) fn root_dir(root: &Path) -> Result<Dir, String> {
    let absolute = if root.is_absolute() {
        root.to_path_buf()
    } else {
        std::env::current_dir().map_err(err)?.join(root)
    };
    if !absolute.is_absolute() {
        return Err("project root must resolve to an absolute path".into());
    }
    #[cfg(not(windows))]
    let anchor = PathBuf::from("/");
    #[cfg(windows)]
    let anchor = {
        use std::path::Prefix;
        match absolute.components().next() {
            Some(Component::Prefix(prefix))
                if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)) =>
            {
                PathBuf::from(prefix.as_os_str()).join("\\")
            }
            _ => return Err("project root requires a local disk path".into()),
        }
    };
    let mut dir = Dir::open_ambient_dir(anchor, cap_std::ambient_authority()).map_err(err)?;
    for component in absolute.components() {
        match component {
            #[cfg(windows)]
            Component::Prefix(_) => {}
            Component::RootDir | Component::CurDir => {}
            Component::Normal(name) => dir = dir.open_dir_nofollow(name).map_err(err)?,
            _ => return Err("unsafe project root component".into()),
        }
        #[cfg(windows)]
        {
            use cap_std::fs::MetadataExt;
            if dir.dir_metadata().map_err(err)?.file_attributes() & 0x400 != 0 {
                return Err("reparse project root component refused".into());
            }
        }
    }
    Ok(dir)
}
pub(crate) fn child(dir: &Dir, name: &str, create: bool) -> Result<Dir, String> {
    if create {
        match dir.create_dir(name) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(err(e)),
        }
    }
    dir.open_dir_nofollow(name).map_err(err)
}
pub(crate) fn regular(
    dir: &Dir,
    name: &str,
    create: bool,
    write: bool,
) -> Result<cap_std::fs::File, String> {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(write)
        .create(create)
        .follow(FollowSymlinks::No);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = dir.open_with(name, &options).map_err(err)?;
    let meta = file.metadata().map_err(err)?;
    if !meta.is_file() {
        return Err("storage must be regular file".into());
    }
    #[cfg(unix)]
    {
        use cap_std::fs::MetadataExt;
        if meta.nlink() != 1 {
            return Err("hard-linked storage refused".into());
        }
    }
    #[cfg(windows)]
    {
        use cap_primitives::fs::_WindowsByHandle;
        if meta.number_of_links() != Some(1) || meta.file_attributes() & 0x400 != 0 {
            return Err("reparse or hard-linked storage refused".into());
        }
    }
    Ok(file)
}
pub(crate) fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

pub struct ProjectAutomationLease {
    pub(crate) root: PathBuf,
    _file: std::fs::File,
    owner_pid: u32,
    epoch: String,
    capacities: Mutex<HashMap<String, Arc<tokio::sync::Semaphore>>>,
}
impl Drop for ProjectAutomationLease {
    fn drop(&mut self) {
        if self.owner_pid == std::process::id() {
            let _ = self._file.unlock();
        }
    }
}
impl ProjectAutomationLease {
    pub fn acquire(root: impl AsRef<Path>) -> Result<Arc<Self>, String> {
        static LEASES: OnceLock<Mutex<HashMap<PathBuf, Weak<ProjectAutomationLease>>>> =
            OnceLock::new();
        let root = root.as_ref();
        // Validate before canonicalization: canonicalization alone hides symlink components.
        let _ = root_dir(root)?;
        let key = std::fs::canonicalize(root).map_err(err)?;
        let mut leases = LEASES.get_or_init(Default::default).lock().map_err(err)?;
        leases.retain(|_, lease| lease.strong_count() > 0);
        if let Some(lease) = leases.get(&key).and_then(Weak::upgrade) {
            return Ok(lease);
        }
        let lease = Self::acquire_exclusive(&key)?;
        leases.insert(key, Arc::downgrade(&lease));
        Ok(lease)
    }
    pub fn acquire_exclusive(root: impl AsRef<Path>) -> Result<Arc<Self>, String> {
        let canonical_root = std::fs::canonicalize(root.as_ref()).map_err(err)?;
        let _ = root_dir(root.as_ref())?;
        let path = super::journal::journal_path(root.as_ref())?;
        let directory = super::journal::private_runtime_directory(
            path.parent().ok_or("runtime directory")?,
            true,
        )?;
        // Private lock inode cannot be replaced by a project-tree writer.
        let file = regular(&directory, "owner.lock", true, true)?.into_std();
        file.try_lock()
            .map_err(|e| format!("automation execution owner already active: {e}"))?;
        Ok(Arc::new(Self {
            root: canonical_root,
            _file: file,
            owner_pid: std::process::id(),
            epoch: uuid::Uuid::new_v4().to_string(),
            capacities: Mutex::new(HashMap::new()),
        }))
    }
    pub fn epoch(&self) -> &str {
        &self.epoch
    }
    pub fn capacity(&self, lane: &str, max: usize) -> Arc<tokio::sync::Semaphore> {
        let mut capacities = self.capacities.lock().unwrap_or_else(|p| p.into_inner());
        capacities
            .entry(lane.into())
            .or_insert_with(|| Arc::new(tokio::sync::Semaphore::new(max.clamp(1, 256))))
            .clone()
    }
}

pub struct AutomationCatalog {
    dir: Dir,
}
impl AutomationCatalog {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, String> {
        Ok(Self {
            dir: child(&root_dir(root.as_ref())?, ".refact", true)?,
        })
    }
    pub(crate) fn from_directory(dir: Dir) -> Self {
        Self {
            dir,
        }
    }
    pub fn read(&self) -> Result<CatalogDocument, String> {
        match self.dir.symlink_metadata("automations.json") {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(CatalogDocument::default())
            }
            Err(e) => return Err(err(e)),
            Ok(_) => {}
        }
        let file = regular(&self.dir, "automations.json", false, false)?;
        if file.metadata().map_err(err)?.len() > 4 * 1024 * 1024 {
            return Err("catalog size limit".into());
        }
        let mut bytes = Vec::new();
        file.take(4 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(err)?;
        let doc: CatalogDocument = serde_json::from_slice(&bytes).map_err(err)?;
        validate_document(&doc)?;
        Ok(doc)
    }
    /// Atomic file CAS. SQLite reconciliation is deliberately a separate idempotent step.
    pub fn compare_and_swap(
        &self,
        expected_revision: u64,
        definitions: Vec<Definition>,
    ) -> Result<CatalogDocument, String> {
        self.compare_and_swap_document(expected_revision, definitions, None)
    }
    pub fn compare_and_swap_legacy_jobs(
        &self,
        expected_revision: u64,
        jobs: Vec<crate::types::Job>,
    ) -> Result<CatalogDocument, String> {
        self.compare_and_swap_document(expected_revision, Vec::new(), Some(jobs))
    }
    fn compare_and_swap_document(
        &self,
        expected_revision: u64,
        definitions: Vec<Definition>,
        jobs: Option<Vec<crate::types::Job>>,
    ) -> Result<CatalogDocument, String> {
        let lock = regular(&self.dir, ".automations.catalog.lock", true, true)?.into_std();
        lock.lock().map_err(err)?;
        let old = self.read()?;
        if old.revision != expected_revision {
            return Err("catalog revision conflict".into());
        }
        let (definitions, legacy_jobs) = if let Some(jobs) = jobs {
            (old.definitions.clone(), jobs)
        } else {
            (definitions, old.legacy_jobs.clone())
        };
        let next = CatalogDocument {
            revision: old.revision.checked_add(1).ok_or("revision overflow")?,
            definitions,
            legacy_jobs,
        };
        validate_document(&next)?;
        for def in &next.definitions {
            if let Some(previous) = old.definitions.iter().find(|d| d.id == def.id) {
                let before = serde_json::to_value(previous).map_err(err)?;
                let after = serde_json::to_value(def).map_err(err)?;
                if before != after && def.revision <= previous.revision {
                    return Err("definition revision must increase".into());
                }
            }
        }
        let bytes = serde_json::to_vec_pretty(&next).map_err(err)?;
        if bytes.len() > 4 * 1024 * 1024 {
            return Err("catalog size limit".into());
        }
        let temporary = format!(".automations.{}.tmp", uuid::Uuid::new_v4());
        let mut options = OpenOptions::new();
        options
            .write(true)
            .create_new(true)
            .follow(FollowSymlinks::No);
        let mut file = self.dir.open_with(&temporary, &options).map_err(err)?;
        let result = (|| {
            file.write_all(&bytes).map_err(err)?;
            file.sync_all().map_err(err)?;
            self.dir
                .rename(&temporary, &self.dir, "automations.json")
                .map_err(err)?;
            self.dir.open(".").map_err(err)?.sync_all().map_err(err)
        })();
        if result.is_err() {
            let _ = self.dir.remove_file(&temporary);
        }
        result?;
        Ok(next)
    }
}
fn validate_document(doc: &CatalogDocument) -> Result<(), String> {
    if doc.legacy_jobs.len() > 4096 {
        return Err("legacy job count limit".into());
    }
    let mut jobs = HashSet::new();
    for job in &doc.legacy_jobs {
        super::types::identifier(&job.id)?;
        if !jobs.insert(&job.id) {
            return Err("duplicate legacy job".into());
        }
    }
    if doc.definitions.len() > 4096 {
        return Err("definition count limit".into());
    }
    let mut ids = HashSet::new();
    for def in &doc.definitions {
        def.validate()?;
        if !matches!(def.owner, DefinitionOwner::Project) {
            return Err("App definitions belong to published manifests".into());
        }
        if !ids.insert(&def.id) {
            return Err("duplicate definition".into());
        }
    }
    Ok(())
}
