use crate::{
    kernel::{exception, HelperCall, HelperError, Mark, OsBackend},
    protocol::{FileListRequest, KernelEvent, WriteOriginal, PathDenyRules},
};
use monty_types::{
    *,
    scan::{collect_scan, EntryInfo, Listing, ScanSource},
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    fs,
    io::{self, BufRead, Read, Write},
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use chrono::{Datelike, Timelike};
use rand::RngCore;

#[derive(Debug, Clone, Copy)]
pub enum Platform {
    Unix,
    Windows,
}
fn platform() -> Platform {
    if cfg!(windows) {
        Platform::Windows
    } else {
        Platform::Unix
    }
}
pub fn host_path(path: &str, platform: Platform) -> PathBuf {
    match platform {
        Platform::Unix => PathBuf::from(path),
        Platform::Windows => {
            let p = path
                .strip_prefix('/')
                .filter(|s| s.as_bytes().get(1) == Some(&b':'))
                .unwrap_or(path);
            PathBuf::from(p.replace('/', "\\"))
        }
    }
}
pub fn virtual_path(path: &Path, platform: Platform) -> String {
    let p = path.to_string_lossy();
    match platform {
        Platform::Unix => p.into_owned(),
        Platform::Windows => {
            let p = match p.strip_prefix(r"\\?\UNC\") {
                Some(rest) => format!(r"\\{rest}"),
                None => match p.strip_prefix(r"\\?\") {
                    Some(rest) if rest.as_bytes().get(1) == Some(&b':') => rest.to_string(),
                    _ => p.into_owned(),
                },
            };
            let p = p.replace('\\', "/");
            if p.as_bytes().get(1) == Some(&b':') {
                format!("/{p}")
            } else {
                p
            }
        }
    }
}
#[derive(Clone, Debug)]
pub struct HostFsConfig {
    pub deny_write: PathDenyRules,
    pub deny_read: PathDenyRules,
    pub project_root: PathBuf,
    pub env: BTreeMap<String, String>,
    pub capture_dir: Option<PathBuf>,
    pub max_capture_file_bytes: u64,
    pub max_capture_run_bytes: u64,
    /// Bytes one `grep()` call reads before it stops early.
    pub max_scan_bytes: u64,
    /// Runs inside `replace()` between staging the new text and checking the file again.
    pub replace_hook: Option<fn(&Path)>,
    /// Runs between the check of a write inside an allow prefix and opening the file.
    pub write_hook: Option<fn(&Path)>,
    /// Path rules ignore letter case, as the usual volumes of macOS and Windows do.
    pub fold_case: bool,
}
impl Default for HostFsConfig {
    fn default() -> Self {
        Self {
            project_root: std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
            env: BTreeMap::new(),
            deny_write: PathDenyRules::default(),
            deny_read: PathDenyRules::default(),
            capture_dir: None,
            max_capture_file_bytes: 2 * 1024 * 1024,
            max_capture_run_bytes: 32 * 1024 * 1024,
            max_scan_bytes: 256 * 1024 * 1024,
            replace_hook: None,
            write_hook: None,
            fold_case: cfg!(windows) || cfg!(target_os = "macos"),
        }
    }
}
pub struct HostFs {
    config: HostFsConfig,
    events: Vec<KernelEvent>,
    captured: HashSet<PathBuf>,
    capture_bytes: u64,
    clock: Instant,
}
impl Default for HostFs {
    fn default() -> Self {
        Self::new(HostFsConfig::default())
    }
}
type Answer = Result<MontyObject, (String, String)>;
#[derive(Clone, Copy, PartialEq, Eq)]
enum Access {
    Read,
    Content,
    Create,
    Remove,
}
fn reject_dir(p: &Path) -> io::Result<()> {
    if p.is_dir() {
        return Err(io::Error::from(io::ErrorKind::IsADirectory));
    }
    Ok(())
}
fn error(e: io::Error) -> (String, String) {
    let kind = match e.kind() {
        io::ErrorKind::NotFound => "FileNotFoundError",
        io::ErrorKind::PermissionDenied => "PermissionError",
        io::ErrorKind::IsADirectory => "IsADirectoryError",
        io::ErrorKind::NotADirectory => "NotADirectoryError",
        io::ErrorKind::AlreadyExists => "FileExistsError",
        _ => "OSError",
    };
    (kind.into(), e.to_string())
}
fn denied_path(p: &Path, write: bool) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        if write {
            format!(
                "engine configuration or state: {}; use the file-edit tools to change it",
                p.display()
            )
        } else {
            format!(
                "reading {} is not allowed (engine configuration)",
                p.display()
            )
        },
    )
}
impl HostFs {
    pub fn new(mut config: HostFsConfig) -> Self {
        if !config.project_root.is_absolute() {
            config.project_root = std::env::current_dir()
                .unwrap_or_default()
                .join(&config.project_root);
        }
        if let Some(p) = &config.capture_dir {
            if !p.is_absolute() {
                config.capture_dir = Some(config.project_root.join(p));
            }
        }
        Self {
            config,
            events: vec![],
            captured: HashSet::new(),
            capture_bytes: 0,
            clock: Instant::now(),
        }
    }
    pub fn resolve(&self, p: &str) -> PathBuf {
        let p = host_path(p, platform());
        if p.is_absolute() {
            p
        } else {
            self.config.project_root.join(p)
        }
    }
    fn guard(&self, p: &Path, write: bool) -> io::Result<()> {
        self.check(p, if write { Access::Content } else { Access::Read })
            .map(drop)
    }
    /// Whether a script may read `p`: the check `open()` makes before reading.
    pub fn may_read(&self, p: &Path) -> bool {
        self.check(p, Access::Read).is_ok()
    }
    /// `Some(checked path)` when an allow prefix authorized the write.
    fn check(&self, p: &Path, access: Access) -> io::Result<Option<PathBuf>> {
        let write = access != Access::Read;
        if write {
            self.check_read_only(p, access)?;
            reject_dangling_symlinks(p)?;
        }
        self.judge(&self.rules(write), p, access)
    }
    /// The rules of reads or of writes with their paths resolved, for one call.
    fn rules(&self, write: bool) -> Rules {
        let config = if write {
            &self.config.deny_write
        } else {
            &self.config.deny_read
        };
        let resolve = |path: &PathBuf| self.resolve(&path.to_string_lossy());
        Rules {
            write,
            allow: config
                .allow_prefixes
                .iter()
                .map(|allowed| {
                    let allowed = resolve(allowed);
                    // A write exception authorizes contents, not replacing the root itself.
                    // Never let a missing or substituted symlink root widen the exception.
                    let usable = !write
                        || fs::symlink_metadata(&allowed)
                            .is_ok_and(|m| m.is_dir() && !m.is_symlink());
                    (lazy(resolved_path(&allowed)), usable)
                })
                .collect(),
            deny: config
                .prefixes
                .iter()
                .map(|p| (p, true))
                .chain(config.files.iter().map(|p| (p, false)))
                .map(|(path, prefix)| {
                    (
                        lazy(resolved_path(&resolve(path)).map(|p| self.comparable(p))),
                        prefix,
                    )
                })
                .collect(),
            files: config
                .files
                .iter()
                .map(|path| {
                    let denied: PathBuf = resolve(path).components().collect();
                    lazy(match fs::metadata(&denied) {
                        Ok(meta) if meta.is_file() => {
                            path_identity(&denied, &meta, true).map(|(id, _)| Some(id))
                        }
                        Ok(_) => Ok(None),
                        Err(e)
                            if matches!(
                                e.kind(),
                                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                            ) =>
                        {
                            Ok(None)
                        }
                        Err(e) => Err(e),
                    })
                })
                .collect(),
        }
    }
    fn judge(&self, rules: &Rules, p: &Path, access: Access) -> io::Result<Option<PathBuf>> {
        let write = rules.write;
        let entry = matches!(access, Access::Create | Access::Remove);
        let destructive = access == Access::Remove;
        if self.matches_denied_file(p, rules, !entry)? {
            return Err(denied_path(p, write));
        }
        let target = if entry {
            entry_path(p)?
        } else {
            resolved_path(p)?
        };
        let resolved = self.comparable(target.clone());
        for (root, usable) in &rules.allow {
            if destructive
                && root
                    .as_ref()
                    .is_ok_and(|root| self.comparable(root.clone()) == resolved)
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!(
                        "the directory {} itself cannot be renamed or removed; work with the files inside it",
                        p.display()
                    ),
                ));
            }
            if !usable {
                continue;
            }
            let root = root.as_ref().map_err(again)?;
            if target.starts_with(root) && (!write || &target != root) {
                if access == Access::Content {
                    match fs::metadata(p) {
                        Ok(meta) if !meta.is_dir() => {
                            self.check_contained_file(p, &meta, || {
                                path_identity(p, &meta, true).map(|(_, links)| links)
                            })?
                        }
                        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                        _ => {}
                    }
                }
                return Ok(Some(target));
            }
        }
        let mut judged = vec![resolved];
        if entry {
            if self.matches_denied_file(p, rules, true)? {
                return Err(denied_path(p, write));
            }
            judged.push(self.comparable(resolved_path(p)?));
        }
        for (denied, prefix) in &rules.deny {
            let denied = denied.as_ref().map_err(again)?;
            if judged.iter().any(|resolved| {
                resolved == denied
                    || (*prefix && resolved.starts_with(denied))
                    || (destructive && denied.starts_with(resolved))
            }) {
                return Err(denied_path(p, write));
            }
        }
        Ok(None)
    }
    fn comparable(&self, p: PathBuf) -> PathBuf {
        if self.config.fold_case {
            PathBuf::from(p.to_string_lossy().to_lowercase())
        } else {
            p
        }
    }
    fn check_read_only(&self, p: &Path, access: Access) -> io::Result<()> {
        if self.config.deny_write.read_only.is_none() {
            return Ok(());
        }
        if !entry_path(p).is_ok_and(|entry| self.inside_allowed(&entry)) {
            return Err(self.refusal(p, None));
        }
        if access == Access::Content && fs::symlink_metadata(p).is_ok_and(|m| m.is_symlink()) {
            return Err(self.refusal(p, Some("is a symlink")));
        }
        Ok(())
    }
    fn inside_allowed(&self, entry: &Path) -> bool {
        self.config.deny_write.allow_prefixes.iter().any(|allowed| {
            let allowed = self.resolve(&allowed.to_string_lossy());
            fs::symlink_metadata(&allowed).is_ok_and(|m| m.is_dir() && !m.is_symlink())
                && resolved_path(&allowed)
                    .is_ok_and(|allowed| entry.starts_with(&allowed) && entry != allowed)
        })
    }
    fn refusal(&self, p: &Path, detail: Option<&str>) -> io::Error {
        let detail = detail.map(|d| format!(" {d}")).unwrap_or_default();
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            match &self.config.deny_write.read_only {
                Some(reason) => format!("{reason}; refused: {}{detail}", p.display()),
                None => format!(
                    "refused: {}{detail}; inside an allowed directory a script writes only regular files with one link",
                    p.display()
                ),
            },
        )
    }
    fn check_contained_file(
        &self,
        p: &Path,
        meta: &fs::Metadata,
        links: impl FnOnce() -> io::Result<u64>,
    ) -> io::Result<()> {
        if !meta.is_file() {
            return Err(self.refusal(p, Some("is not a regular file")));
        }
        match links()? {
            1 => Ok(()),
            n => Err(self.refusal(p, Some(&format!("has {n} links")))),
        }
    }
    /// Opens the path `check` authorized, never through a link at its leaf.
    fn open_contained(
        &self,
        p: &Path,
        checked: &Path,
        options: &mut fs::OpenOptions,
        truncate: bool,
    ) -> io::Result<fs::File> {
        if let Some(hook) = self.config.write_hook {
            hook(checked);
        }
        no_follow(options);
        let file = options.open(checked).map_err(|e| {
            #[cfg(unix)]
            if e.raw_os_error() == Some(libc::ELOOP) {
                return self.refusal(p, Some("is a symlink"));
            }
            e
        })?;
        let meta = file.metadata()?;
        if meta.is_symlink() {
            return Err(self.refusal(p, Some("is a symlink")));
        }
        if meta.is_dir() {
            return Err(io::Error::from(io::ErrorKind::IsADirectory));
        }
        self.check_contained_file(p, &meta, || {
            opened_identity(&file, &meta).map(|(_, links)| links)
        })?;
        if truncate {
            file.set_len(0)?;
        }
        Ok(file)
    }
    fn matches_denied_file(&self, p: &Path, rules: &Rules, follow: bool) -> io::Result<bool> {
        if rules.files.is_empty() {
            return Ok(false);
        }
        let meta = if follow {
            fs::metadata(p)
        } else {
            fs::symlink_metadata(p)
        };
        let target = match meta {
            Ok(m) if m.is_file() => path_identity(p, &m, follow)?.0,
            Ok(_) => return Ok(false),
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                return Ok(false)
            }
            Err(e) => return Err(e),
        };
        for denied in &rules.files {
            match denied {
                Ok(Some(denied)) if *denied == target => return Ok(true),
                Ok(_) => {}
                Err(e) => return Err(again(e)),
            }
        }
        Ok(false)
    }
    fn record(&mut self, op: &str, p: &Path, bytes: u64) {
        self.events.push(KernelEvent::Fs {
            run_id: String::new(),
            op: op.into(),
            path: p.to_string_lossy().into_owned(),
            bytes,
        });
    }
    fn capture(&mut self, p: &Path) -> io::Result<()> {
        if self.captured.contains(p) {
            return Ok(());
        }
        let original = match fs::metadata(p) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => WriteOriginal::Missing,
            Err(e) => return Err(e),
            Ok(m) if !m.is_file() => return Ok(()),
            Ok(m) => {
                self.guard(p, false)?;
                if let Some(dir) = &self.config.capture_dir {
                    if m.len() > self.config.max_capture_file_bytes
                        || m.len()
                            > self
                                .config
                                .max_capture_run_bytes
                                .saturating_sub(self.capture_bytes)
                    {
                        WriteOriginal::TooLarge
                    } else {
                        let (target, mut output) = self.capture_file(dir)?;
                        let mut input = fs::File::open(p)?;
                        let n = io::copy(&mut (&mut input).take(m.len() + 1), &mut output)?;
                        if n > m.len() {
                            fs::remove_file(&target)?;
                            WriteOriginal::TooLarge
                        } else {
                            self.capture_bytes += n;
                            WriteOriginal::File {
                                capture_path: target.to_string_lossy().into_owned(),
                            }
                        }
                    }
                } else {
                    WriteOriginal::TooLarge
                }
            }
        };
        self.captured.insert(p.to_path_buf());
        self.events.push(KernelEvent::WriteCapture {
            run_id: String::new(),
            path: p.to_string_lossy().into_owned(),
            original,
        });
        Ok(())
    }
    fn capture_file(&self, dir: &Path) -> io::Result<(PathBuf, fs::File)> {
        fs::create_dir_all(dir)?;
        let mut n = self.captured.len();
        loop {
            let target = dir.join(n.to_string());
            match fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&target)
            {
                Ok(file) => return Ok((target, file)),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => n += 1,
                Err(e) => return Err(e),
            }
        }
    }
    /// The first-write capture of `p` from bytes already read, written but not yet recorded.
    fn prepare_capture(&mut self, p: &Path, original: &[u8]) -> io::Result<Option<Prepared>> {
        if self.captured.contains(p) {
            return Ok(None);
        }
        let size = original.len() as u64;
        let (original, charged) = match &self.config.capture_dir {
            Some(dir)
                if size <= self.config.max_capture_file_bytes
                    && size
                        <= self
                            .config
                            .max_capture_run_bytes
                            .saturating_sub(self.capture_bytes) =>
            {
                let (target, mut output) = self.capture_file(dir)?;
                if let Err(e) = output.write_all(original) {
                    drop(output);
                    let _ = fs::remove_file(&target);
                    return Err(e);
                }
                (
                    WriteOriginal::File {
                        capture_path: target.to_string_lossy().into_owned(),
                    },
                    size,
                )
            }
            _ => (WriteOriginal::TooLarge, 0),
        };
        Ok(Some(Prepared {
            path: p.to_path_buf(),
            original,
            charged,
        }))
    }
    fn commit_capture(&mut self, prepared: Option<Prepared>) {
        if let Some(prepared) = prepared {
            self.capture_bytes += prepared.charged;
            self.events.push(KernelEvent::WriteCapture {
                run_id: String::new(),
                path: prepared.path.to_string_lossy().into_owned(),
                original: prepared.original,
            });
            self.captured.insert(prepared.path);
        }
    }
    fn capture_entry(&mut self, p: &Path) -> io::Result<()> {
        if fs::symlink_metadata(p).is_ok_and(|m| m.is_symlink()) {
            return Ok(());
        }
        self.capture(p)
    }
    fn write(&mut self, p: &Path, data: &[u8], append: bool) -> io::Result<()> {
        let contained = self.check(p, Access::Content)?;
        self.capture(p)?;
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).append(append);
        let mut f = match contained {
            Some(checked) => self.open_contained(p, &checked, &mut options, !append)?,
            None => options.truncate(!append).open(p)?,
        };
        f.write_all(data)
    }
    fn dispatch(&mut self, call: OsFunctionCall) -> io::Result<MontyObject> {
        let op = call.name();
        let p = call.fs_primary_path().map(|p| self.resolve(p));
        let p = p.as_deref().unwrap_or(Path::new(""));
        let mut bytes = 0;
        let result = match call {
            OsFunctionCall::Exists(_) => MontyObject::bool(p.try_exists()?),
            OsFunctionCall::IsFile(_) => MontyObject::bool(p.is_file()),
            OsFunctionCall::IsDir(_) => MontyObject::bool(p.is_dir()),
            OsFunctionCall::IsSymlink(_) => MontyObject::bool(p.is_symlink()),
            OsFunctionCall::ReadText(_) => {
                self.guard(p, false)?;
                reject_dir(p)?;
                let s = fs::read_to_string(p)?;
                bytes = s.len() as u64;
                MontyObject::string(s)
            }
            OsFunctionCall::ReadBytes(_) => {
                self.guard(p, false)?;
                reject_dir(p)?;
                let b = fs::read(p)?;
                bytes = b.len() as u64;
                MontyObject::bytes(b)
            }
            OsFunctionCall::WriteText(a) | OsFunctionCall::AppendText(a) => {
                bytes = a.data.len() as u64;
                self.write(p, a.data.as_bytes(), op == "Path.append_text")?;
                MontyObject::int(a.data.chars().count() as i64)
            }
            OsFunctionCall::WriteBytes(a) | OsFunctionCall::AppendBytes(a) => {
                bytes = a.data.len() as u64;
                self.write(p, &a.data, op == "Path.append_bytes")?;
                MontyObject::int(bytes as i64)
            }
            OsFunctionCall::Stat(_) => {
                self.guard(p, false)?;
                stat(&fs::metadata(p)?)
            }
            OsFunctionCall::Iterdir(_) => {
                self.guard(p, false)?;
                if p.exists() && !p.is_dir() {
                    return Err(io::Error::from(io::ErrorKind::NotADirectory));
                }
                let entries = fs::read_dir(p)?
                    .map(|e| e.map(|e| MontyObject::path(virtual_path(&e.path(), platform()))))
                    .collect::<io::Result<Vec<_>>>()?;
                MontyObject::list(entries)
            }
            OsFunctionCall::Scan(args) => {
                let mut source = Source {
                    fs: self,
                    root: p.to_path_buf(),
                    visits: 0,
                    charged: 0,
                };
                collect_scan(&args, &mut source)?
                    .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?
                    .into_object()
            }
            OsFunctionCall::Resolve(_) => {
                MontyObject::path(virtual_path(&fs::canonicalize(p)?, platform()))
            }
            OsFunctionCall::Absolute(_) => MontyObject::path(virtual_path(p, platform())),
            OsFunctionCall::Open(a) => {
                if a.mode.readable() {
                    self.guard(p, false)?;
                }
                let contained = if a.mode.writable() || a.mode.create() {
                    self.check(p, Access::Content)?
                } else {
                    None
                };
                reject_dir(p)?;
                if a.mode.create() {
                    self.capture(p)?;
                }
                let mut options = fs::OpenOptions::new();
                options
                    .read(a.mode.readable())
                    .write(a.mode.writable())
                    .create(a.mode.create())
                    .append(a.mode.is_append());
                let f = match contained {
                    Some(checked) => {
                        self.open_contained(p, &checked, &mut options, a.mode.truncate())?
                    }
                    None => options.truncate(a.mode.truncate()).open(p)?,
                };
                if f.metadata()?.is_dir() {
                    return Err(io::Error::from(io::ErrorKind::IsADirectory));
                }
                MontyObject::file_handle(MontyFileHandle {
                    path: virtual_path(p, platform()),
                    mode: a.mode,
                    position: 0,
                })
            }
            OsFunctionCall::CreateExclusive(a) => {
                self.guard(p, true)?;
                a.validate()
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
                self.capture(p)?;
                let mut options = fs::OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(a.permissions);
                }
                options.open(p)?;
                MontyObject::file_handle(MontyFileHandle {
                    path: virtual_path(p, platform()),
                    mode: a.mode,
                    position: 0,
                })
            }
            OsFunctionCall::Mkdir(a) => {
                if p.is_dir() {
                    if !a.exist_ok {
                        return Err(io::Error::from(io::ErrorKind::AlreadyExists));
                    }
                    // A no-op mkdir writes nothing and is not reported as a write.
                    return Ok(MontyObject::none());
                } else {
                    self.check(p, Access::Create)?;
                    if a.parents {
                        if p.exists() {
                            return Err(io::Error::from(io::ErrorKind::AlreadyExists));
                        }
                        fs::create_dir_all(p)?;
                    } else {
                        match fs::create_dir(p) {
                            Err(e) if a.exist_ok && p.is_dir() => {
                                let _ = e;
                            }
                            r => r?,
                        }
                    }
                }
                MontyObject::none()
            }
            OsFunctionCall::Unlink(_) => {
                self.check(p, Access::Remove)?;
                self.capture_entry(p)?;
                fs::remove_file(p)?;
                MontyObject::none()
            }
            OsFunctionCall::Rmdir(_) => {
                self.check(p, Access::Remove)?;
                fs::remove_dir(p)?;
                MontyObject::none()
            }
            OsFunctionCall::Rename(a) => {
                self.check(p, Access::Remove)?;
                let dst = self.resolve(&a.dst);
                self.check(&dst, Access::Remove)?;
                self.capture_entry(p)?;
                self.capture_entry(&dst)?;
                fs::rename(p, &dst)?;
                self.events.push(KernelEvent::Renamed {
                    run_id: String::new(),
                    src: p.to_string_lossy().into_owned(),
                    dst: dst.to_string_lossy().into_owned(),
                });
                self.record(op, &dst, 0);
                MontyObject::path(virtual_path(&dst, platform()))
            }
            OsFunctionCall::Getenv(a) => self
                .config
                .env
                .get(&a.key)
                .map(MontyObject::string)
                .unwrap_or(a.default),
            OsFunctionCall::GetEnviron => MontyObject::dict(
                self.config
                    .env
                    .iter()
                    .map(|(k, v)| (MontyObject::string(k), MontyObject::string(v))),
            ),
            OsFunctionCall::Sleep(d)
            | OsFunctionCall::SystemSleep(d)
            | OsFunctionCall::AsyncSleep(d)
            | OsFunctionCall::AsyncSystemSleep(d) => {
                std::thread::sleep(d);
                MontyObject::none()
            }
            OsFunctionCall::Time(caller) => {
                let seconds = if matches!(
                    caller,
                    TimeCaller::Monotonic
                        | TimeCaller::MonotonicNs
                        | TimeCaller::PerfCounter
                        | TimeCaller::PerfCounterNs
                ) {
                    self.clock.elapsed().as_secs_f64()
                } else {
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs_f64()
                };
                MontyObject::float(seconds)
            }
            OsFunctionCall::DateToday => {
                let d = chrono::Local::now();
                MontyObject::date(MontyDate {
                    year: d.year(),
                    month: d.month() as u8,
                    day: d.day() as u8,
                })
            }
            OsFunctionCall::DateTimeNow(tz) => {
                let now = chrono::Utc::now();
                let d = if let Some(t) = &tz {
                    now.naive_utc() + chrono::Duration::seconds(t.offset_seconds as i64)
                } else {
                    chrono::Local::now().naive_local()
                };
                MontyObject::datetime(MontyDateTime {
                    year: d.year(),
                    month: d.month() as u8,
                    day: d.day() as u8,
                    hour: d.hour() as u8,
                    minute: d.minute() as u8,
                    second: d.second() as u8,
                    microsecond: d.nanosecond() / 1000,
                    offset_seconds: tz.as_ref().map(|t| t.offset_seconds),
                    timezone_name: tz.and_then(|t| t.name),
                })
            }
            OsFunctionCall::Urandom(a) => {
                if a.size > 32 * 1024 * 1024 {
                    return Err(io::Error::other("entropy request too large"));
                }
                let mut b = vec![0; a.size as usize];
                rand::rngs::OsRng
                    .try_fill_bytes(&mut b)
                    .map_err(io::Error::other)?;
                MontyObject::bytes(b)
            }
            OsFunctionCall::GetTypeHints(_) | OsFunctionCall::ImportStubs(_) => {
                MontyObject::dict([])
            }
            OsFunctionCall::Import(_) => {
                return Err(io::Error::other("module imports require handle_result"))
            }
        };
        if !p.as_os_str().is_empty() {
            self.record(op, p, bytes);
        }
        Ok(result)
    }
}
impl OsBackend for HostFs {
    fn configure(
        &mut self,
        project_root: PathBuf,
        env: BTreeMap<String, String>,
        capture_dir: Option<PathBuf>,
        deny_write: PathDenyRules,
        deny_read: PathDenyRules,
    ) {
        let config = HostFsConfig {
            project_root,
            env,
            capture_dir,
            deny_write,
            deny_read,
            ..self.config.clone()
        };
        *self = Self::new(config);
    }
    fn cwd(&self) -> Option<String> {
        Some(virtual_path(&self.config.project_root, platform()))
    }
    fn handle(&mut self, call: OsFunctionCall) -> Answer {
        self.dispatch(call).map_err(error)
    }
    fn handle_result(&mut self, call: OsFunctionCall) -> ExtFunctionResult {
        if let OsFunctionCall::Import(args) = call {
            for candidate in args.candidates() {
                match self.handle(OsFunctionCall::ReadText(candidate.as_str().into())) {
                    Ok(v) => {
                        return ExtFunctionResult::ModuleFile(ModuleFile {
                            path: candidate,
                            source: v.as_ref().as_str().unwrap_or_default().into(),
                        })
                    }
                    Err((kind, message))
                        if !matches!(
                            kind.as_str(),
                            "FileNotFoundError" | "IsADirectoryError" | "NotADirectoryError"
                        ) =>
                    {
                        return exception(&kind, message).into()
                    }
                    _ => {}
                }
            }
            exception(
                "ModuleNotFoundError",
                format!("No module named '{}'", args.module),
            )
            .into()
        } else {
            match self.handle(call) {
                Ok(v) => v.into(),
                Err((kind, message)) => exception(&kind, message).into(),
            }
        }
    }
    fn take_events(&mut self) -> Vec<KernelEvent> {
        std::mem::take(&mut self.events)
    }
    fn helper(&mut self, call: HelperCall<'_>) -> Result<Value, HelperError> {
        match call.name {
            "read" => self.read_helper(call),
            "grep" => self.grep_helper(call),
            "replace" => self.replace_helper(call),
            name => Err(raise(
                "NameError",
                format!("name '{name}' is not available in this session"),
            )),
        }
    }
}
const GREP_MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
const GREP_MAX_ENTRIES: u64 = 20_000;
const GREP_MAX_LINE_CHARS: usize = 400;
const GREP_MAX_TEXT_CHARS: usize = 200_000;
const REPLACE_MAX_BYTES: u64 = 32 * 1024 * 1024;
const DIFF_MAX_LINES: usize = 60;
const DIFF_MAX_CHARS: usize = 4_000;
impl HostFs {
    fn shown(&self, p: &Path) -> String {
        let relative = p
            .strip_prefix(&self.config.project_root)
            .ok()
            .and_then(|rel| {
                rel.components()
                    .filter(|c| !matches!(c, std::path::Component::CurDir))
                    .map(|c| match c {
                        std::path::Component::Normal(part) => Some(part.to_string_lossy()),
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>()
            });
        match relative {
            Some(parts) if parts.is_empty() => ".".into(),
            Some(parts) => parts.join("/"),
            None => virtual_path(p, platform()),
        }
    }
    fn read_helper(&mut self, mut call: HelperCall<'_>) -> Result<Value, HelperError> {
        let mut a = Bound::new(
            "read",
            &["path", "start", "end", "numbered"],
            3,
            1,
            &mut call,
        )?;
        let path = a.path("path", None)?;
        let start = a.int("start", 1)?;
        let end = a.optional_int("end")?;
        let numbered = a.flag("numbered", true)?;
        let p = self.resolve(&path);
        let shown = self.shown(&p);
        let failed = |e: io::Error| HelperError::from(helper_error("read", &shown, e));
        self.guard(&p, false).map_err(failed)?;
        reject_dir(&p).map_err(failed)?;
        let file = fs::File::open(&p).map_err(failed)?;
        let refusal = match (start, end) {
            (0, _) => Some("start must be 1 or more, or negative to count from the end (-40 reads the last 40 lines)".to_string()),
            (start, Some(_)) if start < 0 => {
                Some("end cannot be combined with a negative start".into())
            }
            (start, Some(end)) if end < start => {
                Some(format!("end ({end}) is before start ({start})"))
            }
            _ => None,
        };
        if let Some(refusal) = refusal {
            let (lines, bytes) = count_lines(file).map_err(failed)?;
            self.record("read", &p, bytes);
            return Err(raise(
                "ValueError",
                format!("read(): {refusal}; {shown} has {}", counted(lines, "line")),
            ));
        }
        let first = start.max(1) as u64;
        let tail = (start < 0).then(|| start.unsigned_abs());
        let last = end.map(|end| end as u64);
        let limit = call.max_result_bytes;
        let over = || {
            raise(
                "RuntimeError",
                format!(
                    "read() returned more than {limit} bytes, over the limit a script can receive at once ({limit} bytes); read a smaller range with start and end"
                ),
            )
        };
        let mut reader = io::BufReader::new(file);
        let mut check = Utf8Check::default();
        let mut line = Vec::new();
        let (mut number, mut held) = (0u64, 0u64);
        // A line without its text was over the limit, or gave it up for later lines.
        let mut picked: VecDeque<(u64, Option<String>)> = VecDeque::new();
        while last.is_none_or(|last| number < last) {
            let keep = tail.is_some() || number + 1 >= first;
            let room = match (keep, tail) {
                (false, _) => 0,
                (true, Some(_)) => limit,
                (true, None) => limit.saturating_sub(held),
            };
            line.clear();
            let cap = if keep { room.saturating_add(1) } else { 0 };
            let n = match read_line(&mut reader, &mut check, &mut line, cap).map_err(failed)? {
                Ok(n) => n,
                Err(offset) => {
                    self.record("read", &p, check.offset);
                    return Err(raise(
                        "UnicodeDecodeError",
                        format!(
                            "read(): {shown} is not UTF-8 text (invalid byte at offset {offset}); use open(path, \"rb\")"
                        ),
                    ));
                }
            };
            if n == 0 {
                break;
            }
            number += 1;
            if !keep {
                continue;
            }
            if n > room {
                if tail.is_none() {
                    self.record("read", &p, check.offset);
                    return Err(over());
                }
                picked.push_back((number, None));
            } else {
                held += n;
                let text = String::from_utf8(std::mem::take(&mut line))
                    .map_err(|e| failed(io::Error::new(io::ErrorKind::InvalidData, e)))?;
                picked.push_back((number, Some(text)));
            }
            if let Some(tail) = tail {
                if picked.len() as u64 > tail {
                    if let Some((_, Some(text))) = picked.pop_front() {
                        held -= text.len() as u64;
                    }
                }
                for (_, text) in picked.iter_mut() {
                    if held <= limit {
                        break;
                    }
                    if let Some(text) = text.take() {
                        held -= text.len() as u64;
                    }
                }
            }
        }
        self.record("read", &p, check.offset);
        if picked.iter().any(|(_, text)| text.is_none()) {
            return Err(over());
        }
        if tail.is_none() && number < first && !(number == 0 && first == 1) {
            return Err(raise(
                "ValueError",
                format!(
                    "read(): start {first} is past the end of {shown} ({})",
                    counted(number, "line")
                ),
            ));
        }
        let width = picked.back().map_or(0, |(n, _)| n.to_string().len()).max(4);
        let picked = picked
            .into_iter()
            .map(|(n, text)| (n, text.unwrap_or_default()));
        if !numbered {
            return Ok(Value::String(picked.map(|(_, l)| l).collect()));
        }
        Ok(Value::String(
            picked
                .map(|(n, line)| {
                    let line = line.strip_suffix('\n').unwrap_or(&line);
                    let line = line.strip_suffix('\r').unwrap_or(line);
                    format!("{n:>width$} | {line}\n")
                })
                .collect(),
        ))
    }
    fn grep_helper(&mut self, mut call: HelperCall<'_>) -> Result<Value, HelperError> {
        let mut a = Bound::new(
            "grep",
            &[
                "pattern",
                "path",
                "context",
                "glob",
                "ignore_case",
                "limit",
                "data",
            ],
            2,
            1,
            &mut call,
        )?;
        let pattern = a.string("pattern")?;
        let path = a.path("path", Some("."))?;
        let context = a.int("context", 0)?;
        let glob_text = a.optional_string("glob")?;
        let ignore_case = a.flag("ignore_case", false)?;
        let limit = a.int("limit", 200)?;
        let data = a.flag("data", false)?;
        if !(0..=20).contains(&context) {
            return Err(raise(
                "ValueError",
                "grep(): context must be between 0 and 20".into(),
            ));
        }
        if !(1..=10_000).contains(&limit) {
            return Err(raise(
                "ValueError",
                "grep(): limit must be between 1 and 10000".into(),
            ));
        }
        let regex = regex::RegexBuilder::new(&pattern)
            .case_insensitive(ignore_case)
            .build()
            .map_err(|e| {
                raise(
                    "ValueError",
                    format!("grep(): invalid pattern: {}", regex_error(&e)),
                )
            })?;
        let glob = glob_text
            .as_ref()
            .map(|g| {
                glob::Pattern::new(g)
                    .map(|pattern| (pattern, g.contains('/')))
                    .map_err(|e| raise("ValueError", format!("grep(): invalid glob: {e}")))
            })
            .transpose()?;
        let p: PathBuf = self.resolve(&path).components().collect();
        let shown = self.shown(&p);
        let failed = |e: io::Error| HelperError::from(helper_error("grep", &shown, e));
        let rules = self.rules(false);
        self.judge(&rules, &p, Access::Read).map_err(failed)?;
        let single = !fs::metadata(&p).map_err(failed)?.is_dir();
        if single && glob.is_some() {
            return Err(raise(
                "ValueError",
                "grep(): glob filters directory searches; path is a file".into(),
            ));
        }
        let listing = (call.list_files)(&FileListRequest {
            path: p.clone(),
            max_entries: GREP_MAX_ENTRIES,
            glob: glob_text,
        })?;
        let mut files: Vec<PathBuf> = listing
            .files
            .into_iter()
            .filter(|f| {
                f.starts_with(&p)
                    && glob
                        .as_ref()
                        .is_none_or(|(pattern, nested)| glob_matches(pattern, *nested, &p, f))
            })
            .collect();
        files.sort();
        files.dedup();
        let (context, limit) = (context as usize, limit as u64);
        let mut skipped = Skipped {
            blocked: listing.blocked,
            unlisted: listing.unlisted as u64,
            ..Default::default()
        };
        let (mut searched, mut total, mut returned, mut scanned) = (0u64, 0u64, 0u64, 0u64);
        let mut over_budget = false;
        let mut matches = vec![];
        let mut out = Text::default();
        for file in &files {
            match self.judge(&rules, file, Access::Read) {
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
                    skipped.protected += 1;
                    continue;
                }
                Err(_) => {
                    skipped.unreadable += 1;
                    continue;
                }
            }
            let Ok(meta) = fs::metadata(file) else {
                skipped.unreadable += 1;
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            if meta.len() > GREP_MAX_FILE_BYTES {
                skipped.large += 1;
                continue;
            }
            if scanned + meta.len() > self.config.max_scan_bytes {
                over_budget = true;
                break;
            }
            let mut bytes = vec![];
            if fs::File::open(file)
                .and_then(|f| f.take(GREP_MAX_FILE_BYTES + 1).read_to_end(&mut bytes))
                .is_err()
            {
                skipped.unreadable += 1;
                continue;
            }
            scanned += bytes.len() as u64;
            if single {
                self.record("grep", file, bytes.len() as u64);
            }
            if bytes.len() as u64 > GREP_MAX_FILE_BYTES {
                skipped.large += 1;
                continue;
            }
            if bytes[..bytes.len().min(8192)].contains(&0) {
                skipped.binary += 1;
                continue;
            }
            let Ok(text) = std::str::from_utf8(&bytes) else {
                skipped.binary += 1;
                continue;
            };
            searched += 1;
            let lines: Vec<&str> = text
                .strip_prefix('\u{feff}')
                .unwrap_or(text)
                .lines()
                .collect();
            let hits: Vec<usize> = (0..lines.len())
                .filter(|i| regex.is_match(lines[*i]))
                .collect();
            if hits.is_empty() {
                continue;
            }
            if !single {
                self.record("grep", file, bytes.len() as u64);
            }
            let name = self.shown(file);
            let visible = &hits[..hits.len().min(limit.saturating_sub(total) as usize)];
            total += hits.len() as u64;
            returned += visible.len() as u64;
            if data {
                for &i in visible {
                    matches.push(json!({"path": name, "line": i + 1, "text": cut_line(lines[i])}));
                }
            } else {
                context_groups(&mut out, &name, &lines, visible, context);
            }
        }
        let stopped = listing.truncated || over_budget;
        if data {
            return Ok(json!({
                "matches": matches,
                "total": total,
                "truncated": total > returned || stopped,
                "files_searched": searched,
                "files_skipped": skipped.total(),
            }));
        }
        let mut text = out.lines;
        if out.cut {
            text.push(format!(
                "… output cut at {} characters; narrow the search",
                thousands(GREP_MAX_TEXT_CHARS as u64)
            ));
        }
        if total == 0 {
            text.push(format!(
                "no matches (searched {})",
                counted(searched, "file")
            ));
        }
        if total > returned {
            text.push(format!(
                "… {} more matches not shown (limit {limit}); narrow the pattern or raise limit",
                thousands(total - returned)
            ));
        }
        if listing.truncated {
            text.push(format!(
                "stopped early: the file listing of {shown} stopped at {} entries; counts cover only the files searched; narrow path or glob",
                thousands(GREP_MAX_ENTRIES)
            ));
        }
        if over_budget {
            text.push(format!(
                "stopped early: the files to search hold more than {}; counts cover only the files searched; narrow path or glob",
                size(self.config.max_scan_bytes)
            ));
        }
        if let Some(skipped) = skipped.describe() {
            text.push(format!("skipped: {skipped}"));
        }
        Ok(Value::String(text.join("\n")))
    }
    fn replace_helper(&mut self, mut call: HelperCall<'_>) -> Result<Value, HelperError> {
        let mut a = Bound::new("replace", &["path", "old", "new", "count"], 3, 3, &mut call)?;
        let path = a.path("path", None)?;
        let old = a.string("old")?;
        let new = a.string("new")?;
        let count = a.int("count", 1)?;
        let p = self.resolve(&path);
        let shown = self.shown(&p);
        let failed = |e: io::Error| HelperError::from(helper_error("replace", &shown, e));
        self.check(&p, Access::Content).map_err(failed)?;
        if old.is_empty() {
            return Err(raise(
                "ValueError",
                "replace(): old must not be empty".into(),
            ));
        }
        if old == new {
            return Err(raise(
                "ValueError",
                "replace(): old and new are identical; nothing to change".into(),
            ));
        }
        if count < 0 {
            return Err(raise(
                "ValueError",
                "replace(): count must be 0 or more (0 replaces every occurrence)".into(),
            ));
        }
        let target = if fs::symlink_metadata(&p).is_ok_and(|m| m.is_symlink()) {
            fs::canonicalize(&p).map_err(failed)?
        } else {
            p.clone()
        };
        self.guard(&target, false).map_err(failed)?;
        reject_dir(&target).map_err(failed)?;
        let unchanged = "the file is unchanged.";
        let not_regular = || {
            raise(
                "PermissionError",
                format!("replace(): {shown} is not a regular file; {unchanged}"),
            )
        };
        if !fs::metadata(&target).map_err(failed)?.is_file() {
            return Err(not_regular());
        }
        let mut options = fs::OpenOptions::new();
        options.read(true).write(true);
        no_follow(&mut options);
        let mut file = options.open(&target).map_err(failed)?;
        let opened = file.metadata().map_err(failed)?;
        if !opened.is_file() {
            return Err(not_regular());
        }
        let identity = opened_identity(&file, &opened).map_err(failed)?;
        if identity.1 != 1 {
            return Err(raise(
                "PermissionError",
                format!(
                    "replace(): {shown} has {} links; replace() edits only files with one link; {unchanged}",
                    identity.1
                ),
            ));
        }
        let too_big = |bytes: u64| {
            raise(
                "ValueError",
                format!(
                    "replace(): {shown} is {} bytes; replace() edits files up to 32 MiB",
                    thousands(bytes)
                ),
            )
        };
        if opened.len() > REPLACE_MAX_BYTES {
            return Err(too_big(opened.len()));
        }
        let mut bytes = vec![];
        (&mut file)
            .take(REPLACE_MAX_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(failed)?;
        drop(file);
        self.record("replace", &p, bytes.len() as u64);
        if bytes.len() as u64 > REPLACE_MAX_BYTES {
            return Err(too_big(bytes.len() as u64));
        }
        let text = std::str::from_utf8(&bytes).map_err(|e| {
            raise(
                "UnicodeDecodeError",
                format!(
                    "replace(): {shown} is not UTF-8 text (invalid byte at offset {}); {unchanged}",
                    e.valid_up_to()
                ),
            )
        })?;
        let found = text.matches(old.as_str()).count() as u64;
        let expected = count as u64;
        if found == 0 || (expected > 0 && found != expected) {
            let wanted = if expected == 0 {
                "at least 1".to_string()
            } else {
                expected.to_string()
            };
            let mut message = format!(
                "replace(): found {} of old in {shown}, expected {wanted}; {unchanged}",
                counted(found, "occurrence")
            );
            if found == 0 {
                message.push_str(" Check whitespace and line endings; read(path, numbered=False) shows the exact text.");
                if text.contains("\r\n") && old.contains('\n') && !old.contains("\r\n") {
                    message.push_str(" The file uses CRLF line endings and old uses LF.");
                }
            } else if found > expected {
                let select = if expected == 1 {
                    "one".to_string()
                } else {
                    expected.to_string()
                };
                message.push_str(&format!(" Pass count={found} (or count=0) to replace all of them, or include more surrounding text to select {select}."));
            }
            return Err(raise("ValueError", message));
        }
        let result_size = text.len() as u128 + found as u128 * new.len() as u128
            - found as u128 * old.len() as u128;
        if result_size > REPLACE_MAX_BYTES as u128 {
            return Err(raise(
                "ValueError",
                format!(
                    "replace(): replacing {} {} would make {shown} {} bytes; replace() writes files up to 32 MiB; {unchanged}",
                    thousands(found),
                    if found == 1 { "occurrence" } else { "occurrences" },
                    thousands(result_size as u64)
                ),
            ));
        }
        let updated = text.replace(old.as_str(), &new);
        self.publish(
            &p,
            &target,
            &shown,
            &opened,
            identity,
            &bytes,
            updated.as_bytes(),
        )?;
        self.record("replace.write", &p, updated.len() as u64);
        Ok(Value::String(unified_diff(&shown, text, &updated)))
    }
    #[allow(clippy::too_many_arguments)]
    fn publish(
        &mut self,
        p: &Path,
        target: &Path,
        shown: &str,
        read: &fs::Metadata,
        identity: (FileId, u64),
        original: &[u8],
        updated: &[u8],
    ) -> Result<(), HelperError> {
        let failed = |e: io::Error| HelperError::from(helper_error("replace", shown, e));
        let name = target
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut attempts = 0;
        let (staged, mut file) = loop {
            let staged =
                target.with_file_name(format!(".{name}.{:08x}.replace", rand::random::<u32>()));
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
                options.mode(read.permissions().mode() & 0o777);
            }
            match options.open(&staged) {
                Ok(file) => break (staged, file),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists && attempts < 8 => attempts += 1,
                Err(e) => return Err(failed(e)),
            }
        };
        let staging = file
            .write_all(updated)
            .and_then(|()| file.set_permissions(read.permissions()))
            .and_then(|()| file.sync_all());
        drop(file);
        if let Err(e) = staging {
            let _ = fs::remove_file(&staged);
            return Err(failed(e));
        }
        let prepared = match self.prepare_capture(p, original) {
            Ok(prepared) => prepared,
            Err(e) => {
                let _ = fs::remove_file(&staged);
                return Err(failed(e));
            }
        };
        if let Some(hook) = self.config.replace_hook {
            hook(target);
        }
        let same = fs::symlink_metadata(target).is_ok_and(|now| {
            now.is_file()
                && now.len() == read.len()
                && now.modified().ok() == read.modified().ok()
                && now.permissions() == read.permissions()
                && path_identity(target, &now, false).is_ok_and(|now| now == identity)
        });
        if !same {
            discard(&prepared);
            let _ = fs::remove_file(&staged);
            return Err(raise(
                "RuntimeError",
                format!("replace(): {shown} changed on disk while replace() ran; nothing was written. Read it again and retry."),
            ));
        }
        if let Err(e) = fs::rename(&staged, target) {
            discard(&prepared);
            let _ = fs::remove_file(&staged);
            let (kind, text) = error(e);
            return Err(raise(
                &kind,
                format!("replace(): renaming the new text onto {shown} failed ({text}); the file is unchanged."),
            ));
        }
        self.commit_capture(prepared);
        Ok(())
    }
}
#[cfg(windows)]
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
/// The open never follows a link at the leaf: it fails on Unix and opens the link itself on Windows.
fn no_follow(options: &mut fs::OpenOptions) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
}
/// The identity and link count of an open file.
#[cfg(unix)]
fn opened_identity(_: &fs::File, meta: &fs::Metadata) -> io::Result<(FileId, u64)> {
    use std::os::unix::fs::MetadataExt;
    Ok(((meta.dev(), meta.ino()), meta.nlink()))
}
#[cfg(windows)]
fn opened_identity(file: &fs::File, _: &fs::Metadata) -> io::Result<(FileId, u64)> {
    let info = winapi_util::file::information(file)?;
    Ok((
        (info.volume_serial_number(), info.file_index()),
        info.number_of_links(),
    ))
}
/// The identity and link count of the regular file at `path`, whose metadata is `meta`.
#[cfg(unix)]
fn path_identity(_: &Path, meta: &fs::Metadata, _: bool) -> io::Result<(FileId, u64)> {
    use std::os::unix::fs::MetadataExt;
    Ok(((meta.dev(), meta.ino()), meta.nlink()))
}
#[cfg(windows)]
fn path_identity(path: &Path, meta: &fs::Metadata, follow: bool) -> io::Result<(FileId, u64)> {
    use std::os::windows::fs::OpenOptionsExt;
    let mut options = fs::OpenOptions::new();
    options.access_mode(0);
    if !follow {
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    opened_identity(&options.open(path)?, meta)
}
fn stat(m: &fs::Metadata) -> MontyObject {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        stat_result(
            m.mode() as i64,
            m.ino() as i64,
            m.dev() as i64,
            m.nlink() as i64,
            m.uid() as i64,
            m.gid() as i64,
            m.len() as i64,
            m.atime() as f64 + m.atime_nsec() as f64 / 1e9,
            m.mtime() as f64 + m.mtime_nsec() as f64 / 1e9,
            m.ctime() as f64 + m.ctime_nsec() as f64 / 1e9,
        )
    }
    #[cfg(not(unix))]
    {
        let seconds = |t: io::Result<SystemTime>| {
            t.ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0., |d| d.as_secs_f64())
        };
        stat_result(
            if m.is_dir() { 0o040755 } else { 0o100644 },
            0,
            0,
            1,
            0,
            0,
            m.len() as i64,
            seconds(m.accessed()),
            seconds(m.modified()),
            seconds(m.created()),
        )
    }
}
struct Source<'a> {
    fs: &'a mut HostFs,
    root: PathBuf,
    visits: usize,
    charged: usize,
}
fn info(p: &Path) -> io::Result<EntryInfo> {
    let link = fs::symlink_metadata(p)?;
    let m = fs::metadata(p).ok();
    Ok(EntryInfo {
        is_dir: m.as_ref().is_some_and(|m| m.is_dir()),
        is_file: m.as_ref().is_some_and(|m| m.is_file()),
        is_symlink: link.is_symlink(),
    })
}
impl ScanSource for Source<'_> {
    type Error = io::Error;
    fn list(&mut self, dir: &str) -> io::Result<Option<Listing>> {
        let p = self.root.join(dir);
        if let Err(e) = self.fs.guard(&p, false) {
            return if e.kind() == io::ErrorKind::PermissionDenied {
                Ok(None)
            } else {
                Err(e)
            };
        }
        self.fs.record("Path.scan", &p, 0);
        let entries = match fs::read_dir(&p) {
            Ok(e) => e,
            Err(_) => return Ok(None),
        };
        let mut result = vec![];
        for e in entries {
            self.visit()?;
            let e = e?;
            match self.fs.guard(&e.path(), false) {
                Err(e) if e.kind() == io::ErrorKind::PermissionDenied => continue,
                Err(e) => return Err(e),
                Ok(()) => {}
            }
            result.push((
                e.file_name().to_string_lossy().into_owned(),
                info(&e.path())?,
            ));
        }
        Ok(Some(result.into()))
    }
    fn lookup(&mut self, path: &str) -> io::Result<Option<EntryInfo>> {
        let p = self.root.join(path);
        if let Err(e) = self.fs.guard(&p, false) {
            return if e.kind() == io::ErrorKind::PermissionDenied {
                Ok(None)
            } else {
                Err(e)
            };
        }
        self.fs.record("Path.stat", &p, 0);
        match info(&p) {
            Ok(i) => Ok(Some(i)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }
    fn visit(&mut self) -> io::Result<()> {
        self.visits += 1;
        if self.visits > 100_000 {
            Err(io::Error::other("scan entry limit exceeded"))
        } else {
            Ok(())
        }
    }
    fn charge(&mut self, bytes: usize) -> io::Result<()> {
        self.charged = self.charged.saturating_add(bytes);
        if self.charged > 32 * 1024 * 1024 {
            Err(io::Error::other("scan memory limit exceeded"))
        } else {
            Ok(())
        }
    }
}

// canonicalize falls back to an existing ancestor for new paths. Before doing
// that for writes, distinguish an ordinary missing component from a dangling
// symlink: the latter must not be treated as a lexical path in an allowed tree.
fn reject_dangling_symlinks(path: &Path) -> io::Result<()> {
    let mut component_path = PathBuf::new();
    for component in path.components() {
        component_path.push(component.as_os_str());
        match fs::symlink_metadata(&component_path) {
            Ok(metadata) if metadata.is_symlink() => {
                if let Err(e) = fs::canonicalize(&component_path) {
                    if matches!(
                        e.kind(),
                        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                    ) {
                        return Err(io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            format!(
                                "writes through dangling symlinks are denied: {}",
                                path.display()
                            ),
                        ));
                    }
                    return Err(e);
                }
            }
            Ok(_) => {}
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// The directory entry an operation changes: its resolved parent plus its own name.
fn entry_path(path: &Path) -> io::Result<PathBuf> {
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => {
            let mut entry = resolved_path(parent)?;
            entry.push(name);
            Ok(entry)
        }
        _ => resolved_path(path),
    }
}

fn resolved_path(path: &Path) -> io::Result<PathBuf> {
    let mut existing = path.to_path_buf();
    let mut suffix = Vec::new();
    loop {
        match fs::canonicalize(&existing) {
            Ok(mut resolved) => {
                for part in suffix.into_iter().rev() {
                    resolved.push(part);
                }
                let mut normalized = PathBuf::new();
                for part in resolved.components() {
                    match part {
                        std::path::Component::CurDir => {}
                        std::path::Component::ParentDir => {
                            normalized.pop();
                        }
                        part => normalized.push(part.as_os_str()),
                    }
                }
                return Ok(normalized);
            }
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                let name = existing
                    .file_name()
                    .ok_or_else(|| io::Error::other("cannot resolve protected path"))?
                    .to_owned();
                suffix.push(name);
                if !existing.pop() {
                    return Err(e);
                }
            }
            Err(e) => return Err(e),
        }
    }
}

/// Device and inode on Unix, volume serial number and file index on Windows.
type FileId = (u64, u64);

fn raise(kind: &str, message: String) -> HelperError {
    HelperError::Raise(kind.into(), message)
}

fn helper_error(function: &str, shown: &str, e: io::Error) -> (String, String) {
    let (kind, text) = error(e);
    let message = match kind.as_str() {
        "FileNotFoundError" if function == "grep" => {
            format!("grep(): no such file or directory: {shown}")
        }
        "FileNotFoundError" => format!("{function}(): no such file: {shown}"),
        "IsADirectoryError" => format!("{function}(): {shown} is a directory"),
        _ => format!("{function}(): {text}"),
    };
    (kind, message)
}

struct Bound {
    function: &'static str,
    names: &'static [&'static str],
    values: Vec<Option<Value>>,
    marks: Vec<Option<Mark>>,
}
impl Bound {
    fn new(
        function: &'static str,
        names: &'static [&'static str],
        positional: usize,
        required: usize,
        call: &mut HelperCall<'_>,
    ) -> Result<Self, (String, String)> {
        let type_error = |message: String| ("TypeError".to_string(), message);
        let args = std::mem::take(&mut call.args);
        if args.len() > positional {
            return Err(type_error(format!(
                "{function}() takes at most {positional} positional arguments ({} given)",
                args.len()
            )));
        }
        let marked = |slot: &str| {
            call.marks
                .iter()
                .find(|(marked, _)| marked == slot)
                .map(|(_, mark)| *mark)
        };
        let mut values: Vec<Option<Value>> = vec![None; names.len()];
        let mut marks: Vec<Option<Mark>> = vec![None; names.len()];
        for (i, value) in args.into_iter().enumerate() {
            values[i] = Some(value);
            marks[i] = marked(&i.to_string());
        }
        for (key, value) in std::mem::take(&mut call.kwargs) {
            let Some(i) = names.iter().position(|name| *name == key) else {
                return Err(type_error(format!(
                    "{function}() got an unexpected keyword argument '{key}'"
                )));
            };
            if values[i].is_some() {
                return Err(type_error(format!(
                    "{function}() got multiple values for argument '{key}'"
                )));
            }
            marks[i] = marked(&key);
            values[i] = Some(value);
        }
        if let Some((name, _)) = names[..required]
            .iter()
            .zip(&values)
            .find(|(_, value)| value.is_none())
        {
            return Err(type_error(format!(
                "{function}() missing required argument: '{name}'"
            )));
        }
        Ok(Self {
            function,
            names,
            values,
            marks,
        })
    }
    fn take(&mut self, name: &str) -> (Option<Value>, Option<Mark>) {
        match self.names.iter().position(|n| *n == name) {
            Some(i) => (self.values[i].take(), self.marks[i]),
            None => (None, None),
        }
    }
    fn wrong(
        &self,
        name: &str,
        expected: &str,
        value: &Value,
        mark: Option<Mark>,
    ) -> (String, String) {
        let found = match (mark, value) {
            (Some(Mark::Path), _) => "PosixPath",
            (Some(Mark::BigInt), _) => "int",
            (None, Value::Null) => "NoneType",
            (None, Value::Bool(_)) => "bool",
            (None, Value::Number(n)) if n.is_i64() || n.is_u64() => "int",
            (None, Value::Number(_)) => "float",
            (None, Value::String(_)) => "str",
            (None, Value::Array(_)) => "list",
            (None, Value::Object(_)) => "dict",
        };
        (
            "TypeError".into(),
            format!(
                "{}() argument {name} must be {expected}, not {found}",
                self.function
            ),
        )
    }
    fn out_of_range(&self, name: &str) -> (String, String) {
        (
            "ValueError".into(),
            format!("{}() argument {name} is out of range", self.function),
        )
    }
    fn path(&mut self, name: &str, default: Option<&str>) -> Result<String, (String, String)> {
        match self.take(name) {
            (None, _) => Ok(default.unwrap_or_default().into()),
            (Some(Value::String(s)), None | Some(Mark::Path)) => Ok(s),
            (Some(v), mark) => Err(self.wrong(name, "str or Path", &v, mark)),
        }
    }
    fn string(&mut self, name: &str) -> Result<String, (String, String)> {
        match self.take(name) {
            (Some(Value::String(s)), None) => Ok(s),
            (Some(v), mark) => Err(self.wrong(name, "str", &v, mark)),
            (None, _) => Ok(String::new()),
        }
    }
    fn optional_string(&mut self, name: &str) -> Result<Option<String>, (String, String)> {
        match self.take(name) {
            (None, _) | (Some(Value::Null), None) => Ok(None),
            (Some(Value::String(s)), None) => Ok(Some(s)),
            (Some(v), mark) => Err(self.wrong(name, "str or None", &v, mark)),
        }
    }
    fn int(&mut self, name: &str, default: i64) -> Result<i64, (String, String)> {
        match self.take(name) {
            (None, _) => Ok(default),
            (Some(v), mark) => self.integer(name, "int", v, mark),
        }
    }
    fn optional_int(&mut self, name: &str) -> Result<Option<i64>, (String, String)> {
        match self.take(name) {
            (None, _) | (Some(Value::Null), None) => Ok(None),
            (Some(v), mark) => self.integer(name, "int or None", v, mark).map(Some),
        }
    }
    fn integer(
        &self,
        name: &str,
        expected: &str,
        value: Value,
        mark: Option<Mark>,
    ) -> Result<i64, (String, String)> {
        match (value, mark) {
            (_, Some(Mark::BigInt)) => Err(self.out_of_range(name)),
            (Value::Number(n), None) if n.is_i64() || n.is_u64() => {
                n.as_i64().ok_or_else(|| self.out_of_range(name))
            }
            (v, mark) => Err(self.wrong(name, expected, &v, mark)),
        }
    }
    fn flag(&mut self, name: &str, default: bool) -> Result<bool, (String, String)> {
        match self.take(name) {
            (None, _) => Ok(default),
            (Some(Value::Bool(b)), None) => Ok(b),
            (Some(v), mark) => Err(self.wrong(name, "bool", &v, mark)),
        }
    }
}

fn count_lines(mut file: fs::File) -> io::Result<(u64, u64)> {
    let mut buffer = vec![0; 64 * 1024];
    let (mut lines, mut bytes, mut last) = (0u64, 0u64, b'\n');
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            return Ok((lines + u64::from(last != b'\n'), bytes));
        }
        lines += buffer[..n].iter().filter(|b| **b == b'\n').count() as u64;
        bytes += n as u64;
        last = buffer[n - 1];
    }
}

/// UTF-8 checked across reads; `offset` counts the bytes checked so far.
#[derive(Default)]
struct Utf8Check {
    pending: Vec<u8>,
    offset: u64,
}
impl Utf8Check {
    /// `Err` holds the offset of the first byte that is not UTF-8.
    fn feed(&mut self, mut bytes: &[u8]) -> Result<(), u64> {
        if let Some(&lead) = self.pending.first() {
            let width = match lead {
                0xC0..=0xDF => 2,
                0xE0..=0xEF => 3,
                _ => 4,
            };
            let take = (width - self.pending.len()).min(bytes.len());
            self.pending.extend_from_slice(&bytes[..take]);
            bytes = &bytes[take..];
            if self.pending.len() < width {
                return Ok(());
            }
            if std::str::from_utf8(&self.pending).is_err() {
                return Err(self.offset);
            }
            self.offset += width as u64;
            self.pending.clear();
        }
        match std::str::from_utf8(bytes) {
            Ok(_) => {
                self.offset += bytes.len() as u64;
                Ok(())
            }
            Err(e) if e.error_len().is_none() => {
                self.offset += e.valid_up_to() as u64;
                self.pending.extend_from_slice(&bytes[e.valid_up_to()..]);
                Ok(())
            }
            Err(e) => Err(self.offset + e.valid_up_to() as u64),
        }
    }
}

/// Consumes one line through its `\n` and keeps at most `room` of its bytes in `kept`.
/// Returns its length (0 at the end of the file), or `Err` with the offset of a byte that is
/// not UTF-8.
fn read_line(
    reader: &mut impl BufRead,
    check: &mut Utf8Check,
    kept: &mut Vec<u8>,
    room: u64,
) -> io::Result<Result<u64, u64>> {
    let mut length = 0u64;
    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            return Ok(if check.pending.is_empty() {
                Ok(length)
            } else {
                Err(check.offset)
            });
        }
        let (part, done) = match chunk.iter().position(|b| *b == b'\n') {
            Some(i) => (&chunk[..=i], true),
            None => (chunk, false),
        };
        if let Err(offset) = check.feed(part) {
            return Ok(Err(offset));
        }
        let keep = (part.len() as u64).min(room.saturating_sub(kept.len() as u64)) as usize;
        kept.extend_from_slice(&part[..keep]);
        let n = part.len();
        reader.consume(n);
        length += n as u64;
        if done {
            return Ok(Ok(length));
        }
    }
}

fn counted(n: u64, noun: &str) -> String {
    format!("{} {noun}{}", thousands(n), if n == 1 { "" } else { "s" })
}

fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn size(bytes: u64) -> String {
    const MIB: u64 = 1024 * 1024;
    if bytes >= MIB && bytes % MIB == 0 {
        format!("{} MiB", bytes / MIB)
    } else {
        counted(bytes, "byte")
    }
}

fn cut_line(line: &str) -> String {
    match line.char_indices().nth(GREP_MAX_LINE_CHARS) {
        Some((end, _)) => format!("{}…", &line[..end]),
        None => line.into(),
    }
}

/// Text output rows, kept until the next one would pass `GREP_MAX_TEXT_CHARS`.
#[derive(Default)]
struct Text {
    lines: Vec<String>,
    chars: usize,
    cut: bool,
}
impl Text {
    fn push(&mut self, line: String) {
        let added = line.chars().count() + usize::from(!self.lines.is_empty());
        if self.cut || self.chars + added > GREP_MAX_TEXT_CHARS {
            self.cut = true;
            return;
        }
        self.chars += added;
        self.lines.push(line);
    }
}

fn context_groups(out: &mut Text, name: &str, lines: &[&str], hits: &[usize], context: usize) {
    let last = lines.len().saturating_sub(1);
    let mut i = 0;
    while i < hits.len() && !out.cut {
        let start = hits[i].saturating_sub(context);
        let mut end = (hits[i] + context).min(last);
        let mut j = i + 1;
        while j < hits.len() && hits[j].saturating_sub(context) <= end + 1 {
            end = (hits[j] + context).min(last);
            j += 1;
        }
        if context > 0 && !out.lines.is_empty() {
            out.push("--".into());
        }
        for k in start..=end {
            if out.cut {
                return;
            }
            let mark = if hits[i..j].binary_search(&k).is_ok() {
                ':'
            } else {
                '-'
            };
            out.push(format!(
                "{name}{mark}{}{mark} {}",
                k + 1,
                cut_line(lines[k])
            ));
        }
        i = j;
    }
}

/// Whether `file`, found below `dir`, matches the `glob` of a `grep()` call (`nested` when the
/// glob has a `/`); the engine filters its listing with the same rule.
pub fn glob_matches(pattern: &glob::Pattern, nested: bool, dir: &Path, file: &Path) -> bool {
    let options = glob::MatchOptions {
        case_sensitive: true,
        require_literal_separator: true,
        require_literal_leading_dot: false,
    };
    if !nested {
        return file
            .file_name()
            .is_some_and(|name| pattern.matches_with(&name.to_string_lossy(), options));
    }
    file.strip_prefix(dir).is_ok_and(|relative| {
        let relative: Vec<_> = relative
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect();
        pattern.matches_with(&relative.join("/"), options)
    })
}

fn regex_error(e: &regex::Error) -> String {
    let text = e.to_string();
    text.lines()
        .find_map(|line| line.trim().strip_prefix("error: "))
        .unwrap_or_else(|| text.lines().next().unwrap_or_default())
        .to_string()
}

fn unified_diff(shown: &str, before: &str, after: &str) -> String {
    let diff = similar::TextDiff::configure()
        .timeout(std::time::Duration::from_secs(1))
        .diff_lines(before, after);
    let mut unified = diff.unified_diff();
    unified.context_radius(2);
    let mut out = DiffText::default();
    for (i, hunk) in unified.iter_hunks().enumerate() {
        if i == 0 {
            out.push(|| format!("--- a/{shown}"));
            out.push(|| format!("+++ b/{shown}"));
        }
        out.push(|| hunk.header().to_string());
        for change in hunk.iter_changes() {
            out.push(|| {
                let value = change.value();
                format!(
                    "{}{}",
                    change.tag(),
                    value.strip_suffix('\n').unwrap_or(value)
                )
            });
            if change.missing_newline() {
                out.push(|| "\\ No newline at end of file".into());
            }
        }
    }
    out.finish()
}

/// Unified diff lines, kept up to `DIFF_MAX_LINES` and `DIFF_MAX_CHARS`; the rest only counted.
#[derive(Default)]
struct DiffText {
    lines: Vec<String>,
    chars: usize,
    total: usize,
    cut: bool,
}
impl DiffText {
    fn push(&mut self, line: impl FnOnce() -> String) {
        self.total += 1;
        if self.cut {
            return;
        }
        let line = line();
        let added = line.strip_suffix('\r').unwrap_or(&line).chars().count() + 1;
        if self.lines.len() == DIFF_MAX_LINES || self.chars + added > DIFF_MAX_CHARS {
            self.cut = true;
            return;
        }
        self.chars += added;
        self.lines.push(line);
    }
    fn finish(self) -> String {
        if !self.cut {
            return self.lines.iter().map(|line| format!("{line}\n")).collect();
        }
        let kept: Vec<&str> = self
            .lines
            .iter()
            .map(|line| line.strip_suffix('\r').unwrap_or(line))
            .collect();
        format!(
            "{}\n… {} more diff lines\n",
            kept.join("\n"),
            self.total - self.lines.len()
        )
    }
}

/// A result kept to be raised where the rules would have met it.
type Lazy<T> = Result<T, (io::ErrorKind, String)>;
fn lazy<T>(result: io::Result<T>) -> Lazy<T> {
    result.map_err(|e| (e.kind(), e.to_string()))
}
fn again((kind, message): &(io::ErrorKind, String)) -> io::Error {
    io::Error::new(*kind, message.clone())
}
struct Rules {
    write: bool,
    /// Each allow prefix where it resolves, and whether it can authorize anything.
    allow: Vec<(Lazy<PathBuf>, bool)>,
    /// Each deny prefix (`true`) and protected file (`false`), resolved to compare.
    deny: Vec<(Lazy<PathBuf>, bool)>,
    /// The identity of each protected file; `None` when it is not a regular file.
    files: Vec<Lazy<Option<FileId>>>,
}

struct Prepared {
    path: PathBuf,
    original: WriteOriginal,
    charged: u64,
}
fn discard(prepared: &Option<Prepared>) {
    if let Some(Prepared {
        original: WriteOriginal::File {
            capture_path,
        },
        ..
    }) = prepared
    {
        let _ = fs::remove_file(capture_path);
    }
}

#[derive(Default)]
struct Skipped {
    large: u64,
    binary: u64,
    protected: u64,
    blocked: u64,
    unreadable: u64,
    unlisted: u64,
}
impl Skipped {
    fn total(&self) -> u64 {
        self.large + self.binary + self.protected + self.blocked + self.unreadable + self.unlisted
    }
    fn describe(&self) -> Option<String> {
        let files = |n: u64| if n == 1 { "file" } else { "files" };
        let parts: Vec<String> = [
            (
                self.large,
                format!("{} {} over 4 MiB", self.large, files(self.large)),
            ),
            (
                self.binary,
                format!("{} binary or non-UTF-8 {}", self.binary, files(self.binary)),
            ),
            (
                self.protected,
                format!("{} protected {}", self.protected, files(self.protected)),
            ),
            (
                self.blocked,
                format!("{} privacy-blocked {}", self.blocked, files(self.blocked)),
            ),
            (
                self.unreadable,
                format!("{} unreadable {}", self.unreadable, files(self.unreadable)),
            ),
            (
                self.unlisted,
                format!(
                    "{} {} whose names are not UTF-8",
                    self.unlisted,
                    files(self.unlisted)
                ),
            ),
        ]
        .into_iter()
        .filter(|(n, _)| *n > 0)
        .map(|(_, part)| part)
        .collect();
        (!parts.is_empty()).then(|| parts.join(", "))
    }
}
