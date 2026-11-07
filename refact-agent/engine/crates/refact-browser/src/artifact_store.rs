//! Runtime-scoped, bounded artifact storage. IDs are capabilities only within this store.
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct StoredArtifact {
    pub id: String,
    /// Presentation-only path; never used for retrieval or eviction.
    pub path: PathBuf,
    pub mime: String,
    pub bytes: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ReportChunk {
    pub content: String,
    pub total_bytes: usize,
    pub next_offset: Option<usize>,
}

pub const MAX_REPORT_CHUNK: usize = 32 * 1024;

#[cfg(unix)]
mod platform {
    use super::*;
    use std::collections::HashMap;
    use std::ffi::CString;
    use std::fs::File;
    use std::io::{Read, Seek, SeekFrom, Write};
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    const MAX_FILES: usize = 64;
    const MAX_BYTES: usize = 32 * 1024 * 1024;
    const MAX_FILE: usize = 8 * 1024 * 1024;
    const TTL: Duration = Duration::from_secs(3600);

    #[derive(Clone)]
    struct Entry {
        bytes: usize,
        dev: u64,
        ino: u64,
        last: Instant,
        mime: String,
    }
    struct State {
        entries: HashMap<String, Entry>,
        bytes: usize,
        write_failed: bool,
    }
    pub struct ArtifactStore {
        root: File,
        directory_name: CString,
        dir: File,
        display_dir: PathBuf,
        state: Mutex<State>,
        clock: Box<dyn Fn() -> Instant + Send + Sync>,
        max_files: usize,
        max_bytes: usize,
        max_file: usize,
        ttl: Duration,
    }
    fn ioerr(e: std::io::Error) -> String {
        format!("artifact filesystem error: {e}")
    }
    fn name(s: &str) -> CString {
        CString::new(s).expect("generated UUID has no NUL")
    }
    fn openat(dir: &File, n: &CString, flags: i32, mode: libc::mode_t) -> Result<File, String> {
        let fd = unsafe {
            libc::openat(
                dir.as_raw_fd(),
                n.as_ptr(),
                flags | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                mode,
            )
        };
        if fd < 0 {
            return Err(ioerr(std::io::Error::last_os_error()));
        }
        Ok(unsafe { File::from_raw_fd(fd) })
    }
    impl ArtifactStore {
        pub fn new(root: &Path) -> Result<Self, String> {
            Self::with_clock(root, Box::new(Instant::now))
        }
        fn with_clock(
            root: &Path,
            clock: Box<dyn Fn() -> Instant + Send + Sync>,
        ) -> Result<Self, String> {
            std::fs::create_dir_all(root).map_err(ioerr)?;
            let root_name = CString::new(root.as_os_str().as_bytes())
                .map_err(|_| "invalid artifact root".to_string())?;
            let fd = unsafe {
                libc::open(
                    root_name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                return Err(ioerr(std::io::Error::last_os_error()));
            }
            let root_fd = unsafe { File::from_raw_fd(fd) };
            let dirname = format!("managed-{}", uuid::Uuid::new_v4());
            let n = name(&dirname);
            if unsafe { libc::mkdirat(root_fd.as_raw_fd(), n.as_ptr(), 0o700) } != 0 {
                return Err(ioerr(std::io::Error::last_os_error()));
            }
            let dir = openat(&root_fd, &n, libc::O_RDONLY | libc::O_DIRECTORY, 0)?;
            Ok(Self {
                root: root_fd,
                directory_name: n,
                dir,
                display_dir: root.join(dirname),
                state: Mutex::new(State {
                    entries: HashMap::new(),
                    bytes: 0,
                    write_failed: false,
                }),
                clock,
                max_files: MAX_FILES,
                max_bytes: MAX_BYTES,
                max_file: MAX_FILE,
                ttl: TTL,
            })
        }
        fn valid_file(&self, id: &str, entry: &Entry) -> Result<File, String> {
            let f = openat(&self.dir, &name(id), libc::O_RDONLY | libc::O_NONBLOCK, 0)?;
            let m = f.metadata().map_err(ioerr)?;
            if !m.is_file()
                || m.nlink() != 1
                || m.dev() != entry.dev
                || m.ino() != entry.ino
                || m.len() != entry.bytes as u64
            {
                return Err("artifact identity changed".into());
            }
            Ok(f)
        }
        fn evict(&self, state: &mut State, id: &str) -> Result<(), String> {
            let entry = state.entries.get(id).ok_or("artifact not found")?;
            // Never unlink an entry that no longer matches the inode we created.
            let _file = self.valid_file(id, entry)?;
            if unsafe { libc::unlinkat(self.dir.as_raw_fd(), name(id).as_ptr(), 0) } != 0 {
                return Err(ioerr(std::io::Error::last_os_error()));
            }
            let entry = state.entries.remove(id).unwrap();
            state.bytes -= entry.bytes;
            Ok(())
        }
        fn expire(&self, state: &mut State, now: Instant) -> Result<(), String> {
            let expired: Vec<_> = state
                .entries
                .iter()
                .filter(|(_, e)| now.saturating_duration_since(e.last) >= self.ttl)
                .map(|(id, _)| id.clone())
                .collect();
            for id in expired {
                self.evict(state, &id)?;
            }
            Ok(())
        }
        pub fn maintenance(&self) -> Result<(), String> {
            let mut state = self.state.lock().map_err(|_| "artifact store poisoned")?;
            self.expire(&mut state, (self.clock)())
        }
        pub fn write(&self, bytes: &[u8], mime: &str) -> Result<StoredArtifact, String> {
            self.write_using(bytes, mime, |file, bytes| file.write_all(bytes))
        }

        fn write_using(
            &self,
            bytes: &[u8],
            mime: &str,
            write: impl FnOnce(&mut File, &[u8]) -> std::io::Result<()>,
        ) -> Result<StoredArtifact, String> {
            if bytes.len() > self.max_file || bytes.len() > self.max_bytes || self.max_files == 0 {
                return Err("artifact exceeds store quota".into());
            }
            if mime.len() > 256 {
                return Err("artifact MIME too long".into());
            }
            let mut state = self.state.lock().map_err(|_| "artifact store poisoned")?;
            if state.write_failed {
                return Err(
                    "artifact store write accounting is uncertain; further writes refused".into(),
                );
            }
            let now = (self.clock)();
            self.expire(&mut state, now)?;
            while state.entries.len() >= self.max_files
                || state.bytes + bytes.len() > self.max_bytes
            {
                let oldest = state
                    .entries
                    .iter()
                    .min_by_key(|(_, e)| e.last)
                    .map(|(id, _)| id.clone())
                    .ok_or("artifact quota unavailable")?;
                self.evict(&mut state, &oldest)?;
            }
            let id = uuid::Uuid::new_v4().to_string();
            let mut file = openat(
                &self.dir,
                &name(&id),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
                0o600,
            )?;
            let metadata = file.metadata().map_err(ioerr)?;
            let entry = Entry {
                bytes: bytes.len(),
                dev: metadata.dev(),
                ino: metadata.ino(),
                last: now,
                mime: mime.to_string(),
            };
            if !metadata.is_file() || metadata.nlink() != 1 {
                return Err("unsafe newly created artifact".into());
            }
            state.bytes += bytes.len();
            state.entries.insert(id.clone(), entry.clone());
            if let Err(error) = write(&mut file, bytes) {
                state.write_failed = true;
                if let Ok(metadata) = file.metadata() {
                    let partial = Entry {
                        bytes: metadata.len() as usize,
                        ..entry
                    };
                    if self.valid_file(&id, &partial).is_ok()
                        && unsafe { libc::unlinkat(self.dir.as_raw_fd(), name(&id).as_ptr(), 0) }
                            == 0
                    {
                        state.entries.remove(&id);
                        state.bytes -= bytes.len();
                        state.write_failed = false;
                    }
                }
                return Err(ioerr(error));
            }
            if let Err(error) = self.valid_file(&id, &entry) {
                state.write_failed = true;
                return Err(error);
            }
            Ok(StoredArtifact {
                path: self.display_dir.join(&id),
                id,
                mime: mime.into(),
                bytes: bytes.len(),
            })
        }
        pub fn read_report(
            &self,
            id: &str,
            offset: usize,
            limit: usize,
        ) -> Result<ReportChunk, String> {
            let mut state = self.state.lock().map_err(|_| "artifact store poisoned")?;
            let now = (self.clock)();
            self.expire(&mut state, now)?;
            let entry = state.entries.get(id).ok_or("artifact not found")?.clone();
            if !entry.mime.starts_with("text/") && entry.mime != "application/json" {
                return Err("artifact is not text".into());
            }
            if offset > entry.bytes || limit == 0 {
                return Err("invalid artifact cursor or limit".into());
            }
            let mut f = self.valid_file(id, &entry)?;
            // Read at most one byte before the cursor plus a bounded chunk and UTF-8 lookahead.
            let start = offset.saturating_sub(1);
            f.seek(SeekFrom::Start(start as u64)).map_err(ioerr)?;
            let mut data = Vec::new();
            f.take((limit.min(MAX_REPORT_CHUNK) + 5) as u64)
                .read_to_end(&mut data)
                .map_err(ioerr)?;
            let skip = offset - start;
            let slice = &data[skip..];
            if slice.first().is_some_and(|b| b & 0xc0 == 0x80) {
                return Err("invalid UTF-8 cursor".into());
            }
            let mut end = slice.len().min(limit.min(MAX_REPORT_CHUNK));
            while end > 0 && end < slice.len() && slice[end] & 0xc0 == 0x80 {
                end -= 1;
            }
            if end == 0 && offset < entry.bytes {
                return Err("chunk limit too small for UTF-8 character".into());
            }
            let content = std::str::from_utf8(&slice[..end])
                .map_err(|_| "artifact is not valid UTF-8")?
                .to_string();
            state.entries.get_mut(id).unwrap().last = now;
            let next = offset + end;
            Ok(ReportChunk {
                content,
                total_bytes: entry.bytes,
                next_offset: (next < entry.bytes).then_some(next),
            })
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        use std::os::unix::fs::symlink;
        use std::sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        };

        #[test]
        fn quota_lru_ttl_and_cross_session() {
            let root = tempfile::tempdir().unwrap();
            let ticks = Arc::new(AtomicU64::new(0));
            let base = Instant::now();
            let t = ticks.clone();
            let mut store = ArtifactStore::with_clock(
                root.path(),
                Box::new(move || base + Duration::from_secs(t.load(Ordering::SeqCst))),
            )
            .unwrap();
            store.max_files = 2;
            store.max_bytes = 6;
            store.max_file = 6;
            let a = store.write(b"aaa", "text/plain").unwrap();
            ticks.store(1, Ordering::SeqCst);
            let b = store.write(b"bb", "text/plain").unwrap();
            ticks.store(2, Ordering::SeqCst);
            store.read_report(&a.id, 0, 10).unwrap();
            let c = store.write(b"cc", "text/plain").unwrap();
            assert!(!b.path.exists());
            assert_eq!(store.read_report(&a.id, 0, 10).unwrap().content, "aaa");
            assert!(store.write(b"1234567", "text/plain").is_err());
            let other = ArtifactStore::new(root.path()).unwrap();
            assert!(other.read_report(&c.id, 0, 10).is_err());
            ticks.store(3602, Ordering::SeqCst);
            store.maintenance().unwrap();
            assert!(!a.path.exists());
            assert!(!c.path.exists());
            assert_eq!(store.state.lock().unwrap().bytes, 0);
        }
        #[test]
        fn chunks_validate_utf8_and_binary() {
            let root = tempfile::tempdir().unwrap();
            let store = ArtifactStore::new(root.path()).unwrap();
            let a = store.write("a€b".as_bytes(), "text/plain").unwrap();
            let first = store.read_report(&a.id, 0, 3).unwrap();
            assert_eq!(first.content, "a");
            assert_eq!(first.next_offset, Some(1));
            assert!(store.read_report(&a.id, 2, 10).is_err());
            assert_eq!(store.read_report(&a.id, 1, 4).unwrap().content, "€b");
            assert!(store.read_report(&a.id, 1, 2).is_err());
            let b = store.write(&[0xff], "image/png").unwrap();
            assert!(store.read_report(&b.id, 0, 8).is_err());
        }
        #[test]
        fn symlink_and_hardlink_substitution_are_rejected() {
            let root = tempfile::tempdir().unwrap();
            let outside = root.path().join("outside");
            std::fs::write(&outside, b"secret").unwrap();
            let store = ArtifactStore::new(root.path()).unwrap();
            let a = store.write(b"public", "text/plain").unwrap();
            std::fs::remove_file(&a.path).unwrap();
            symlink(&outside, &a.path).unwrap();
            assert!(store.read_report(&a.id, 0, 20).is_err());
            assert!(store
                .evict(&mut store.state.lock().unwrap(), &a.id)
                .is_err());
            assert_eq!(std::fs::read(&outside).unwrap(), b"secret");
            std::fs::remove_file(&a.path).unwrap();
            std::fs::hard_link(&outside, &a.path).unwrap();
            assert!(store.read_report(&a.id, 0, 20).is_err());
            assert!(store
                .evict(&mut store.state.lock().unwrap(), &a.id)
                .is_err());
            assert_eq!(std::fs::read(&outside).unwrap(), b"secret");
        }
        #[test]
        fn renamed_root_never_redirects_operations() {
            let base = tempfile::tempdir().unwrap();
            let root = base.path().join("root");
            let moved = base.path().join("moved");
            let store = ArtifactStore::new(&root).unwrap();
            let a = store.write(b"hello", "text/plain").unwrap();
            std::fs::rename(&root, &moved).unwrap();
            std::fs::create_dir(&root).unwrap();
            assert_eq!(store.read_report(&a.id, 0, 9).unwrap().content, "hello");
            let b = store.write(b"world", "text/plain").unwrap();
            assert!(!b.path.exists());
            assert!(moved
                .join(store.display_dir.file_name().unwrap())
                .join(&b.id)
                .exists());
        }
        #[test]
        fn failed_partial_write_releases_quota_and_drop_removes_owned_directory() {
            let root = tempfile::tempdir().unwrap();
            let store = ArtifactStore::new(root.path()).unwrap();
            let managed = store.display_dir.clone();
            let result = store.write_using(b"secret-payload", "text/plain", |file, bytes| {
                file.write_all(&bytes[..3])?;
                Err(std::io::Error::other("injected partial write failure"))
            });
            assert!(result.is_err());
            let state = store.state.lock().unwrap();
            assert_eq!(state.bytes, 0);
            assert!(state.entries.is_empty());
            assert!(!state.write_failed);
            drop(state);
            assert_eq!(std::fs::read_dir(&managed).unwrap().count(), 0);
            let saved = store.write(b"ok", "text/plain").unwrap();
            assert_eq!(store.read_report(&saved.id, 0, 8).unwrap().content, "ok");
            drop(store);
            assert!(!managed.exists());
        }

        #[test]
        fn replaced_root_symlink_is_rejected() {
            let base = tempfile::tempdir().unwrap();
            let outside = base.path().join("outside");
            std::fs::create_dir(&outside).unwrap();
            let root = base.path().join("root");
            symlink(&outside, &root).unwrap();
            assert!(ArtifactStore::new(&root).is_err());
            assert_eq!(std::fs::read_dir(outside).unwrap().count(), 0);
        }
    }
    impl Drop for ArtifactStore {
        fn drop(&mut self) {
            if let Ok(mut state) = self.state.lock() {
                let ids: Vec<_> = state.entries.keys().cloned().collect();
                for id in ids {
                    let _ = self.evict(&mut state, &id);
                }
            }
            if let Ok(named) = openat(
                &self.root,
                &self.directory_name,
                libc::O_RDONLY | libc::O_DIRECTORY,
                0,
            ) {
                if let (Ok(current), Ok(owned)) = (named.metadata(), self.dir.metadata()) {
                    if current.dev() == owned.dev() && current.ino() == owned.ino() {
                        unsafe {
                            libc::unlinkat(
                                self.root.as_raw_fd(),
                                self.directory_name.as_ptr(),
                                libc::AT_REMOVEDIR,
                            );
                        }
                    }
                }
            }
        }
    }
}

#[cfg(windows)]
mod windows_platform {
    use super::*;
    use std::collections::HashMap;
    use std::fs::{File, OpenOptions};
    use std::io::{Read, Seek, SeekFrom, Write};
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use std::path::{Component, Prefix};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, GENERIC_READ, GENERIC_WRITE};
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
    };
    use windows_sys::Win32::Security::{
        GetTokenInformation, TokenUser, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    };
    use windows_sys::Win32::Storage::FileSystem::*;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    const MAX_FILES: usize = 64;
    const MAX_BYTES: usize = 32 * 1024 * 1024;
    const MAX_FILE: usize = 8 * 1024 * 1024;
    const TTL: Duration = Duration::from_secs(3600);

    fn ioerr(e: std::io::Error) -> String {
        format!("artifact filesystem error: {e}")
    }
    fn last_error() -> String {
        ioerr(std::io::Error::last_os_error())
    }
    fn wide(path: &std::ffi::OsStr) -> Result<Vec<u16>, String> {
        let mut value: Vec<_> = path.encode_wide().collect();
        if value.contains(&0) {
            return Err("invalid artifact path".into());
        }
        value.push(0);
        Ok(value)
    }
    struct LocalAllocation(*mut std::ffi::c_void);
    impl Drop for LocalAllocation {
        fn drop(&mut self) {
            unsafe {
                LocalFree(self.0);
            }
        }
    }
    fn private_directory(path: &Path) -> Result<(), String> {
        let mut token: HANDLE = std::ptr::null_mut();
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(last_error());
        }
        let mut needed = 0;
        unsafe {
            GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut needed);
        }
        let mut buffer = vec![0usize; (needed as usize).div_ceil(std::mem::size_of::<usize>())];
        let ok = unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                needed,
                &mut needed,
            )
        };
        let error = last_error();
        unsafe {
            CloseHandle(token);
        }
        if ok == 0 {
            return Err(error);
        }
        let user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
        let mut sid = std::ptr::null_mut();
        if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut sid) } == 0 {
            return Err(last_error());
        }
        let sid_allocation = LocalAllocation(sid.cast());
        let mut len = 0;
        while unsafe { *sid.add(len) } != 0 {
            len += 1;
        }
        let sid_string = String::from_utf16(unsafe { std::slice::from_raw_parts(sid, len) })
            .map_err(|_| "invalid owner SID")?;
        drop(sid_allocation);
        let sddl = wide(std::ffi::OsStr::new(&format!(
            "O:{sid_string}D:P(A;OICI;FA;;;{sid_string})"
        )))?;
        let mut descriptor = std::ptr::null_mut();
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(last_error());
        }
        let _descriptor = LocalAllocation(descriptor);
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        if unsafe { CreateDirectoryW(wide(path.as_os_str())?.as_ptr(), &attributes) } == 0 {
            return Err(last_error());
        }
        Ok(())
    }
    fn info(file: &File) -> Result<BY_HANDLE_FILE_INFORMATION, String> {
        let mut value = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), &mut value) } == 0 {
            return Err(last_error());
        }
        if value.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err("artifact reparse point rejected".into());
        }
        Ok(value)
    }
    fn open_directory(path: &Path, delete: bool) -> Result<File, String> {
        let file = OpenOptions::new()
            .access_mode(
                FILE_READ_ATTRIBUTES | FILE_LIST_DIRECTORY | if delete { DELETE } else { 0 },
            )
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
            .map_err(ioerr)?;
        if info(&file)?.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
            return Err("artifact root is not a directory".into());
        }
        Ok(file)
    }
    fn pin_root(root: &Path) -> Result<(PathBuf, Vec<File>), String> {
        let absolute = std::path::absolute(root).map_err(ioerr)?;
        let mut path = PathBuf::new();
        let mut pins = Vec::new();
        for component in absolute.components() {
            match component {
                Component::Prefix(prefix)
                    if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)) =>
                {
                    path.push(component)
                }
                Component::RootDir => {
                    path.push(component);
                    pins.push(open_directory(&path, false)?);
                }
                Component::Normal(_) => {
                    path.push(component);
                    if !path.try_exists().map_err(ioerr)? {
                        std::fs::create_dir(&path).map_err(ioerr)?;
                    }
                    pins.push(open_directory(&path, false)?);
                }
                _ => {
                    return Err(
                        "artifact root requires a local absolute disk path without traversal"
                            .into(),
                    )
                }
            }
        }
        if pins.is_empty() {
            return Err("invalid artifact root".into());
        }
        Ok((path, pins))
    }
    fn reopen(file: &File, access: u32, share: u32) -> Result<File, String> {
        let handle = unsafe { ReOpenFile(file.as_raw_handle().cast(), access, share, 0) };
        if handle == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
            return Err(last_error());
        }
        Ok(unsafe { File::from_raw_handle(handle.cast()) })
    }
    fn publish_read_handle(entry: &mut Entry) -> Result<(), String> {
        let bridge = reopen(
            &entry.file,
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        )?;
        drop(std::mem::replace(&mut entry.file, bridge));
        entry.published = true;
        let readonly = reopen(&entry.file, GENERIC_READ, FILE_SHARE_READ)?;
        entry.file = readonly;
        entry.published = true;
        validate(entry, entry.bytes)
    }
    fn delete_entry(entry: &mut Entry) -> Result<(), String> {
        if !entry.published {
            return delete_owned(&entry.file);
        }
        let bridge = reopen(
            &entry.file,
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_DELETE,
        )?;
        drop(std::mem::replace(&mut entry.file, bridge));
        let deletion = reopen(&entry.file, DELETE, FILE_SHARE_READ | FILE_SHARE_DELETE);
        match deletion {
            Ok(file) => {
                let result = delete_owned(&file);
                if result.is_err() {
                    drop(file);
                    entry.file = reopen(&entry.file, GENERIC_READ, FILE_SHARE_READ)?;
                }
                result
            }
            Err(error) => {
                entry.file = reopen(&entry.file, GENERIC_READ, FILE_SHARE_READ)?;
                Err(error)
            }
        }
    }
    fn delete_owned(file: &File) -> Result<(), String> {
        let disposition = FILE_DISPOSITION_INFO {
            DeleteFile: true,
        };
        if unsafe {
            SetFileInformationByHandle(
                file.as_raw_handle().cast(),
                FileDispositionInfo,
                (&disposition as *const FILE_DISPOSITION_INFO).cast(),
                std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
            )
        } == 0
        {
            return Err(last_error());
        }
        Ok(())
    }
    struct Entry {
        file: File,
        published: bool,
        identity: (u32, u32, u32),
        bytes: usize,
        last: Instant,
        mime: String,
    }
    fn identity(value: &BY_HANDLE_FILE_INFORMATION) -> (u32, u32, u32) {
        (
            value.dwVolumeSerialNumber,
            value.nFileIndexHigh,
            value.nFileIndexLow,
        )
    }
    fn validate(entry: &Entry, expected_bytes: usize) -> Result<(), String> {
        let value = info(&entry.file)?;
        let size = ((value.nFileSizeHigh as u64) << 32) | value.nFileSizeLow as u64;
        if value.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0
            || value.nNumberOfLinks != 1
            || identity(&value) != entry.identity
            || size != expected_bytes as u64
        {
            return Err("artifact identity changed".into());
        }
        Ok(())
    }
    struct State {
        entries: HashMap<String, Entry>,
        bytes: usize,
        write_failed: bool,
    }
    pub struct ArtifactStore {
        dir: File,
        _root_pins: Vec<File>,
        display_dir: PathBuf,
        state: Mutex<State>,
        clock: Box<dyn Fn() -> Instant + Send + Sync>,
    }
    impl ArtifactStore {
        pub fn new(root: &Path) -> Result<Self, String> {
            Self::with_clock(root, Box::new(Instant::now))
        }
        fn with_clock(
            root: &Path,
            clock: Box<dyn Fn() -> Instant + Send + Sync>,
        ) -> Result<Self, String> {
            let (root, pins) = pin_root(root)?;
            let display_dir = root.join(format!("managed-{}", uuid::Uuid::new_v4()));
            private_directory(&display_dir)?;
            let dir = open_directory(&display_dir, true)?;
            Ok(Self {
                dir,
                _root_pins: pins,
                display_dir,
                state: Mutex::new(State {
                    entries: HashMap::new(),
                    bytes: 0,
                    write_failed: false,
                }),
                clock,
            })
        }
        fn evict(&self, state: &mut State, id: &str) -> Result<(), String> {
            let entry = state.entries.get_mut(id).ok_or("artifact not found")?;
            validate(entry, entry.bytes)?;
            delete_entry(entry)?;
            let entry = state.entries.remove(id).unwrap();
            state.bytes -= entry.bytes;
            Ok(())
        }
        fn expire(&self, state: &mut State, now: Instant) -> Result<(), String> {
            let ids: Vec<_> = state
                .entries
                .iter()
                .filter(|(_, e)| now.saturating_duration_since(e.last) >= TTL)
                .map(|(id, _)| id.clone())
                .collect();
            let mut failure = None;
            for id in ids {
                if let Err(error) = self.evict(state, &id) {
                    failure.get_or_insert(error);
                }
            }
            failure.map_or(Ok(()), Err)
        }
        pub fn maintenance(&self) -> Result<(), String> {
            let mut state = self.state.lock().map_err(|_| "artifact store poisoned")?;
            self.expire(&mut state, (self.clock)())
        }
        pub fn write(&self, bytes: &[u8], mime: &str) -> Result<StoredArtifact, String> {
            self.write_using(bytes, mime, |file, data| file.write_all(data))
        }
        fn write_using(
            &self,
            bytes: &[u8],
            mime: &str,
            write: impl FnOnce(&mut File, &[u8]) -> std::io::Result<()>,
        ) -> Result<StoredArtifact, String> {
            if bytes.len() > MAX_FILE {
                return Err("artifact exceeds store quota".into());
            }
            if mime.len() > 256 {
                return Err("artifact MIME too long".into());
            }
            let mut state = self.state.lock().map_err(|_| "artifact store poisoned")?;
            if state.write_failed {
                return Err(
                    "artifact store write accounting is uncertain; further writes refused".into(),
                );
            }
            let now = (self.clock)();
            let _ = self.expire(&mut state, now);
            while state.entries.len() >= MAX_FILES || state.bytes + bytes.len() > MAX_BYTES {
                let mut candidates: Vec<_> = state
                    .entries
                    .iter()
                    .map(|(id, e)| (e.last, id.clone()))
                    .collect();
                candidates.sort_by_key(|(last, _)| *last);
                if !candidates
                    .into_iter()
                    .any(|(_, id)| self.evict(&mut state, &id).is_ok())
                {
                    return Err(
                        "artifact quota unavailable: owned artifacts could not be deleted".into(),
                    );
                }
            }
            let id = uuid::Uuid::new_v4().to_string();
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .access_mode(GENERIC_READ | GENERIC_WRITE | DELETE)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                .open(self.display_dir.join(&id))
                .map_err(ioerr)?;
            let value = info(&file)?;
            let entry = Entry {
                file,
                published: false,
                identity: identity(&value),
                bytes: bytes.len(),
                last: now,
                mime: mime.into(),
            };
            validate(&entry, 0)?;
            state.bytes += bytes.len();
            state.entries.insert(id.clone(), entry);
            let entry = state.entries.get_mut(&id).unwrap();
            let result = write(&mut entry.file, bytes)
                .map_err(ioerr)
                .and_then(|_| validate(entry, bytes.len()))
                .and_then(|_| publish_read_handle(entry));
            if let Err(error) = result {
                state.write_failed = true;
                let entry = state.entries.get_mut(&id).unwrap();
                if info(&entry.file)
                    .is_ok_and(|v| identity(&v) == entry.identity && v.nNumberOfLinks == 1)
                    && delete_entry(entry).is_ok()
                {
                    state.entries.remove(&id);
                    state.bytes -= bytes.len();
                    state.write_failed = false;
                }
                return Err(error);
            }
            Ok(StoredArtifact {
                path: self.display_dir.join(&id),
                id,
                mime: mime.into(),
                bytes: bytes.len(),
            })
        }
        pub fn read_report(
            &self,
            id: &str,
            offset: usize,
            limit: usize,
        ) -> Result<ReportChunk, String> {
            let mut state = self.state.lock().map_err(|_| "artifact store poisoned")?;
            let now = (self.clock)();
            let _ = self.expire(&mut state, now);
            let entry = state.entries.get_mut(id).ok_or("artifact not found")?;
            if now.saturating_duration_since(entry.last) >= TTL {
                return Err("artifact expired but cleanup is blocked".into());
            }
            if !entry.mime.starts_with("text/") && entry.mime != "application/json" {
                return Err("artifact is not text".into());
            }
            if offset > entry.bytes || limit == 0 {
                return Err("invalid artifact cursor or limit".into());
            }
            validate(entry, entry.bytes)?;
            let start = offset.saturating_sub(1);
            entry
                .file
                .seek(SeekFrom::Start(start as u64))
                .map_err(ioerr)?;
            let mut data = Vec::new();
            (&mut entry.file)
                .take((limit.min(MAX_REPORT_CHUNK) + 5) as u64)
                .read_to_end(&mut data)
                .map_err(ioerr)?;
            let slice = data
                .get(offset - start..)
                .ok_or("artifact shortened during read")?;
            if slice.first().is_some_and(|b| b & 0xc0 == 0x80) {
                return Err("invalid UTF-8 cursor".into());
            }
            let mut end = slice.len().min(limit.min(MAX_REPORT_CHUNK));
            while end > 0 && end < slice.len() && slice[end] & 0xc0 == 0x80 {
                end -= 1;
            }
            if end == 0 && offset < entry.bytes {
                return Err("chunk limit too small for UTF-8 character".into());
            }
            let content = std::str::from_utf8(&slice[..end])
                .map_err(|_| "artifact is not valid UTF-8")?
                .to_string();
            entry.last = now;
            let next = offset + end;
            Ok(ReportChunk {
                content,
                total_bytes: entry.bytes,
                next_offset: (next < entry.bytes).then_some(next),
            })
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        use std::sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        };

        #[test]
        fn windows_quota_lru_and_idle_ttl() {
            let root = tempfile::tempdir().unwrap();
            let ticks = Arc::new(AtomicU64::new(0));
            let t = ticks.clone();
            let base = Instant::now();
            let store = ArtifactStore::with_clock(
                root.path(),
                Box::new(move || base + Duration::from_secs(t.load(Ordering::SeqCst))),
            )
            .unwrap();
            let first = store.write(b"first", "text/plain").unwrap();
            for tick in 1..64 {
                ticks.store(tick, Ordering::SeqCst);
                store.write(b"x", "text/plain").unwrap();
            }
            assert_eq!(store.state.lock().unwrap().entries.len(), 64);
            ticks.store(64, Ordering::SeqCst);
            store.read_report(&first.id, 0, 8).unwrap();
            store.write(b"next", "text/plain").unwrap();
            assert_eq!(store.state.lock().unwrap().entries.len(), 64);
            assert_eq!(store.read_report(&first.id, 0, 8).unwrap().content, "first");
            assert_eq!(std::fs::read_dir(&store.display_dir).unwrap().count(), 64);
            ticks.store(3663, Ordering::SeqCst);
            store.maintenance().unwrap();
            assert_eq!(store.state.lock().unwrap().entries.len(), 2);
            ticks.store(3664, Ordering::SeqCst);
            store.maintenance().unwrap();
            assert!(store.state.lock().unwrap().entries.is_empty());
            assert_eq!(store.state.lock().unwrap().bytes, 0);
            let blocked = store.write(b"blocked", "text/plain").unwrap();
            let reader = OpenOptions::new()
                .access_mode(GENERIC_READ)
                .share_mode(FILE_SHARE_READ)
                .open(&blocked.path)
                .unwrap();
            ticks.store(3665, Ordering::SeqCst);
            let removable = store.write(b"removable", "text/plain").unwrap();
            ticks.store(7265, Ordering::SeqCst);
            assert!(store.maintenance().is_err());
            assert!(!store
                .state
                .lock()
                .unwrap()
                .entries
                .contains_key(&removable.id));
            assert!(store.read_report(&removable.id, 0, 16).is_err());
            assert!(store.read_report(&blocked.id, 0, 16).is_err());
            assert_eq!(store.state.lock().unwrap().bytes, 7);
            let current = store.write(b"current", "text/plain").unwrap();
            assert_eq!(
                store.read_report(&current.id, 0, 16).unwrap().content,
                "current"
            );
            drop(reader);
            store.maintenance().unwrap();
            assert!(!blocked.path.exists());
        }
        #[test]
        fn windows_byte_quota_and_file_ceiling() {
            let root = tempfile::tempdir().unwrap();
            let store = ArtifactStore::new(root.path()).unwrap();
            let payload = vec![b'x'; MAX_FILE];
            let first = store.write(&payload, "text/plain").unwrap();
            for _ in 0..3 {
                store.write(&payload, "text/plain").unwrap();
            }
            assert_eq!(store.state.lock().unwrap().bytes, MAX_BYTES);
            assert!(store.write(&vec![0; MAX_FILE + 1], "text/plain").is_err());
            store.write(b"x", "text/plain").unwrap();
            assert_eq!(store.state.lock().unwrap().bytes, MAX_BYTES - MAX_FILE + 1);
            assert_eq!(store.state.lock().unwrap().entries.len(), 4);
            assert!(!first.path.exists());
        }
        #[test]
        fn windows_bounded_utf8_and_runtime_capability() {
            let root = tempfile::tempdir().unwrap();
            let store = ArtifactStore::new(root.path()).unwrap();
            let artifact = store.write("a€b".as_bytes(), "text/plain").unwrap();
            let first = store.read_report(&artifact.id, 0, 3).unwrap();
            assert_eq!(
                (first.content.as_str(), first.total_bytes, first.next_offset),
                ("a", 5, Some(1))
            );
            assert!(store.read_report(&artifact.id, 2, 10).is_err());
            assert_eq!(store.read_report(&artifact.id, 1, 4).unwrap().content, "€b");
            assert!(store.read_report(&artifact.id, 1, 2).is_err());
            let other = ArtifactStore::new(root.path()).unwrap();
            assert!(other.read_report(&artifact.id, 0, 10).is_err());
            let large = store
                .write(&vec![b'x'; MAX_REPORT_CHUNK + 1], "application/json")
                .unwrap();
            let chunk = store.read_report(&large.id, 0, usize::MAX).unwrap();
            assert_eq!(chunk.content, "x".repeat(MAX_REPORT_CHUNK));
            assert_eq!(chunk.next_offset, Some(MAX_REPORT_CHUNK));
            let binary = store.write(&[0xff], "image/png").unwrap();
            assert!(store.read_report(&binary.id, 0, 8).is_err());
        }
        #[test]
        fn windows_pinned_handles_prevent_substitution() {
            let root = tempfile::tempdir().unwrap();
            let outside = root.path().join("outside");
            std::fs::write(&outside, b"secret").unwrap();
            let store = ArtifactStore::new(&root.path().join("root")).unwrap();
            assert!(
                std::fs::rename(root.path().join("root"), root.path().join("before-write"))
                    .is_err()
            );
            let artifact = store.write(b"public", "text/plain").unwrap();
            let mut reader = OpenOptions::new()
                .access_mode(GENERIC_READ)
                .share_mode(FILE_SHARE_READ)
                .open(&artifact.path)
                .expect("published artifact must support read-only path consumers");
            let mut content = String::new();
            reader.read_to_string(&mut content).unwrap();
            assert_eq!(content, "public");
            assert!(store
                .evict(&mut store.state.lock().unwrap(), &artifact.id)
                .is_err());
            assert_eq!(
                store.read_report(&artifact.id, 0, 8).unwrap().content,
                "public"
            );
            assert_eq!(store.state.lock().unwrap().bytes, 6);
            assert_eq!(store.state.lock().unwrap().entries.len(), 1);
            drop(reader);
            assert!(std::fs::remove_file(&artifact.path).is_err());
            assert!(std::fs::rename(&artifact.path, root.path().join("moved")).is_err());
            assert!(OpenOptions::new().write(true).open(&artifact.path).is_err());
            assert!(
                std::fs::rename(root.path().join("root"), root.path().join("moved-root")).is_err()
            );
            assert_eq!(
                store.read_report(&artifact.id, 0, 8).unwrap().content,
                "public"
            );
            assert_eq!(std::fs::read(&outside).unwrap(), b"secret");
        }
        #[test]
        fn windows_private_acl_is_protected_owner_only() {
            let root = tempfile::tempdir().unwrap();
            let store = ArtifactStore::new(root.path()).unwrap();
            let artifact = store.write(b"private", "text/plain").unwrap();
            let output = std::process::Command::new("powershell")
                .args(["-NoProfile", "-NonInteractive", "-Command", "$ErrorActionPreference='Stop'; $sid=[System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value; $acl=Get-Acl -LiteralPath $env:ARTIFACT_DIR; if (!$acl.AreAccessRulesProtected) {throw 'unprotected DACL'}; $rules=@($acl.GetAccessRules($true,$true,[System.Security.Principal.SecurityIdentifier])); if ($rules.Count -ne 1 -or $rules[0].IdentityReference.Value -ne $sid -or $rules[0].AccessControlType -ne 'Allow' -or $rules[0].FileSystemRights -ne 'FullControl') {throw 'directory ACL not owner-only full control'}; $fileAcl=Get-Acl -LiteralPath $env:ARTIFACT_FILE; $fileRules=@($fileAcl.GetAccessRules($true,$true,[System.Security.Principal.SecurityIdentifier])); if ($fileRules.Count -ne 1 -or $fileRules[0].IdentityReference.Value -ne $sid -or $fileRules[0].AccessControlType -ne 'Allow' -or $fileRules[0].FileSystemRights -ne 'FullControl') {throw 'file ACL not owner-only full control'}"])
                .env("ARTIFACT_DIR", &store.display_dir).env("ARTIFACT_FILE", &artifact.path)
                .output().unwrap();
            assert!(
                output.status.success(),
                "ACL inspection failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        #[test]
        fn windows_existing_junction_root_is_rejected() {
            let root = tempfile::tempdir().unwrap();
            let outside = root.path().join("outside");
            std::fs::create_dir(&outside).unwrap();
            std::fs::write(outside.join("secret"), b"secret").unwrap();
            let junction = root.path().join("junction");
            let status = std::process::Command::new("cmd")
                .args(["/D", "/C", "mklink", "/J"])
                .arg(&junction)
                .arg(&outside)
                .status()
                .unwrap();
            assert!(status.success(), "junction setup must succeed");
            assert!(ArtifactStore::new(&junction).is_err());
            assert!(ArtifactStore::new(&junction.join("child")).is_err());
            assert_eq!(std::fs::read(outside.join("secret")).unwrap(), b"secret");
            assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 1);
            std::fs::remove_dir(junction).unwrap();
        }
        #[test]
        fn windows_partial_write_cleanup_and_owned_drop() {
            let root = tempfile::tempdir().unwrap();
            let store = ArtifactStore::new(root.path()).unwrap();
            let managed = store.display_dir.clone();
            assert!(store
                .write_using(b"secret", "text/plain", |file, data| {
                    file.write_all(&data[..3])?;
                    Err(std::io::Error::other("injected partial write failure"))
                })
                .is_err());
            let state = store.state.lock().unwrap();
            assert_eq!(state.bytes, 0);
            assert!(state.entries.is_empty());
            assert!(!state.write_failed);
            drop(state);
            assert_eq!(std::fs::read_dir(&managed).unwrap().count(), 0);
            let saved = store.write(b"ok", "text/plain").unwrap();
            assert_eq!(store.read_report(&saved.id, 0, 8).unwrap().content, "ok");
            drop(store);
            assert!(!managed.exists());
        }
    }
    impl Drop for ArtifactStore {
        fn drop(&mut self) {
            if let Ok(mut state) = self.state.lock() {
                let ids: Vec<_> = state.entries.keys().cloned().collect();
                for id in ids {
                    let _ = self.evict(&mut state, &id);
                }
            }
            let _ = delete_owned(&self.dir);
        }
    }
}

#[cfg(windows)]
pub use windows_platform::ArtifactStore;

#[cfg(unix)]
pub use platform::ArtifactStore;

#[cfg(not(any(unix, windows)))]
pub struct ArtifactStore;
#[cfg(not(any(unix, windows)))]
impl ArtifactStore {
    pub fn new(_: &Path) -> Result<Self, String> {
        Err("secure artifact store is unsupported on this platform".into())
    }
    pub fn write(&self, _: &[u8], _: &str) -> Result<StoredArtifact, String> {
        Err("secure artifact store is unsupported on this platform".into())
    }
    pub fn read_report(&self, _: &str, _: usize, _: usize) -> Result<ReportChunk, String> {
        Err("secure artifact store is unsupported on this platform".into())
    }
    pub fn maintenance(&self) -> Result<(), String> {
        Err("secure artifact store is unsupported on this platform".into())
    }
}
