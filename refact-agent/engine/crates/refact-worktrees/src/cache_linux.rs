use std::collections::BTreeMap;
use std::ffi::{CStr, CString};
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::*;

pub(super) struct Directory {
    file: File,
    path: PathBuf,
    protected: bool,
}

impl Directory {
    pub(super) fn absolute(path: &Path, protected: bool) -> Result<Self, CacheReason> {
        if !path.is_absolute() || path.as_os_str().len() > 4096 {
            return Err(CacheReason::UnsafePath);
        }
        let root = CString::new("/").map_err(|_| CacheReason::UnsafePath)?;
        let fd = unsafe {
            libc::open(
                root.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        };
        let file = checked_fd(fd)?;
        let mut directory = Self {
            file,
            path: PathBuf::from("/"),
            protected,
        };
        directory.check_protection()?;
        for part in path.components() {
            match part {
                Component::RootDir => {}
                Component::Normal(name) => {
                    let name = name.to_str().ok_or(CacheReason::UnsafePath)?;
                    directory = directory.child(name)?;
                }
                _ => return Err(CacheReason::UnsafePath),
            }
        }
        Ok(directory)
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn validate_path(&self) -> Result<(), CacheReason> {
        let current = Self::absolute(&self.path, self.protected)?;
        let before = self.file.metadata().map_err(|_| CacheReason::IoFailure)?;
        let after = current
            .file
            .metadata()
            .map_err(|_| CacheReason::IoFailure)?;
        if before.dev() != after.dev() || before.ino() != after.ino() {
            return Err(CacheReason::IdentityChanged);
        }
        Ok(())
    }

    fn check_protection(&self) -> Result<(), CacheReason> {
        if self.protected {
            trusted_metadata(&self.file)?;
            refuse_writable(self.file.as_raw_fd(), c".", libc::AT_EACCESS)?;
        }
        Ok(())
    }

    fn child(&self, name: &str) -> Result<Self, CacheReason> {
        let name_c = name_c(name)?;
        let fd = unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                name_c.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        let file = checked_fd(fd)?;
        let child = Self {
            file,
            path: self.path.join(name),
            protected: self.protected,
        };
        child.check_protection()?;
        Ok(child)
    }

    fn relative(&self, relative: &str) -> Result<Self, CacheReason> {
        let mut directory = Self {
            file: self.file.try_clone().map_err(|_| CacheReason::IoFailure)?,
            path: self.path.clone(),
            protected: self.protected,
        };
        for part in relative_parts(relative)? {
            directory = directory.child(part)?;
        }
        Ok(directory)
    }

    fn parent(&self, relative: &str) -> Result<(Self, String), CacheReason> {
        let parts = relative_parts(relative)?;
        let name = parts.last().ok_or(CacheReason::UnsafePath)?.to_string();
        let parent = if parts.len() == 1 {
            Self {
                file: self.file.try_clone().map_err(|_| CacheReason::IoFailure)?,
                path: self.path.clone(),
                protected: self.protected,
            }
        } else {
            self.relative(&parts[..parts.len() - 1].join("/"))?
        };
        Ok((parent, name))
    }

    fn exists(&self, name: &str) -> Result<bool, CacheReason> {
        let name = name_c(name)?;
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        let result = unsafe {
            libc::fstatat(
                self.file.as_raw_fd(),
                name.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if result == 0 {
            return Ok(true);
        }
        if std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) {
            Ok(false)
        } else {
            Err(CacheReason::IoFailure)
        }
    }

    fn open_file(&self, name: &str) -> Result<File, CacheReason> {
        let name = name_c(name)?;
        let fd = unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        let file = checked_fd(fd)?;
        if !file
            .metadata()
            .map_err(|_| CacheReason::IoFailure)?
            .is_file()
        {
            return Err(CacheReason::UnsafePath);
        }
        if file.metadata().map_err(|_| CacheReason::IoFailure)?.nlink() != 1 {
            return Err(CacheReason::UnsafePath);
        }
        if self.protected {
            trusted_metadata(&file)?;
            refuse_writable(
                self.file.as_raw_fd(),
                &name,
                libc::AT_EACCESS | libc::AT_SYMLINK_NOFOLLOW,
            )?;
        }
        Ok(file)
    }

    pub(super) fn read_optional(
        &self,
        name: &str,
        max: u64,
        budget: &mut Budget,
    ) -> Result<Option<Vec<u8>>, CacheReason> {
        budget.check()?;
        if !self.exists(name)? {
            return Ok(None);
        }
        read_limited(self.open_file(name)?, max, budget).map(Some)
    }

    fn read(&self, relative: &str, max: u64, budget: &mut Budget) -> Result<Vec<u8>, CacheReason> {
        budget.check()?;
        let (parent, name) = self.parent(relative)?;
        read_limited(parent.open_file(&name)?, max, budget)
    }

    fn create_file(&self, name: &str, executable: bool) -> Result<File, CacheReason> {
        let name = name_c(name)?;
        let fd = unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                name.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                if executable { 0o700 } else { 0o600 },
            )
        };
        checked_fd(fd)
    }

    fn mkdir(&self, name: &str) -> Result<Self, CacheReason> {
        let name_c = name_c(name)?;
        if unsafe { libc::mkdirat(self.file.as_raw_fd(), name_c.as_ptr(), 0o700) } != 0 {
            return Err(CacheReason::IoFailure);
        }
        self.child(name)
    }

    fn names(&self, budget: &mut Budget) -> Result<Vec<String>, CacheReason> {
        let fd = unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                c".".as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        };
        let file = checked_fd(fd)?;
        let duplicate = unsafe { libc::dup(file.as_raw_fd()) };
        if duplicate < 0 {
            return Err(CacheReason::IoFailure);
        }
        let stream = unsafe { libc::fdopendir(duplicate) };
        if stream.is_null() {
            unsafe { libc::close(duplicate) };
            return Err(CacheReason::IoFailure);
        }
        let result = (|| {
            let mut names = Vec::new();
            loop {
                budget.check()?;
                unsafe { *libc::__errno_location() = 0 };
                let entry = unsafe { libc::readdir(stream) };
                if entry.is_null() {
                    if std::io::Error::last_os_error().raw_os_error().unwrap_or(0) != 0 {
                        return Err(CacheReason::IoFailure);
                    }
                    break;
                }
                let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }
                    .to_str()
                    .map_err(|_| CacheReason::UnsafePath)?;
                if name == "." || name == ".." {
                    continue;
                }
                name_c(name)?;
                budget.charge(0)?;
                names.push(name.to_string());
            }
            names.sort();
            Ok(names)
        })();
        unsafe { libc::closedir(stream) };
        result
    }

    fn kind(&self, name: &str) -> Result<u32, CacheReason> {
        let name = name_c(name)?;
        let mut info = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe {
            libc::fstatat(
                self.file.as_raw_fd(),
                name.as_ptr(),
                info.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return Err(CacheReason::IoFailure);
        }
        let info = unsafe { info.assume_init() };
        if self.protected
            && (info.st_uid != 0
                || (info.st_mode & libc::S_IFMT != libc::S_IFLNK && info.st_mode & 0o022 != 0))
        {
            return Err(CacheReason::UntrustedProducer);
        }
        Ok(info.st_mode & libc::S_IFMT)
    }

    fn link_target(&self, name: &str, budget: &mut Budget) -> Result<String, CacheReason> {
        let name = name_c(name)?;
        let limit = budget.read_limit(MAX_PATH + 1)?;
        let mut bytes = vec![0u8; limit];
        let size = unsafe {
            libc::readlinkat(
                self.file.as_raw_fd(),
                name.as_ptr(),
                bytes.as_mut_ptr().cast(),
                bytes.len(),
            )
        };
        if size < 0 {
            return Err(CacheReason::UnsafePath);
        }
        budget.charge(size as u64)?;
        if size as usize > MAX_PATH {
            return Err(CacheReason::UnsafePath);
        }
        if size as usize == limit {
            return Err(CacheReason::LimitExceeded);
        }
        bytes.truncate(size as usize);
        String::from_utf8(bytes).map_err(|_| CacheReason::UnsafePath)
    }

    fn symlink(&self, name: &str, target: &str) -> Result<(), CacheReason> {
        let name = name_c(name)?;
        let target = CString::new(target).map_err(|_| CacheReason::UnsafePath)?;
        if unsafe { libc::symlinkat(target.as_ptr(), self.file.as_raw_fd(), name.as_ptr()) } != 0 {
            return Err(CacheReason::IoFailure);
        }
        Ok(())
    }

    fn publish(&self, temporary: &str, destination: &str) -> Result<(), CacheReason> {
        let temporary = name_c(temporary)?;
        let destination = name_c(destination)?;
        if unsafe {
            libc::renameat2(
                self.file.as_raw_fd(),
                temporary.as_ptr(),
                self.file.as_raw_fd(),
                destination.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        } != 0
        {
            return Err(CacheReason::ExistingUnownedOutput);
        }
        self.file.sync_all().map_err(|_| CacheReason::IoFailure)
    }
}

fn checked_fd(fd: RawFd) -> Result<File, CacheReason> {
    if fd < 0 {
        Err(CacheReason::UnsafePath)
    } else {
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

fn trusted_metadata(file: &File) -> Result<(), CacheReason> {
    let metadata = file.metadata().map_err(|_| CacheReason::IoFailure)?;
    if metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
        return Err(CacheReason::UntrustedProducer);
    }
    Ok(())
}

fn refuse_writable(directory: RawFd, name: &CStr, flags: libc::c_int) -> Result<(), CacheReason> {
    if unsafe { libc::faccessat(directory, name.as_ptr(), libc::W_OK, flags) } == 0 {
        return Err(CacheReason::UntrustedProducer);
    }
    match std::io::Error::last_os_error().raw_os_error() {
        Some(libc::EACCES | libc::EROFS) => Ok(()),
        _ => Err(CacheReason::UnverifiedIsolation),
    }
}

fn name_c(name: &str) -> Result<CString, CacheReason> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.len() > 255
        || !name.is_ascii()
        || name.contains(['/', '\\'])
        || name.bytes().any(|b| b.is_ascii_control())
    {
        return Err(CacheReason::UnsafePath);
    }
    CString::new(name).map_err(|_| CacheReason::UnsafePath)
}

fn relative_parts(relative: &str) -> Result<Vec<&str>, CacheReason> {
    if relative.len() > MAX_PATH {
        return Err(CacheReason::LimitExceeded);
    }
    let parts: Vec<_> = relative.split('/').collect();
    if parts.len() > MAX_DEPTH {
        return Err(CacheReason::LimitExceeded);
    }
    for part in &parts {
        name_c(part)?;
    }
    Ok(parts)
}

fn read_limited(mut file: File, max: u64, budget: &mut Budget) -> Result<Vec<u8>, CacheReason> {
    budget.check()?;
    let size = file.metadata().map_err(|_| CacheReason::IoFailure)?.len();
    if size > max || size > budget.max_bytes.saturating_sub(budget.bytes) {
        return Err(CacheReason::LimitExceeded);
    }
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let limit = (max + 1 - bytes.len() as u64).min(buffer.len() as u64) as usize;
        let limit = budget.read_limit(limit)?;
        let size = file
            .read(&mut buffer[..limit])
            .map_err(|_| CacheReason::IoFailure)?;
        budget.check()?;
        if size == 0 {
            return Ok(bytes);
        }
        budget.charge(size as u64)?;
        if bytes.len() as u64 + size as u64 > max {
            return Err(CacheReason::LimitExceeded);
        }
        bytes.extend_from_slice(&buffer[..size]);
    }
}

pub(super) fn read_absolute(
    path: &Path,
    max: u64,
    protected: bool,
    budget: &mut Budget,
) -> Result<Vec<u8>, CacheReason> {
    budget.check()?;
    let parent = Directory::absolute(path.parent().ok_or(CacheReason::UnsafePath)?, protected)?;
    parent.read(
        path.file_name()
            .and_then(|name| name.to_str())
            .ok_or(CacheReason::UnsafePath)?,
        max,
        budget,
    )
}

pub(super) fn validate_checkout_backref(
    context: &CacheExecutionContext,
    budget: &mut Budget,
) -> Result<(), CacheReason> {
    let marker = context.checkout.read(".git", 4096, budget)?;
    let marker = std::str::from_utf8(&marker).map_err(|_| CacheReason::IdentityChanged)?;
    let target = marker
        .trim()
        .strip_prefix("gitdir: ")
        .ok_or(CacheReason::IdentityChanged)?;
    let resolved = dunce::canonicalize(context.meta.root.join(target))
        .map_err(|_| CacheReason::IdentityChanged)?;
    let backref = context.gitdir.read("gitdir", 4096, budget)?;
    let backref = std::str::from_utf8(&backref).map_err(|_| CacheReason::IdentityChanged)?;
    if resolved != context.gitdir.path
        || Path::new(backref.trim()) != context.meta.root.join(".git")
    {
        return Err(CacheReason::IdentityChanged);
    }
    Ok(())
}

fn host_boundary(budget: &mut Budget) -> Result<(), CacheReason> {
    budget.check()?;
    if unsafe { libc::geteuid() } == 0 || unsafe { libc::getuid() } != unsafe { libc::geteuid() } {
        return Err(CacheReason::UnverifiedIsolation);
    }
    let mapping = read_proc("/proc/self/uid_map", 4096, budget)?;
    let fields: Vec<_> = mapping.split_whitespace().collect();
    if fields != ["0", "0", "4294967295"] {
        return Err(CacheReason::UnverifiedIsolation);
    }
    let status = read_proc("/proc/self/status", 64 * 1024, budget)?;
    for field in ["CapEff:", "CapPrm:"] {
        let value = status
            .lines()
            .find_map(|line| line.strip_prefix(field))
            .ok_or(CacheReason::UnverifiedIsolation)?;
        if u64::from_str_radix(value.trim(), 16).map_err(|_| CacheReason::UnverifiedIsolation)? != 0
        {
            return Err(CacheReason::UnverifiedIsolation);
        }
    }
    Ok(())
}

fn read_proc(path: &str, max: u64, budget: &mut Budget) -> Result<String, CacheReason> {
    budget.check()?;
    let file = File::open(path).map_err(|_| CacheReason::UnverifiedIsolation)?;
    let bytes = read_limited(file, max, budget).map_err(|reason| match reason {
        CacheReason::Cancelled | CacheReason::LimitExceeded => reason,
        _ => CacheReason::UnverifiedIsolation,
    })?;
    String::from_utf8(bytes).map_err(|_| CacheReason::UnverifiedIsolation)
}

pub(super) fn controller_pin_for_checkout(
    checkout: &Directory,
    budget: &mut Budget,
) -> Result<String, CacheReason> {
    checkout
        .relative("refact-agent/gui")
        .map_err(|_| CacheReason::PackageAbsent)?;
    controller_pin(budget)
}

pub(super) fn controller_pin(budget: &mut Budget) -> Result<String, CacheReason> {
    host_boundary(budget)?;
    let etc = Directory::absolute(Path::new("/etc"), true)
        .map_err(|_| CacheReason::UntrustedController)?;
    if !etc.exists("refact")? {
        return Err(CacheReason::NoTrustedEntry);
    }
    let parent = etc.child("refact")?;
    if !parent.exists("worktree-cache")? {
        return Err(CacheReason::NoTrustedEntry);
    }
    let controller = parent.child("worktree-cache")?;
    let bytes = controller
        .read_optional("controller.json", MAX_JSON_BYTES, budget)?
        .ok_or(CacheReason::NoTrustedEntry)?;
    Ok(digest(&bytes))
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Environment {
    node: String,
    node_abi: String,
    platform: String,
    arch: String,
    libc: String,
    npm: String,
    install_policy: String,
}

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Inputs {
    schema: u32,
    files: BTreeMap<String, Option<String>>,
    environment: Environment,
    copy_policy: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Controller {
    schema: u32,
    kind: String,
    environment: Environment,
    packages: Vec<PackagePin>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PackagePin {
    package: String,
    entry: String,
    receipt_sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SourceReceipt {
    schema: u32,
    kind: String,
    producer: String,
    install_id: String,
    source: String,
    inputs: Inputs,
    key: String,
    tree_sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
enum Entry {
    Directory {
        path: String,
    },
    File {
        path: String,
        sha256: String,
        executable: bool,
    },
    Symlink {
        path: String,
        target: String,
        resolved: String,
    },
}

impl Entry {
    fn path(&self) -> &str {
        match self {
            Self::Directory {
                path,
            }
            | Self::File {
                path, ..
            }
            | Self::Symlink {
                path, ..
            } => path,
        }
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Owner {
    schema: u32,
    checkout_id: String,
    checkout_device: u64,
    checkout_inode: u64,
    head: String,
    package: String,
    source_receipt_sha256: String,
    key: String,
    tree_sha256: String,
}

fn encoded<T: Serialize>(value: &T) -> Result<Vec<u8>, CacheReason> {
    fn ordered(value: &serde_json::Value, output: &mut Vec<u8>) -> Result<(), CacheReason> {
        match value {
            serde_json::Value::Object(map) => {
                output.push(b'{');
                let sorted: BTreeMap<_, _> = map.iter().collect();
                for (index, (key, value)) in sorted.into_iter().enumerate() {
                    if index != 0 {
                        output.push(b',');
                    }
                    output.extend(serde_json::to_vec(key).map_err(|_| CacheReason::CorruptBlob)?);
                    output.push(b':');
                    ordered(value, output)?;
                }
                output.push(b'}');
            }
            serde_json::Value::Array(values) => {
                output.push(b'[');
                for (index, value) in values.iter().enumerate() {
                    if index != 0 {
                        output.push(b',');
                    }
                    ordered(value, output)?;
                }
                output.push(b']');
            }
            other => {
                output.extend(serde_json::to_vec(other).map_err(|_| CacheReason::CorruptBlob)?)
            }
        }
        Ok(())
    }
    let mut bytes = Vec::new();
    ordered(
        &serde_json::to_value(value).map_err(|_| CacheReason::CorruptBlob)?,
        &mut bytes,
    )?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn file_digest(
    mut file: File,
    mut output: Option<&mut File>,
    budget: &mut Budget,
) -> Result<String, CacheReason> {
    let metadata = file.metadata().map_err(|_| CacheReason::IoFailure)?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        return Err(CacheReason::LimitExceeded);
    }
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let limit = budget.read_limit(buffer.len())?;
        let size = file
            .read(&mut buffer[..limit])
            .map_err(|_| CacheReason::IoFailure)?;
        budget.check()?;
        if size == 0 {
            break;
        }
        budget.charge(size as u64)?;
        digest.update(&buffer[..size]);
        if let Some(output) = output.as_deref_mut() {
            output
                .write_all(&buffer[..size])
                .map_err(|_| CacheReason::IoFailure)?;
        }
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn inputs(
    package: &Directory,
    environment: &Environment,
    budget: &mut Budget,
) -> Result<Inputs, CacheReason> {
    let mut files = BTreeMap::new();
    let mut documents = Vec::new();
    for name in ["package.json", "package-lock.json", ".npmrc"] {
        budget.check()?;
        let bytes = package.read_optional(name, MAX_INPUT_BYTES, budget)?;
        if bytes.is_none() && name != ".npmrc" {
            return Err(CacheReason::InputMismatch);
        }
        if let Some(bytes) = &bytes {
            if name != ".npmrc" {
                documents.push(
                    serde_json::from_slice::<serde_json::Value>(bytes)
                        .map_err(|_| CacheReason::InputMismatch)?,
                );
            }
        }
        files.insert(name.to_string(), bytes.map(|bytes| digest(&bytes)));
    }
    let manifest = &documents[0];
    let lock = &documents[1];
    let root = lock
        .get("packages")
        .and_then(|packages| packages.get(""))
        .ok_or(CacheReason::InputMismatch)?;
    if !manifest.is_object()
        || !root.is_object()
        || !matches!(
            lock.get("lockfileVersion")
                .and_then(|version| version.as_u64()),
            Some(2 | 3)
        )
    {
        return Err(CacheReason::InputMismatch);
    }
    for field in [
        "name",
        "version",
        "dependencies",
        "devDependencies",
        "optionalDependencies",
    ] {
        if manifest.get(field) != root.get(field) {
            return Err(CacheReason::InputMismatch);
        }
    }
    if package.exists("patches")? {
        patch_inputs(&package.child("patches")?, "patches", &mut files, budget)?;
    }
    Ok(Inputs {
        schema: 1,
        files,
        environment: environment.clone(),
        copy_policy: "private-regular-files-and-internal-file-links-v1".into(),
    })
}

fn patch_inputs(
    directory: &Directory,
    prefix: &str,
    files: &mut BTreeMap<String, Option<String>>,
    budget: &mut Budget,
) -> Result<(), CacheReason> {
    relative_parts(prefix)?;
    for name in directory.names(budget)? {
        let path = format!("{prefix}/{name}");
        relative_parts(&path)?;
        match directory.kind(&name)? {
            libc::S_IFDIR => patch_inputs(&directory.child(&name)?, &path, files, budget)?,
            libc::S_IFREG => {
                files.insert(
                    path,
                    Some(file_digest(directory.open_file(&name)?, None, budget)?),
                );
            }
            _ => return Err(CacheReason::UnsafePath),
        }
    }
    Ok(())
}

fn excluded(name: &str) -> bool {
    [
        ".cache",
        ".turbo",
        ".parcel-cache",
        ".eslintcache",
        ".rpt2_cache",
        ".tscache",
        ".swc",
        ".rollup.cache",
        ".webpack",
        ".next",
        ".nuxt",
        ".svelte-kit",
        "coverage",
        ".nyc_output",
    ]
    .contains(&name)
        || name.starts_with(".vite")
        || name.ends_with(".tsbuildinfo")
}

fn inventory(directory: &Directory, budget: &mut Budget) -> Result<Vec<Entry>, CacheReason> {
    fn walk(
        directory: &Directory,
        prefix: &str,
        entries: &mut Vec<Entry>,
        budget: &mut Budget,
    ) -> Result<(), CacheReason> {
        for name in directory.names(budget)? {
            if excluded(&name) || (prefix.is_empty() && name == OWNER_NAME) {
                continue;
            }
            if prefix.is_empty() && (name == "node_modules" || name == ".refact-deps-receipt.json")
            {
                return Err(CacheReason::UnsafePath);
            }
            let path = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            relative_parts(&path)?;
            let entry = match directory.kind(&name)? {
                libc::S_IFDIR => {
                    entries.push(Entry::Directory {
                        path: path.clone(),
                    });
                    walk(&directory.child(&name)?, &path, entries, budget)?;
                    continue;
                }
                libc::S_IFREG => {
                    let file = directory.open_file(&name)?;
                    let executable =
                        file.metadata().map_err(|_| CacheReason::IoFailure)?.mode() & 0o111 != 0;
                    Entry::File {
                        path,
                        sha256: file_digest(file, None, budget)?,
                        executable,
                    }
                }
                libc::S_IFLNK => {
                    let target = directory.link_target(&name, budget)?;
                    let resolved = resolve_link(&path, &target)?;
                    Entry::Symlink {
                        path,
                        target,
                        resolved,
                    }
                }
                _ => return Err(CacheReason::UnsafePath),
            };
            entries.push(entry);
        }
        Ok(())
    }
    let mut entries = Vec::new();
    walk(directory, "", &mut entries, budget)?;
    validate_links(&entries)?;
    Ok(entries)
}

fn resolve_link(path: &str, target: &str) -> Result<String, CacheReason> {
    if target.starts_with('/') || target.len() > MAX_PATH || target.contains('\\') {
        return Err(CacheReason::UnsafePath);
    }
    let mut parts: Vec<_> = path.split('/').collect();
    parts.pop();
    for part in target.split('/') {
        match part {
            ".." => {
                parts.pop().ok_or(CacheReason::UnsafePath)?;
            }
            "." => {}
            part => {
                name_c(part)?;
                parts.push(part);
            }
        }
    }
    let resolved = parts.join("/");
    relative_parts(&resolved)?;
    if parts.iter().any(|part| excluded(part))
        || parts.first() == Some(&"node_modules")
        || resolved == OWNER_NAME
    {
        return Err(CacheReason::UnsafePath);
    }
    Ok(resolved)
}

fn validate_links(entries: &[Entry]) -> Result<(), CacheReason> {
    let by_path: BTreeMap<_, _> = entries.iter().map(|entry| (entry.path(), entry)).collect();
    for entry in entries {
        if let Entry::Symlink {
            path,
            target,
            resolved,
        } = entry
        {
            let mut parts: Vec<_> = path.split('/').collect();
            parts.pop();
            let targets: Vec<_> = target.split('/').collect();
            for (index, part) in targets.iter().enumerate() {
                match *part {
                    ".." => {
                        parts.pop().ok_or(CacheReason::UnsafePath)?;
                    }
                    "." => {}
                    part => parts.push(part),
                }
                if index + 1 < targets.len()
                    && !parts.is_empty()
                    && !matches!(
                        by_path.get(parts.join("/").as_str()),
                        Some(Entry::Directory { .. })
                    )
                {
                    return Err(CacheReason::UnsafePath);
                }
            }
            if !matches!(by_path.get(resolved.as_str()), Some(Entry::File { .. })) {
                return Err(CacheReason::UnsafePath);
            }
        }
    }
    Ok(())
}

fn validate_environment(environment: &Environment) -> Result<(), CacheReason> {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        _ => return Err(CacheReason::UnsupportedPlatform),
    };
    let libc = if cfg!(target_env = "musl") {
        "musl"
    } else if cfg!(target_env = "gnu") {
        "glibc"
    } else {
        return Err(CacheReason::UnsupportedPlatform);
    };
    for value in [
        &environment.node,
        &environment.node_abi,
        &environment.platform,
        &environment.arch,
        &environment.libc,
        &environment.npm,
        &environment.install_policy,
    ] {
        if value.is_empty()
            || value.len() > 1024
            || !value.is_ascii()
            || value.bytes().any(|b| b.is_ascii_control())
        {
            return Err(CacheReason::InputMismatch);
        }
    }
    if environment.platform != "linux" || environment.arch != arch || environment.libc != libc {
        return Err(CacheReason::InputMismatch);
    }
    Ok(())
}

struct TrustedEntry {
    controller_bytes: Vec<u8>,
    controller_directory: Directory,
    store: Directory,
    source: Directory,
    receipt: SourceReceipt,
    receipt_sha256: String,
}

impl TrustedEntry {
    fn load(
        controller_directory: Directory,
        store: Directory,
        package: &str,
        budget: &mut Budget,
    ) -> Result<Self, CacheReason> {
        budget.check()?;
        let controller_bytes =
            controller_directory.read("controller.json", MAX_JSON_BYTES, budget)?;
        let controller: Controller = serde_json::from_slice(&controller_bytes)
            .map_err(|_| CacheReason::UntrustedController)?;
        if controller.schema != 1
            || controller.kind != "refact-worktree-cache-controller"
            || controller.packages.len() > 16
        {
            return Err(CacheReason::UntrustedController);
        }
        validate_environment(&controller.environment)?;
        let matching: Vec<_> = controller
            .packages
            .iter()
            .filter(|pin| pin.package == package)
            .collect();
        if matching.is_empty() {
            return Err(CacheReason::NoTrustedEntry);
        }
        if matching.len() != 1 {
            return Err(CacheReason::UntrustedController);
        }
        let pin = matching[0];
        if !is_digest(&pin.entry) || !is_digest(&pin.receipt_sha256) {
            return Err(CacheReason::UntrustedController);
        }
        let source = store.child(&pin.entry)?;
        let bytes = source.read("producer.json", MAX_JSON_BYTES, budget)?;
        if digest(&bytes) != pin.receipt_sha256 {
            return Err(CacheReason::UntrustedProducer);
        }
        let receipt: SourceReceipt =
            serde_json::from_slice(&bytes).map_err(|_| CacheReason::UntrustedProducer)?;
        if receipt.schema != 1
            || receipt.kind != "refact-managed-dependency-install"
            || receipt.producer.is_empty()
            || receipt.producer.len() > 256
            || receipt.install_id.is_empty()
            || receipt.install_id.len() > 256
            || Path::new(&receipt.source) != source.path()
            || receipt.inputs.environment != controller.environment
            || !is_digest(&receipt.key)
            || !is_digest(&receipt.tree_sha256)
        {
            return Err(CacheReason::UntrustedProducer);
        }
        let current = inputs(&source, &controller.environment, budget)?;
        if current != receipt.inputs || digest(&encoded(&current)?) != receipt.key {
            return Err(CacheReason::InputMismatch);
        }
        Ok(Self {
            controller_bytes,
            controller_directory,
            store,
            source,
            receipt,
            receipt_sha256: pin.receipt_sha256.clone(),
        })
    }

    fn revalidate(&self, budget: &mut Budget) -> Result<(), CacheReason> {
        budget.check()?;
        #[cfg(not(test))]
        if !self.store.protected || !self.controller_directory.protected {
            return Err(CacheReason::UnverifiedIsolation);
        }
        if self.store.protected {
            host_boundary(budget)?;
        }
        self.controller_directory.validate_path()?;
        self.store.validate_path()?;
        self.source.validate_path()?;
        if self
            .controller_directory
            .read("controller.json", MAX_JSON_BYTES, budget)?
            != self.controller_bytes
            || digest(&self.source.read("producer.json", MAX_JSON_BYTES, budget)?)
                != self.receipt_sha256
        {
            return Err(CacheReason::UntrustedController);
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct TargetOwner {
    schema: u32,
    checkout_id: String,
    device: u64,
    inode: u64,
}

fn target_owner(context: &CacheExecutionContext) -> Result<TargetOwner, CacheReason> {
    let metadata = context
        .checkout
        .file
        .metadata()
        .map_err(|_| CacheReason::IoFailure)?;
    Ok(TargetOwner {
        schema: 1,
        checkout_id: context.meta.id.clone(),
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

pub(super) fn validate_local_target(
    context: &CacheExecutionContext,
    budget: &mut Budget,
) -> Result<(), CacheReason> {
    budget.check()?;
    let engine = context.checkout.relative("refact-agent/engine")?;
    if !engine.exists("target")? {
        return Ok(());
    }
    let target = engine
        .child("target")
        .map_err(|_| CacheReason::ExistingUnownedOutput)?;
    let bytes = target
        .read_optional(OWNER_NAME, 4096, budget)?
        .ok_or(CacheReason::ExistingUnownedOutput)?;
    let actual: TargetOwner =
        serde_json::from_slice(&bytes).map_err(|_| CacheReason::ExistingUnownedOutput)?;
    if actual != target_owner(context)? {
        return Err(CacheReason::ExistingUnownedOutput);
    }
    Ok(())
}

fn prepare_local_target(
    context: &CacheExecutionContext,
    budget: &mut Budget,
) -> Result<CacheOutcome, CacheReason> {
    budget.check()?;
    let engine = context.checkout.relative("refact-agent/engine")?;
    if engine.exists("target")? {
        validate_local_target(context, budget)?;
        return Ok(CacheOutcome::Reused);
    }
    let name = format!(".refact-target-{}", uuid::Uuid::new_v4());
    let temporary = engine.mkdir(&name)?;
    budget.check()?;
    let mut marker = temporary.create_file(OWNER_NAME, false)?;
    marker
        .write_all(&encoded(&target_owner(context)?)?)
        .map_err(|_| CacheReason::IoFailure)?;
    marker.sync_all().map_err(|_| CacheReason::IoFailure)?;
    temporary
        .file
        .sync_all()
        .map_err(|_| CacheReason::IoFailure)?;
    context.validate_current_with_budget(budget)?;
    engine.validate_path()?;
    temporary.validate_path()?;
    budget.check()?;
    engine.publish(&name, "target")?;
    Ok(CacheOutcome::Applied)
}

pub(super) fn prepare(
    context: &CacheExecutionContext,
    budget: &mut Budget,
) -> Result<CachePreparationReport, CacheReason> {
    budget.check()?;
    context.validate_current_with_budget(budget)?;
    let mut report = degraded(CacheReason::NoTrustedEntry);
    report.cargo_target = match prepare_local_target(context, budget) {
        Ok(outcome) => component(outcome, CacheReason::CheckoutLocal, None),
        Err(reason) => component(CacheOutcome::Degraded, reason, None),
    };
    let package = match context.checkout.relative("refact-agent/gui") {
        Ok(package) => package,
        Err(_) => {
            report.dependencies = component(CacheOutcome::Miss, CacheReason::PackageAbsent, None);
            return Ok(report);
        }
    };
    if package.exists("node_modules")? {
        let owned = package
            .child("node_modules")
            .and_then(|modules| modules.read_optional(OWNER_NAME, 4096, budget));
        if let Err(reason @ (CacheReason::Cancelled | CacheReason::LimitExceeded)) = owned {
            return Err(reason);
        }
        if !matches!(owned, Ok(Some(_))) {
            report.dependencies = component(
                CacheOutcome::Degraded,
                CacheReason::ExistingUnownedOutput,
                None,
            );
            return Ok(report);
        }
    }
    let result = (|| {
        let pin = context.controller_pin.as_ref().map_err(|reason| *reason)?;
        host_boundary(budget)?;
        let etc = Directory::absolute(Path::new("/etc"), true)
            .map_err(|_| CacheReason::UntrustedController)?;
        if !etc.exists("refact")? {
            return Err(CacheReason::NoTrustedEntry);
        }
        let parent = etc.child("refact")?;
        if !parent.exists("worktree-cache")? {
            return Err(CacheReason::NoTrustedEntry);
        }
        let controller = parent.child("worktree-cache")?;
        if !controller.exists("controller.json")? {
            return Err(CacheReason::NoTrustedEntry);
        }
        let store = Directory::absolute(Path::new(STORE), true)?;
        let entry = TrustedEntry::load(controller, store, "refact-agent/gui", budget)?;
        if digest(&entry.controller_bytes) != *pin {
            return Err(CacheReason::UntrustedController);
        }
        entry.revalidate(budget)?;
        let result = materialize(context, &package, &entry, budget)?;
        Ok::<_, CacheReason>((result, entry.receipt.key))
    })();
    match result {
        Ok((outcome, key)) => {
            report.trust = CacheTrustBoundary::RootProtectedHostStore;
            let reason = if outcome == CacheOutcome::Reused {
                CacheReason::VerifiedExisting
            } else {
                CacheReason::VerifiedCopy
            };
            report.dependencies = component(outcome, reason, Some(key));
        }
        Err(reason) => {
            let outcome = if matches!(
                reason,
                CacheReason::NoTrustedEntry | CacheReason::InputMismatch
            ) {
                CacheOutcome::Miss
            } else {
                CacheOutcome::Degraded
            };
            report.dependencies = component(outcome, reason, None);
        }
    }
    Ok(report)
}

fn materialize(
    context: &CacheExecutionContext,
    package: &Directory,
    entry: &TrustedEntry,
    budget: &mut Budget,
) -> Result<CacheOutcome, CacheReason> {
    context.validate_current_with_budget(budget)?;
    package.validate_path()?;
    entry.revalidate(budget)?;
    let target_inputs = inputs(package, &entry.receipt.inputs.environment, budget)?;
    if target_inputs != entry.receipt.inputs {
        return Err(CacheReason::InputMismatch);
    }
    let source_modules = entry.source.child("node_modules")?;
    let entries = inventory(&source_modules, budget)?;
    if entries.is_empty() || digest(&encoded(&entries)?) != entry.receipt.tree_sha256 {
        return Err(CacheReason::CorruptBlob);
    }
    let metadata = context
        .checkout
        .file
        .metadata()
        .map_err(|_| CacheReason::IoFailure)?;
    let owner = Owner {
        schema: 1,
        checkout_id: context.meta.id.clone(),
        checkout_device: metadata.dev(),
        checkout_inode: metadata.ino(),
        head: context.head.clone(),
        package: "refact-agent/gui".into(),
        source_receipt_sha256: entry.receipt_sha256.clone(),
        key: entry.receipt.key.clone(),
        tree_sha256: entry.receipt.tree_sha256.clone(),
    };
    if package.exists("node_modules")? {
        let existing = package
            .child("node_modules")
            .map_err(|_| CacheReason::ExistingUnownedOutput)?;
        let bytes = existing
            .read_optional(OWNER_NAME, 4096, budget)?
            .ok_or(CacheReason::ExistingUnownedOutput)?;
        let actual: Owner =
            serde_json::from_slice(&bytes).map_err(|_| CacheReason::ExistingUnownedOutput)?;
        if actual != owner || inventory(&existing, budget)? != entries {
            return Err(CacheReason::ExistingUnownedOutput);
        }
        context.validate_current_with_budget(budget)?;
        package.validate_path()?;
        entry.revalidate(budget)?;
        return Ok(CacheOutcome::Reused);
    }
    let temporary_name = format!(".refact-deps-{}", uuid::Uuid::new_v4());
    let temporary = package.mkdir(&temporary_name)?;
    budget.check()?;
    for entry in &entries {
        budget.check()?;
        let (parent, name) = temporary.parent(entry.path())?;
        match entry {
            Entry::Directory {
                ..
            } => {
                parent.mkdir(&name)?;
            }
            Entry::File {
                path,
                sha256,
                executable,
            } => {
                let (source_parent, source_name) = source_modules.parent(path)?;
                let mut target = parent.create_file(&name, *executable)?;
                if file_digest(
                    source_parent.open_file(&source_name)?,
                    Some(&mut target),
                    budget,
                )? != *sha256
                {
                    return Err(CacheReason::CorruptBlob);
                }
                target.sync_all().map_err(|_| CacheReason::IoFailure)?;
            }
            Entry::Symlink {
                ..
            } => {}
        }
    }
    for entry in &entries {
        budget.check()?;
        if let Entry::Symlink {
            path,
            target,
            ..
        } = entry
        {
            let (parent, name) = temporary.parent(path)?;
            parent.symlink(&name, target)?;
        }
    }
    if inventory(&temporary, budget)? != entries
        || inventory(&source_modules, budget)? != entries
        || inputs(package, &entry.receipt.inputs.environment, budget)? != target_inputs
        || inputs(&entry.source, &entry.receipt.inputs.environment, budget)? != target_inputs
    {
        return Err(CacheReason::CorruptBlob);
    }
    let mut marker = temporary.create_file(OWNER_NAME, false)?;
    marker
        .write_all(&encoded(&owner)?)
        .map_err(|_| CacheReason::IoFailure)?;
    marker.sync_all().map_err(|_| CacheReason::IoFailure)?;
    temporary
        .file
        .sync_all()
        .map_err(|_| CacheReason::IoFailure)?;
    budget.check()?;
    context.validate_current_with_budget(budget)?;
    package.validate_path()?;
    temporary.validate_path()?;
    entry.revalidate(budget)?;
    package.publish(&temporary_name, "node_modules")?;
    Ok(CacheOutcome::Applied)
}

pub(super) fn write_report(
    context: &CacheExecutionContext,
    report: &CachePreparationReport,
    budget: &mut Budget,
) -> Result<(), CacheReason> {
    context.validate_current_with_budget(budget)?;
    let name = format!(".refact-cache-report-{}", uuid::Uuid::new_v4());
    let mut file = context.gitdir.create_file(&name, false)?;
    file.write_all(&encoded(report)?)
        .map_err(|_| CacheReason::IoFailure)?;
    file.sync_all().map_err(|_| CacheReason::IoFailure)?;
    context.gitdir.publish(&name, REPORT_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::WorktreeService;
    use crate::types::CreateWorktreeRequest;
    use std::process::Command;

    struct Fixture {
        _temporary: tempfile::TempDir,
        context: CacheExecutionContext,
        package: Directory,
        controller: PathBuf,
        store: PathBuf,
        source: PathBuf,
        pin: String,
    }

    fn git(root: &Path, arguments: &[&str]) {
        let output = Command::new("git")
            .arg("-c")
            .arg("user.name=Cache fixture")
            .arg("-c")
            .arg("user.email=cache-fixture@example.invalid")
            .args(arguments)
            .current_dir(root)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn budget() -> Budget {
        Budget::new(Arc::new(AtomicBool::new(false)))
    }

    impl Fixture {
        async fn new() -> Self {
            let temporary = tempfile::tempdir().unwrap();
            let root = dunce::canonicalize(temporary.path()).unwrap();
            let repository = root.join("repo");
            std::fs::create_dir_all(repository.join("refact-agent/engine")).unwrap();
            std::fs::write(
                repository.join("refact-agent/engine/Cargo.toml"),
                "[workspace]\n",
            )
            .unwrap();
            git(&repository, &["init"]);
            git(&repository, &["checkout", "-b", "main"]);
            git(&repository, &["add", "."]);
            git(&repository, &["commit", "-m", "fixture"]);
            let service = WorktreeService::new(root.join("cache"), repository).unwrap();
            let created = service
                .create_worktree(CreateWorktreeRequest {
                    branch: Some("refact/chat/cache-fixture".into()),
                    ..Default::default()
                })
                .await
                .unwrap();
            let context = service
                .cache_execution_context(&created.worktree.meta)
                .await
                .unwrap();
            let package_path = context.meta.root.join("refact-agent/gui");
            std::fs::create_dir(&package_path).unwrap();
            std::fs::write(
                package_path.join("package.json"),
                b"{\"name\":\"fixture\",\"version\":\"1\"}\n",
            )
            .unwrap();
            std::fs::write(package_path.join("package-lock.json"), b"{\"lockfileVersion\":3,\"packages\":{\"\":{\"name\":\"fixture\",\"version\":\"1\"}}}\n").unwrap();
            let package = context.checkout.relative("refact-agent/gui").unwrap();
            let store = root.join("store");
            let source = store.join("a".repeat(64));
            std::fs::create_dir_all(source.join("node_modules/pkg")).unwrap();
            std::fs::create_dir_all(source.join("node_modules/.bin")).unwrap();
            std::fs::copy(
                package.path.join("package.json"),
                source.join("package.json"),
            )
            .unwrap();
            std::fs::copy(
                package.path.join("package-lock.json"),
                source.join("package-lock.json"),
            )
            .unwrap();
            std::fs::write(
                source.join("node_modules/pkg/tool.js"),
                b"fixture payload\n",
            )
            .unwrap();
            std::os::unix::fs::symlink("../pkg/tool.js", source.join("node_modules/.bin/tool"))
                .unwrap();
            let source_directory = Directory::absolute(&source, false).unwrap();
            let environment = Environment {
                node: "fixture-node-22".into(),
                node_abi: "fixture-127".into(),
                platform: "linux".into(),
                arch: match std::env::consts::ARCH {
                    "x86_64" => "x64",
                    "aarch64" => "arm64",
                    _ => "unsupported",
                }
                .into(),
                libc: if cfg!(target_env = "musl") {
                    "musl"
                } else {
                    "glibc"
                }
                .into(),
                npm: "fixture-npm-10".into(),
                install_policy: "offline-fixture-no-scripts".into(),
            };
            let input = inputs(&source_directory, &environment, &mut budget()).unwrap();
            let key = digest(&encoded(&input).unwrap());
            let tree = inventory(
                &source_directory.child("node_modules").unwrap(),
                &mut budget(),
            )
            .unwrap();
            let receipt = SourceReceipt {
                schema: 1,
                kind: "refact-managed-dependency-install".into(),
                producer: "fixture-producer".into(),
                install_id: "fixture-install".into(),
                source: source.to_str().unwrap().into(),
                inputs: input,
                key,
                tree_sha256: digest(&encoded(&tree).unwrap()),
            };
            let receipt = encoded(&receipt).unwrap();
            let pin = digest(&receipt);
            std::fs::write(source.join("producer.json"), receipt).unwrap();
            let controller = root.join("controller");
            std::fs::create_dir(&controller).unwrap();
            let manifest = Controller {
                schema: 1,
                kind: "refact-worktree-cache-controller".into(),
                environment,
                packages: vec![PackagePin {
                    package: "refact-agent/gui".into(),
                    entry: "a".repeat(64),
                    receipt_sha256: pin.clone(),
                }],
            };
            std::fs::write(
                controller.join("controller.json"),
                encoded(&manifest).unwrap(),
            )
            .unwrap();
            Self {
                _temporary: temporary,
                context,
                package,
                controller,
                store,
                source,
                pin,
            }
        }

        fn entry(&self, budget: &mut Budget) -> Result<TrustedEntry, CacheReason> {
            TrustedEntry::load(
                Directory::absolute(&self.controller, false)?,
                Directory::absolute(&self.store, false)?,
                "refact-agent/gui",
                budget,
            )
        }

        fn apply(&self) -> Result<CacheOutcome, CacheReason> {
            let mut budget = budget();
            let entry = self.entry(&mut budget)?;
            materialize(&self.context, &self.package, &entry, &mut budget)
        }
    }

    #[tokio::test]
    async fn cache_reuse_verified_copy_is_private_and_owned_repeat_is_read_only() {
        let fixture = Fixture::new().await;
        let entry = fixture.entry(&mut budget()).unwrap();
        assert_eq!(entry.receipt_sha256, fixture.pin);
        assert_eq!(fixture.apply(), Ok(CacheOutcome::Applied));
        let output = fixture.package.path.join("node_modules/pkg/tool.js");
        assert_eq!(std::fs::read(&output).unwrap(), b"fixture payload\n");
        assert_ne!(
            std::fs::metadata(&output).unwrap().ino(),
            std::fs::metadata(fixture.source.join("node_modules/pkg/tool.js"))
                .unwrap()
                .ino()
        );
        assert_eq!(
            std::fs::read_link(fixture.package.path.join("node_modules/.bin/tool")).unwrap(),
            PathBuf::from("../pkg/tool.js")
        );
        assert_eq!(fixture.apply(), Ok(CacheOutcome::Reused));
        std::fs::write(&output, b"private mutation\n").unwrap();
        assert_eq!(
            std::fs::read(fixture.source.join("node_modules/pkg/tool.js")).unwrap(),
            b"fixture payload\n"
        );
        assert_eq!(fixture.apply(), Err(CacheReason::ExistingUnownedOutput));
        assert_eq!(std::fs::read(&output).unwrap(), b"private mutation\n");
    }

    #[tokio::test]
    async fn cache_reuse_input_mismatch_does_not_publish() {
        let fixture = Fixture::new().await;
        std::fs::write(
            fixture.package.path.join("package.json"),
            b"{\"name\":\"mismatch\",\"version\":\"1\"}\n",
        )
        .unwrap();
        std::fs::write(fixture.package.path.join("package-lock.json"), b"{\"lockfileVersion\":3,\"packages\":{\"\":{\"name\":\"mismatch\",\"version\":\"1\"}}}\n").unwrap();
        assert_eq!(fixture.apply(), Err(CacheReason::InputMismatch));
        assert!(!fixture.package.exists("node_modules").unwrap());
    }

    #[tokio::test]
    async fn cache_reuse_corrupt_source_does_not_publish() {
        let fixture = Fixture::new().await;
        std::fs::write(
            fixture.source.join("node_modules/pkg/tool.js"),
            b"corrupt\n",
        )
        .unwrap();
        assert_eq!(fixture.apply(), Err(CacheReason::CorruptBlob));
        assert!(!fixture.package.exists("node_modules").unwrap());
        assert_eq!(
            digest(&std::fs::read(fixture.source.join("producer.json")).unwrap()),
            fixture.pin
        );
    }

    #[tokio::test]
    async fn cache_reuse_escaping_source_link_does_not_publish() {
        let fixture = Fixture::new().await;
        let outside = fixture.source.join("outside");
        std::fs::write(&outside, b"outside fixture\n").unwrap();
        let link = fixture.source.join("node_modules/.bin/tool");
        std::fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink("../../outside", &link).unwrap();
        assert!(outside.is_file());
        assert_eq!(fixture.apply(), Err(CacheReason::UnsafePath));
        assert!(!fixture.package.exists("node_modules").unwrap());
        assert_eq!(std::fs::read(outside).unwrap(), b"outside fixture\n");
    }

    #[tokio::test]
    async fn cache_reuse_publication_never_replaces_destination() {
        let fixture = Fixture::new().await;
        let output = fixture.package.path.join("node_modules");
        let outside = fixture.source.join("node_modules");
        std::os::unix::fs::symlink(&outside, &output).unwrap();
        assert_eq!(fixture.apply(), Err(CacheReason::ExistingUnownedOutput));
        assert_eq!(std::fs::read_link(&output).unwrap(), outside);
        let temporary = fixture.package.mkdir("publication-candidate").unwrap();
        temporary
            .create_file("payload", false)
            .unwrap()
            .write_all(b"candidate")
            .unwrap();
        assert_eq!(
            fixture
                .package
                .publish("publication-candidate", "node_modules"),
            Err(CacheReason::ExistingUnownedOutput)
        );
        assert_eq!(std::fs::read_link(output).unwrap(), outside);
        assert_eq!(
            temporary.read("payload", 128, &mut budget()).unwrap(),
            b"candidate"
        );
    }

    #[tokio::test]
    async fn cache_reuse_changed_producer_pin_and_revoked_checkout_refuse() {
        let fixture = Fixture::new().await;
        let controller = std::fs::read(fixture.controller.join("controller.json")).unwrap();
        let producer = std::fs::read(fixture.source.join("producer.json")).unwrap();
        assert!(!controller.is_empty());
        assert!(!producer.is_empty());
        let metadata_bytes = (controller.len() + producer.len()) as u64;
        let input_bytes = std::fs::metadata(fixture.source.join("package.json"))
            .unwrap()
            .len()
            + std::fs::metadata(fixture.source.join("package-lock.json"))
                .unwrap()
                .len();
        let mut bounded = Budget::with_byte_limit(
            Arc::new(AtomicBool::new(false)),
            input_bytes + metadata_bytes * 3 - 1,
        );
        let entry = fixture.entry(&mut bounded).unwrap();
        assert_eq!(bounded.bytes, input_bytes + metadata_bytes);
        assert_eq!(entry.revalidate(&mut bounded), Ok(()));
        assert_eq!(bounded.bytes, input_bytes + metadata_bytes * 2);
        assert_eq!(
            entry.revalidate(&mut bounded),
            Err(CacheReason::LimitExceeded)
        );
        assert!(!fixture.package.exists("node_modules").unwrap());
        std::fs::write(fixture.source.join("producer.json"), b"{}\n").unwrap();
        assert_eq!(
            fixture.entry(&mut budget()).err(),
            Some(CacheReason::UntrustedProducer)
        );
        assert_eq!(
            materialize(&fixture.context, &fixture.package, &entry, &mut budget()),
            Err(CacheReason::UntrustedController)
        );
        let data = std::fs::read(&fixture.context.registry).unwrap();
        let mut registry: WorktreeRegistry = serde_json::from_slice(&data).unwrap();
        assert_eq!(registry.records.len(), 1);
        registry.records.clear();
        std::fs::write(
            &fixture.context.registry,
            serde_json::to_vec(&registry).unwrap(),
        )
        .unwrap();
        assert_eq!(
            fixture.context.validate_current(),
            Err(CacheReason::IdentityChanged)
        );
        assert!(!fixture.package.exists("node_modules").unwrap());
    }

    #[tokio::test]
    async fn cache_reuse_cancelled_materialization_does_not_publish() {
        let fixture = Fixture::new().await;
        let entry = fixture.entry(&mut budget()).unwrap();
        let cancelled = PreparationCancellation::new();
        let flag = cancelled.flag();
        drop(cancelled);
        assert_eq!(
            materialize(
                &fixture.context,
                &fixture.package,
                &entry,
                &mut Budget::new(flag)
            ),
            Err(CacheReason::Cancelled)
        );
        assert!(!fixture.package.exists("node_modules").unwrap());
        assert_eq!(
            std::fs::read(fixture.source.join("node_modules/pkg/tool.js")).unwrap(),
            b"fixture payload\n"
        );
    }
}
