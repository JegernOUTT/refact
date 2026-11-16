use crate::{
    kernel::{exception, OsBackend},
    protocol::{KernelEvent, WriteOriginal, PathDenyRules},
};
use monty_types::{
    *,
    scan::{collect_scan, EntryInfo, Listing, ScanSource},
};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::{self, Read, Write},
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
        self.guard_operation(p, write, false)
    }
    fn guard_operation(&self, p: &Path, write: bool, destructive: bool) -> io::Result<()> {
        if write {
            reject_dangling_symlinks(p)?;
        }
        let rules = if write {
            &self.config.deny_write
        } else {
            &self.config.deny_read
        };
        let resolved = resolved_path(p)?;
        let comparable = |p: PathBuf| {
            if cfg!(windows) || cfg!(target_os = "macos") {
                PathBuf::from(p.to_string_lossy().to_lowercase())
            } else {
                p
            }
        };
        let resolved = comparable(resolved);
        {
            for allowed in &rules.allow_prefixes {
                if destructive
                    && resolved_path(&self.resolve(&allowed.to_string_lossy()))
                        .is_ok_and(|root| comparable(root) == resolved)
                {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        format!(
                            "the directory {} itself cannot be renamed or removed; work with the files inside it",
                            p.display()
                        ),
                    ));
                }
                let allowed = self.resolve(&allowed.to_string_lossy());
                // A write exception authorizes contents, not replacing the root itself.
                // Never let a missing or substituted symlink root widen the exception.
                if write
                    && !fs::symlink_metadata(&allowed).is_ok_and(|m| m.is_dir() && !m.is_symlink())
                {
                    continue;
                }
                let allowed = comparable(resolved_path(&allowed)?);
                if resolved.starts_with(&allowed) && (!write || resolved != allowed) {
                    return Ok(());
                }
            }
        }
        for (path, prefix) in rules
            .prefixes
            .iter()
            .map(|p| (p, true))
            .chain(rules.files.iter().map(|p| (p, false)))
        {
            let denied = comparable(resolved_path(&self.resolve(&path.to_string_lossy()))?);
            if resolved == denied
                || (prefix && resolved.starts_with(&denied))
                || (destructive && denied.starts_with(&resolved))
            {
                return Err(io::Error::new(
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
                ));
            }
        }
        Ok(())
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
                        fs::create_dir_all(dir)?;
                        let mut n = self.captured.len();
                        let (target, mut output) = loop {
                            let target = dir.join(n.to_string());
                            match fs::OpenOptions::new()
                                .create_new(true)
                                .write(true)
                                .open(&target)
                            {
                                Ok(file) => break (target, file),
                                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => n += 1,
                                Err(e) => return Err(e),
                            }
                        };
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
    fn write(&mut self, p: &Path, data: &[u8], append: bool) -> io::Result<()> {
        self.guard(p, true)?;
        self.capture(p)?;
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(!append)
            .append(append)
            .open(p)?;
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
                if a.mode.writable() || a.mode.create() {
                    self.guard(p, true)?;
                }
                reject_dir(p)?;
                if a.mode.create() {
                    self.capture(p)?;
                }
                let f = fs::OpenOptions::new()
                    .read(a.mode.readable())
                    .write(a.mode.writable())
                    .create(a.mode.create())
                    .truncate(a.mode.truncate())
                    .append(a.mode.is_append())
                    .open(p)?;
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
                    self.guard(p, true)?;
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
                self.guard_operation(p, true, true)?;
                self.capture(p)?;
                fs::remove_file(p)?;
                MontyObject::none()
            }
            OsFunctionCall::Rmdir(_) => {
                self.guard_operation(p, true, true)?;
                fs::remove_dir(p)?;
                MontyObject::none()
            }
            OsFunctionCall::Rename(a) => {
                self.guard_operation(p, true, true)?;
                let dst = self.resolve(&a.dst);
                self.guard_operation(&dst, true, true)?;
                self.capture(p)?;
                self.capture(&dst)?;
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
