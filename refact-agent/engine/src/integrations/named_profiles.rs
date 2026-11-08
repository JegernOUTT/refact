use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(unix)]
struct AllocationLock(std::fs::File);

#[cfg(unix)]
impl Drop for AllocationLock {
    fn drop(&mut self) {
        use std::os::fd::AsRawFd;
        unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
    }
}

#[derive(Clone)]
pub(super) struct OwnedProfile {
    path: PathBuf,
    #[cfg(unix)]
    dir: Arc<cap_std::fs::Dir>,
    #[cfg(unix)]
    parent: Arc<cap_std::fs::Dir>,
    #[cfg(windows)]
    backend: Arc<refact_browser::windows_profiles::WindowsOwnedProfile>,
}

impl OwnedProfile {
    #[cfg(unix)]
    pub(super) fn create(cache: &Path) -> Result<Self, String> {
        use cap_std::fs::{Dir, DirBuilder, DirBuilderExt, MetadataExt};
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::OpenOptionsExt;
        let io = |e: std::io::Error| format!("named profile filesystem: {e}");
        let root_file = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(cache)
            .map_err(io)?;
        let root = Dir::from_std_file(root_file);
        let mut builder = DirBuilder::new();
        builder.mode(0o700);
        match root.create_dir_with("named_browser_profiles", &builder) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(io(e)),
        }
        let meta = root
            .symlink_metadata("named_browser_profiles")
            .map_err(io)?;
        if !meta.is_dir() || meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0 {
            return Err("named profile parent must be a private owned directory".into());
        }
        let parent = root.open_dir("named_browser_profiles").map_err(io)?;
        let current = parent.dir_metadata().map_err(io)?;
        if meta.dev() != current.dev() || meta.ino() != current.ino() {
            return Err("named profile parent changed".into());
        }
        let lock = parent.open(".").map_err(io)?.into_std();
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(io(std::io::Error::last_os_error()));
        }
        let _allocation_lock = AllocationLock(lock);
        let mut count = 0;
        for entry in parent.entries().map_err(io)? {
            entry.map_err(io)?;
            count += 1;
            if count >= 32 {
                return Err("named profile disk quota exceeded".into());
            }
        }
        let name = format!("profile-{}", uuid::Uuid::new_v4());
        parent.create_dir_with(&name, &builder).map_err(io)?;
        let dir = parent.open_dir(&name).map_err(io)?;
        let path = cache.join("named_browser_profiles").join(name);
        Ok(Self {
            path,
            dir: Arc::new(dir),
            parent: Arc::new(parent),
        })
    }
    #[cfg(windows)]
    pub(super) fn create(cache: &Path) -> Result<Self, String> {
        let backend = refact_browser::windows_profiles::WindowsOwnedProfile::create(cache)?;
        Ok(Self {
            path: backend.path().to_path_buf(),
            backend: Arc::new(backend),
        })
    }
    #[cfg(not(any(unix, windows)))]
    pub(super) fn create(_cache: &Path) -> Result<Self, String> {
        Err("secure ephemeral named profiles are unsupported on this platform".into())
    }
    pub(super) fn path(&self) -> &Path {
        &self.path
    }
    #[cfg(unix)]
    fn lock_safe(&self) -> bool {
        let metadata = match self.dir.symlink_metadata("SingletonLock") {
            Ok(metadata) => metadata,
            Err(error) => return error.kind() == std::io::ErrorKind::NotFound,
        };
        if !metadata.file_type().is_symlink() {
            return false;
        }
        let Ok(target) = self.dir.read_link("SingletonLock") else {
            return false;
        };
        let Some(lock) = super::super::parse_profile_lock(&target.to_string_lossy()) else {
            return false;
        };
        lock.host == super::super::local_hostname() && !super::super::pid_is_alive(lock.pid)
    }
    #[cfg(unix)]
    pub(super) fn cleanup(&self, exited: bool, lock_safe: bool) -> Result<(), String> {
        use cap_std::fs::MetadataExt;
        if !exited || !lock_safe || !self.lock_safe() {
            return Err("process exit or profile lock not verified".into());
        }
        let name = self.path.file_name().ok_or("profile name missing")?;
        let named = self
            .parent
            .symlink_metadata(name)
            .map_err(|e| e.to_string())?;
        let owned = self.dir.dir_metadata().map_err(|e| e.to_string())?;
        if !named.is_dir() || named.dev() != owned.dev() || named.ino() != owned.ino() {
            return Err("profile directory identity changed".into());
        }
        self.dir
            .try_clone()
            .map_err(|e| e.to_string())?
            .remove_open_dir_all()
            .map_err(|e| e.to_string())
    }
    #[cfg(windows)]
    pub(super) fn cleanup(&self, exited: bool, lock_safe: bool) -> Result<(), String> {
        self.backend.cleanup(exited, lock_safe)
    }
    #[cfg(not(any(unix, windows)))]
    pub(super) fn cleanup(&self, _exited: bool, _lock_safe: bool) -> Result<(), String> {
        Err("secure ephemeral named profiles are unsupported on this platform".into())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn cleanup_requires_proof_and_preserves_neighbors_and_symlink_targets() {
        let cache = tempfile::tempdir().unwrap();
        let profile = OwnedProfile::create(cache.path()).unwrap();
        let outside = cache.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("secret"), b"keep").unwrap();
        std::os::unix::fs::symlink(&outside, profile.path().join("link")).unwrap();
        assert!(profile.cleanup(false, true).is_err());
        assert!(profile.cleanup(true, false).is_err());
        profile.cleanup(true, true).unwrap();
        assert!(!profile.path().exists());
        assert_eq!(std::fs::read(outside.join("secret")).unwrap(), b"keep");
    }
    #[test]
    fn retained_disk_entries_bound_new_profiles() {
        let cache = tempfile::tempdir().unwrap();
        let profiles: Vec<_> = (0..32)
            .map(|_| OwnedProfile::create(cache.path()).unwrap())
            .collect();
        assert!(OwnedProfile::create(cache.path()).is_err());
        profiles[0].cleanup(true, true).unwrap();
        assert!(OwnedProfile::create(cache.path()).is_ok());
    }
    #[test]
    fn live_foreign_and_malformed_locks_never_authorize_profile_deletion() {
        let cache = tempfile::tempdir().unwrap();
        let profile = OwnedProfile::create(cache.path()).unwrap();
        let lock = profile.path().join("SingletonLock");
        std::os::unix::fs::symlink(
            format!(
                "{}-{}",
                super::super::super::local_hostname(),
                std::process::id()
            ),
            &lock,
        )
        .unwrap();
        assert!(profile.cleanup(true, true).is_err());
        std::fs::remove_file(&lock).unwrap();
        std::fs::write(&lock, b"not a Chrome symlink").unwrap();
        assert!(profile.cleanup(true, true).is_err());
        std::fs::remove_file(&lock).unwrap();
        std::os::unix::fs::symlink("foreign-host-12345", &lock).unwrap();
        assert!(profile.cleanup(true, true).is_err());
        std::fs::remove_file(&lock).unwrap();
        profile.cleanup(true, true).unwrap();
    }
    #[test]
    fn replaced_root_and_symlink_parent_fail_closed() {
        let cache = tempfile::tempdir().unwrap();
        let profile = OwnedProfile::create(cache.path()).unwrap();
        let moved = profile.path().with_extension("moved");
        std::fs::rename(profile.path(), &moved).unwrap();
        std::fs::create_dir(profile.path()).unwrap();
        std::fs::write(profile.path().join("neighbor"), b"keep").unwrap();
        assert!(profile.cleanup(true, true).is_err());
        assert!(profile.path().join("neighbor").exists());
        let other = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(other.path(), cache.path().join("alias")).unwrap();
        assert!(OwnedProfile::create(&cache.path().join("alias")).is_err());
    }
    #[test]
    fn concurrent_creation_never_exceeds_disk_quota() {
        let cache = tempfile::tempdir().unwrap();
        let root = cache.path().to_path_buf();
        let workers: Vec<_> = (0..64)
            .map(|_| {
                let root = root.clone();
                std::thread::spawn(move || OwnedProfile::create(&root))
            })
            .collect();
        for worker in workers {
            let _ = worker.join().unwrap();
        }
        assert!(
            std::fs::read_dir(root.join("named_browser_profiles"))
                .unwrap()
                .count()
                <= 32
        );
    }
}
