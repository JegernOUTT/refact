use crate::runtime_manifest::{decode, require, ContractResult};
use crate::runtime_selection::{RuntimeSelection, MAX_SELECTION_BYTES};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotWire {
    schema_version: u32,
    revision: u64,
    selection: RuntimeSelection,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "SnapshotWire", into = "SnapshotWire")]
pub struct SelectionSnapshot(SnapshotWire);
impl From<SelectionSnapshot> for SnapshotWire {
    fn from(s: SelectionSnapshot) -> Self {
        s.0
    }
}
impl TryFrom<SnapshotWire> for SelectionSnapshot {
    type Error = String;
    fn try_from(w: SnapshotWire) -> ContractResult<Self> {
        require(
            w.schema_version == 1 && w.revision > 0,
            "invalid selection schema/revision",
        )?;
        Ok(Self(w))
    }
}
impl SelectionSnapshot {
    pub fn revision(&self) -> u64 {
        self.0.revision
    }
    pub fn selection(&self) -> &RuntimeSelection {
        &self.0.selection
    }
}
pub struct SelectionStore {
    path: PathBuf,
}
impl SelectionStore {
    pub fn new(path: PathBuf) -> ContractResult<Self> {
        require(
            path.is_absolute() && path.file_name().is_some(),
            "absolute selection store path required",
        )?;
        Ok(Self {
            path,
        })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    fn parent(&self) -> ContractResult<&Path> {
        self.path
            .parent()
            .ok_or_else(|| "store parent absent".into())
    }
    fn check_parent(&self) -> ContractResult<()> {
        let p = self.parent()?;
        require(
            dunce::canonicalize(p).map_err(|e| e.to_string())? == p
                || p.canonicalize().map_err(|e| e.to_string())? == p,
            "store ancestors must be canonical, not symlinks",
        )?;
        let m = std::fs::symlink_metadata(p).map_err(|e| e.to_string())?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            for ancestor in p.ancestors() {
                let metadata = std::fs::symlink_metadata(ancestor).map_err(|e| e.to_string())?;
                require(
                    metadata.file_attributes() & 0x400 == 0,
                    "store ancestor reparse point rejected",
                )?;
            }
        }
        require(
            m.is_dir() && !m.permissions().readonly(),
            "store parent rejected",
        )?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            require(
                m.mode() & 0o022 == 0 && m.uid() == unsafe { libc::geteuid() },
                "store parent must be owned by current user and not shared",
            )?;
        }
        Ok(())
    }
    pub fn load(&self) -> ContractResult<Option<SelectionSnapshot>> {
        self.check_parent()?;
        match std::fs::symlink_metadata(&self.path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.to_string()),
            Ok(m) => require(
                m.is_file()
                    && !m.file_type().is_symlink()
                    && !m.permissions().readonly()
                    && m.len() <= MAX_SELECTION_BYTES as u64,
                "selection file type/mode/size rejected",
            )?,
        }
        let file = safe_open(&self.path, false)?;
        check_file(&file)?;
        let mut bytes = Vec::new();
        file.take(MAX_SELECTION_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        decode(&bytes, MAX_SELECTION_BYTES).map(Some)
    }
    pub fn compare_and_swap(
        &self,
        expected_revision: u64,
        selection: RuntimeSelection,
    ) -> ContractResult<SelectionSnapshot> {
        self.check_parent()?;
        let mut lockname = self.path.as_os_str().to_os_string();
        lockname.push(".lock");
        with_file_lock(Path::new(&lockname), || {
            let previous = self.load()?;
            require(
                previous.as_ref().map_or(0, SelectionSnapshot::revision) == expected_revision,
                "selection revision conflict",
            )?;
            let revision = expected_revision
                .checked_add(1)
                .ok_or("selection revision overflow")?;
            let snapshot = SelectionSnapshot(SnapshotWire {
                schema_version: 1,
                revision,
                selection,
            });
            let bytes = serde_json::to_vec(&snapshot).map_err(|e| e.to_string())?;
            require(
                bytes.len() <= MAX_SELECTION_BYTES,
                "selection byte limit exceeded",
            )?;
            let mut temp =
                tempfile::NamedTempFile::new_in(self.parent()?).map_err(|e| e.to_string())?;
            temp.write_all(&bytes).map_err(|e| e.to_string())?;
            temp.flush().map_err(|e| e.to_string())?;
            temp.as_file().sync_all().map_err(|e| e.to_string())?;
            self.check_parent()?;
            temp.persist(&self.path).map_err(|e| e.to_string())?;
            #[cfg(unix)]
            File::open(self.parent()?)
                .and_then(|f| f.sync_all())
                .map_err(|e| e.to_string())?;
            Ok(snapshot)
        })
    }

    /// Crate-private synchronous fence shared with CAS. The closure runs with the same
    /// cooperative lock held; it must not await or acquire locks in inverse order.
    pub(crate) fn with_current<T>(
        &self,
        expected_revision: u64,
        operation: impl FnOnce(&SelectionSnapshot) -> ContractResult<T>,
    ) -> ContractResult<T> {
        require(
            expected_revision > 0,
            "positive selection revision required",
        )?;
        self.check_parent()?;
        let mut lockname = self.path.as_os_str().to_os_string();
        lockname.push(".lock");
        with_file_lock(Path::new(&lockname), || {
            let snapshot = self.load()?.ok_or("selection absent")?;
            require(
                snapshot.revision() == expected_revision,
                "selection revision conflict",
            )?;
            operation(&snapshot)
        })
    }
}

/// Existing CAS lock validation, shared without duplicating the lock namespace.
/// Exclusive locking also for observations keeps platform behavior identical to CAS.
pub(crate) fn with_file_lock<T>(
    path: &Path,
    operation: impl FnOnce() -> ContractResult<T>,
) -> ContractResult<T> {
    with_file_lock_mode(path, true, operation)
}

pub(crate) fn with_file_lock_existing<T>(
    path: &Path,
    operation: impl FnOnce() -> ContractResult<T>,
) -> ContractResult<T> {
    with_file_lock_mode(path, false, operation)
}

fn with_file_lock_mode<T>(
    path: &Path,
    create: bool,
    operation: impl FnOnce() -> ContractResult<T>,
) -> ContractResult<T> {
    let lockfile = safe_open(path, create)?;
    check_file(&lockfile)?;
    let mut lock = fd_lock::RwLock::new(lockfile);
    let guard = lock.write().map_err(|e| e.to_string())?;
    let named = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    require(
        named.is_file() && !named.file_type().is_symlink(),
        "selection lock path changed",
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let held = guard.metadata().map_err(|e| e.to_string())?;
        require(
            (held.dev(), held.ino()) == (named.dev(), named.ino()),
            "selection lock identity changed",
        )?;
    }
    operation()
}
pub(crate) fn safe_open(path: &Path, create: bool) -> ContractResult<File> {
    if let Ok(m) = std::fs::symlink_metadata(path) {
        require(
            m.is_file() && !m.file_type().is_symlink() && !m.permissions().readonly(),
            "store file rejected",
        )?;
    }
    let mut o = OpenOptions::new();
    o.read(true).write(create).create(create);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .mode(0o600);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        o.custom_flags(0x00200000);
    }
    o.open(path).map_err(|e| e.to_string())
}
pub(crate) fn check_file(file: &File) -> ContractResult<()> {
    let m = file.metadata().map_err(|e| e.to_string())?;
    require(
        m.is_file() && !m.permissions().readonly(),
        "store file rejected",
    )?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        require(
            m.file_attributes() & 0x400 == 0,
            "store reparse point rejected",
        )?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        require(
            m.nlink() == 1 && m.mode() & 0o022 == 0 && m.uid() == unsafe { libc::geteuid() },
            "store file owner/links/mode rejected",
        )?;
    }
    Ok(())
}
