use super::*;
use std::io::Write;

pub(super) fn durable_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let dir = path.parent().ok_or("Sync journal directory missing")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut file = tempfile::NamedTempFile::new_in(dir).map_err(|e| e.to_string())?;
    serde_json::to_writer(file.as_file_mut(), value).map_err(|e| e.to_string())?;
    file.flush().map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(path).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    std::fs::File::open(dir)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub(super) fn output(root: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let result = std::process::Command::new("git")
        .arg("--literal-pathspecs")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_AUTHOR_NAME", "Refact")
        .env("GIT_AUTHOR_EMAIL", "refact@localhost")
        .env("GIT_COMMITTER_NAME", "Refact")
        .env("GIT_COMMITTER_EMAIL", "refact@localhost")
        .current_dir(root)
        .output()
        .map_err(|e| e.to_string())?;
    if !result.status.success() {
        return Err(format!(
            "Git sync {}: {}",
            args.first().unwrap_or(&""),
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    Ok(result.stdout)
}

pub(super) fn text(root: &Path, args: &[&str]) -> Result<String, String> {
    String::from_utf8(output(root, args)?)
        .map(|s| s.trim().to_string())
        .map_err(|_| "Non UTF-8 Git output unsupported".into())
}

pub(super) fn head(root: &Path) -> Result<String, String> {
    text(root, &["rev-parse", "HEAD"])
}
pub(super) fn target(root: &Path, branch: &str) -> Result<String, String> {
    validate_branch_name(branch)?;
    text(
        root,
        &[
            "rev-parse",
            "--verify",
            &format!("refs/heads/{branch}^{{commit}}"),
        ],
    )
}

pub(super) fn safe_path(root: &Path, path: &str) -> Result<PathBuf, String> {
    if path.is_empty()
        || path.contains('\\')
        || path.contains('\0')
        || path
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == ".." || p.eq_ignore_ascii_case(".git"))
        || Path::new(path).is_absolute()
    {
        return Err(format!("Unsafe literal sync path: {path:?}"));
    }
    let mut current = root.to_path_buf();
    let parts: Vec<_> = path.split('/').collect();
    for part in &parts[..parts.len() - 1] {
        current.push(part);
        match std::fs::symlink_metadata(&current) {
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => (),
            Ok(_) => {
                return Err(format!(
                    "Sync path parent is not a plain directory: {}",
                    current.display()
                ))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(root.join(path))
}

pub(super) fn names(root: &Path, args: &[&str]) -> Result<Vec<String>, String> {
    output(root, args)?
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| {
            String::from_utf8(p.to_vec()).map_err(|_| "Non UTF-8 sync path unsupported".into())
        })
        .collect()
}

pub(super) fn conflicts(root: &Path) -> Result<Vec<WorktreeSyncConflict>, String> {
    let bytes = output(root, &["ls-files", "--unmerged", "-z"])?;
    let mut paths = std::collections::BTreeMap::<String, WorktreeSyncConflict>::new();
    for entry in bytes.split(|b| *b == 0).filter(|p| !p.is_empty()) {
        let tab = entry
            .iter()
            .position(|b| *b == b'\t')
            .ok_or("Invalid unmerged index record")?;
        let fields = std::str::from_utf8(&entry[..tab])
            .map_err(|e| e.to_string())?
            .split_whitespace()
            .collect::<Vec<_>>();
        if fields.len() != 3 {
            return Err("Invalid unmerged index fields".into());
        }
        let path = String::from_utf8(entry[tab + 1..].to_vec())
            .map_err(|_| "Non UTF-8 conflict path unsupported")?;
        let stage = WorktreeSyncStage {
            oid: fields[1].into(),
            mode: u32::from_str_radix(fields[0], 8).map_err(|e| e.to_string())?,
        };
        let conflict = paths.entry(path.clone()).or_insert(WorktreeSyncConflict {
            path,
            kind: "unmerged".into(),
            base: None,
            source: None,
            target: None,
        });
        match fields[2] {
            "1" => conflict.base = Some(stage),
            "2" => conflict.source = Some(stage),
            "3" => conflict.target = Some(stage),
            _ => return Err("Unknown conflict stage".into()),
        }
    }
    for c in paths.values_mut() {
        c.kind = match (&c.base, &c.source, &c.target) {
            (None, Some(_), Some(_)) => "add_add",
            (Some(_), None, Some(_)) | (Some(_), Some(_), None) => "delete_modify",
            _ => "content",
        }
        .into();
    }
    Ok(paths.into_values().collect())
}

#[derive(Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub(super) struct SavedFile {
    path: PathBuf,
    bytes: Vec<u8>,
    digest: String,
    mode: u32,
    kind: String,
}

const MAX_RECOVERY_ENTRIES: usize = 10_000;

fn file_digest(path: &Path) -> Result<String, String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = f.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        digest.update(&buffer[..n]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn capture(
    root: &Path,
    relative: &Path,
    files: &mut Vec<SavedFile>,
    total: &mut usize,
) -> Result<(), String> {
    capture_entry(root, relative, files, total, true)
}

fn capture_entry(
    root: &Path,
    relative: &Path,
    files: &mut Vec<SavedFile>,
    total: &mut usize,
    recursive: bool,
) -> Result<(), String> {
    if files.iter().any(|f| f.path == relative) {
        return Ok(());
    }
    let path = root.join(relative);
    let (bytes, mode, kind) = match std::fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Vec::new(), 0, "absent"),
        Err(e) => return Err(e.to_string()),
        Ok(meta) => {
            #[cfg(unix)]
            let mode = {
                use std::os::unix::fs::PermissionsExt;
                meta.permissions().mode()
            };
            #[cfg(not(unix))]
            let mode = 0;
            if meta.file_type().is_symlink() {
                #[cfg(unix)]
                {
                    use std::os::unix::ffi::OsStrExt;
                    (
                        std::fs::read_link(&path)
                            .map_err(|e| e.to_string())?
                            .as_os_str()
                            .as_bytes()
                            .to_vec(),
                        mode,
                        "symlink",
                    )
                }
                #[cfg(not(unix))]
                {
                    return Err("Symlink recovery unsupported on this platform".into());
                }
            } else if meta.is_dir() {
                if path.join(".git").exists() {
                    return Err("Nested repository obstructs recovery; move it outside the checkout before forced abort".into());
                }
                (Vec::new(), mode, "directory")
            } else if meta.is_file() {
                (Vec::new(), mode, "file")
            } else {
                return Err("Special file obstructs recovery; move it outside the checkout before forced abort".into());
            }
        }
    };
    *total += bytes.len();
    if files.len() >= MAX_RECOVERY_ENTRIES {
        return Err("Recovery exceeds bounded archive capacity (10000 entries); preserve the obstructing directory before retrying native abort".into());
    }
    let directory = kind == "directory";
    let digest = if kind == "file" {
        file_digest(&path)?
    } else {
        format!("{:x}", Sha256::digest(&bytes))
    };
    files.push(SavedFile {
        path: relative.into(),
        digest,
        bytes,
        mode,
        kind: kind.into(),
    });
    if directory && recursive {
        let mut entries = std::fs::read_dir(&path)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            capture(root, &relative.join(entry.file_name()), files, total)?;
        }
    }
    Ok(())
}

#[derive(serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub(super) struct Snapshot {
    schema_version: u32,
    head: String,
    worktree: Vec<SavedFile>,
    index: Vec<u8>,
    metadata: Vec<SavedFile>,
    stages: Vec<(String, Vec<u8>)>,
}

pub(super) fn snapshot(root: &Path) -> Result<Snapshot, String> {
    snapshot_paths(root, &[])
}

fn snapshot_paths(root: &Path, additional: &[PathBuf]) -> Result<Snapshot, String> {
    let mut paths = names(root, &["diff", "HEAD", "--name-only", "--no-renames", "-z"])?;
    paths.extend(conflicts(root)?.into_iter().map(|c| c.path));
    paths.extend(additional.iter().map(|p| p.to_string_lossy().into_owned()));
    paths.sort();
    paths.dedup();
    let mut worktree = Vec::new();
    let mut total = 0;
    for path in paths {
        safe_path(root, &path)?;
        capture(root, Path::new(&path), &mut worktree, &mut total)?;
    }
    worktree.sort_by(|a, b| a.path.cmp(&b.path));
    let repo = git2::Repository::open(root).map_err(|e| e.to_string())?;
    let mut metadata = Vec::new();
    for name in [
        "index",
        "HEAD",
        "ORIG_HEAD",
        "MERGE_HEAD",
        "MERGE_MSG",
        "MERGE_MODE",
        "AUTO_MERGE",
        "MERGE_AUTOSTASH",
    ] {
        capture(repo.path(), Path::new(name), &mut metadata, &mut total)?;
        let saved = metadata.last_mut().ok_or("Missing metadata capture")?;
        if saved.kind == "file" {
            saved.bytes = std::fs::read(repo.path().join(name)).map_err(|e| e.to_string())?;
        }
    }
    let index = output(root, &["ls-files", "--stage", "-z"])?;
    let mut stages = Vec::new();
    for c in conflicts(root)? {
        for stage in [c.base, c.source, c.target].into_iter().flatten() {
            if stage.mode == 0o160000 {
                return Err(
                    "Submodule conflict recovery unsupported; preserve nested work before abort"
                        .into(),
                );
            }
            if !stages.iter().any(|(existing, _)| existing == &stage.oid) {
                stages.push((stage.oid, Vec::new()));
            }
        }
    }
    Ok(Snapshot {
        schema_version: 1,
        head: head(root)?,
        worktree,
        index,
        metadata,
        stages,
    })
}

fn snapshot_fingerprint(mut state: Snapshot) -> Result<String, String> {
    state.metadata.retain(|f| f.path != Path::new("index"));
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&state).map_err(|e| e.to_string())?)
    ))
}

pub(super) fn fingerprint(root: &Path) -> Result<String, String> {
    snapshot_fingerprint(snapshot(root)?)
}

pub(super) fn archive(root: &Path, path: &Path) -> Result<(), String> {
    let before = snapshot(root)?;
    let blobs = path.with_extension("files");
    std::fs::create_dir(&blobs).map_err(|e| e.to_string())?;
    for file in &before.worktree {
        if file.kind != "file" {
            continue;
        }
        let destination = blobs.join(&file.digest);
        if !destination.exists() {
            let mut input =
                std::fs::File::open(root.join(&file.path)).map_err(|e| e.to_string())?;
            let mut output = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&destination)
                .map_err(|e| e.to_string())?;
            std::io::copy(&mut input, &mut output).map_err(|e| e.to_string())?;
            output.sync_all().map_err(|e| e.to_string())?;
        }
        if file_digest(&destination)? != file.digest {
            return Err("Recovery copy digest mismatch; source retained".into());
        }
    }
    for (oid, _) in &before.stages {
        let destination = blobs.join(format!("object-{oid}"));
        let file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&destination)
            .map_err(|e| e.to_string())?;
        let sync = file.try_clone().map_err(|e| e.to_string())?;
        let status = std::process::Command::new("git")
            .args(["cat-file", "blob", oid])
            .current_dir(root)
            .stdout(file)
            .status()
            .map_err(|e| e.to_string())?;
        if !status.success() {
            return Err("Failed to archive conflict stage object".into());
        }
        sync.sync_all().map_err(|e| e.to_string())?;
        if text(
            root,
            &[
                "hash-object",
                "--",
                destination.to_str().ok_or("Non UTF-8 recovery path")?,
            ],
        )? != *oid
        {
            return Err("Recovery conflict object verification failed".into());
        }
    }
    #[cfg(unix)]
    std::fs::File::open(&blobs)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())?;
    durable_json(path, &before)?;
    let read: Snapshot = serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if read != before || snapshot(root)? != before {
        return Err(
            "Recovery archive verification failed or checkout changed; no reset performed".into(),
        );
    }
    Ok(())
}

#[cfg(test)]
thread_local! {
    pub(super) static ARCHIVE_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn read_archive(path: &Path) -> Result<Snapshot, String> {
    #[cfg(test)]
    ARCHIVE_READS.with(|n| n.set(n.get() + 1));
    let saved: Snapshot = serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if saved.schema_version != 1
        || saved.worktree.iter().chain(&saved.metadata).any(|f| {
            f.digest.len() != 64
                || !f.digest.bytes().all(|b| b.is_ascii_hexdigit())
                || ((f.kind != "file" || !f.bytes.is_empty())
                    && f.digest != format!("{:x}", Sha256::digest(&f.bytes)))
        })
    {
        return Err("Recovery archive schema or digest invalid".into());
    }
    for file in &saved.worktree {
        if file.kind == "file"
            && file_digest(&path.with_extension("files").join(&file.digest))? != file.digest
        {
            return Err("Recovery file digest mismatch".into());
        }
    }
    Ok(saved)
}

pub(super) fn recovery_fingerprint(root: &Path, archive: &Path) -> Result<String, String> {
    let saved = read_archive(archive)?;
    let paths = saved
        .worktree
        .iter()
        .filter(|f| {
            !saved.worktree.iter().any(|parent| {
                parent.kind == "directory"
                    && parent.path != f.path
                    && f.path.starts_with(&parent.path)
            })
        })
        .map(|f| f.path.clone())
        .collect::<Vec<_>>();
    snapshot_fingerprint(snapshot_paths(root, &paths)?)
}

fn entry_identity(root: &Path, file: &SavedFile) -> Result<String, String> {
    let mut files = Vec::new();
    let mut total = 0;
    capture_entry(root, &file.path, &mut files, &mut total, false)?;
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&files).map_err(|e| e.to_string())?)
    ))
}

fn expected_identity(file: &SavedFile) -> Result<String, String> {
    let mut copy = SavedFile {
        path: file.path.clone(),
        bytes: file.bytes.clone(),
        digest: file.digest.clone(),
        mode: file.mode,
        kind: file.kind.clone(),
    };
    if copy.kind == "file" {
        copy.bytes.clear();
    }
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&vec![copy]).map_err(|e| e.to_string())?)
    ))
}

pub(super) struct CachedRestore {
    saved: Snapshot,
    archive: PathBuf,
    staged: Vec<SavedFile>,
}

impl CachedRestore {
    pub(super) fn load(archive: &Path) -> Result<Self, String> {
        let saved = read_archive(archive)?;
        let mut staged = saved.worktree.clone();
        #[cfg(unix)]
        for file in &mut staged {
            if file.kind == "directory" {
                file.mode |= 0o700;
            }
        }
        Ok(Self {
            saved,
            archive: archive.into(),
            staged,
        })
    }

    pub(super) fn prepare_abort_directories(
        &self,
        root: &Path,
        validate: impl Fn() -> Result<(), String>,
    ) -> Result<(), String> {
        validate()?;
        for (saved, staged) in self.saved.worktree.iter().zip(&self.staged) {
            let actual = self.identity(root, saved)?;
            if actual != expected_identity(saved)? && actual != expected_identity(staged)? {
                return Err("Abort managed path changed after archive; retained".into());
            }
        }
        #[cfg(unix)]
        for (saved, staged) in self.saved.worktree.iter().zip(&self.staged) {
            if saved.kind != "directory" || saved.mode == staged.mode {
                continue;
            }
            validate()?;
            let path = safe_path(root, saved.path.to_str().ok_or("Invalid recovery path")?)?;
            let actual = entry_identity(root, saved)?;
            if actual != expected_identity(saved)? && actual != expected_identity(staged)? {
                return Err("Abort directory changed after archive; retained".into());
            }
            for entry in std::fs::read_dir(&path).map_err(|e| e.to_string())? {
                let relative = entry
                    .map_err(|e| e.to_string())?
                    .path()
                    .strip_prefix(root)
                    .map_err(|e| e.to_string())?
                    .to_path_buf();
                if !self.saved.worktree.iter().any(|f| f.path == relative) {
                    return Err("Abort directory has newer unknown child; retained".into());
                }
            }
            use std::os::unix::fs::PermissionsExt;
            let directory = std::fs::File::open(path).map_err(|e| e.to_string())?;
            validate()?;
            if entry_identity(root, saved)? != actual {
                return Err("Abort directory changed before permission staging".into());
            }
            directory
                .set_permissions(std::fs::Permissions::from_mode(staged.mode))
                .map_err(|e| e.to_string())?;
            directory.sync_all().map_err(|e| e.to_string())?;
            validate()?;
        }
        Ok(())
    }

    fn identity(&self, root: &Path, file: &SavedFile) -> Result<String, String> {
        for ancestor in file
            .path
            .ancestors()
            .skip(1)
            .filter(|p| !p.as_os_str().is_empty())
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            if self
                .saved
                .worktree
                .iter()
                .any(|f| f.path == ancestor && f.kind == "directory")
            {
                let path = safe_path(root, ancestor.to_str().ok_or("Invalid recovery path")?)?;
                if std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file()) {
                    return expected_identity(&SavedFile {
                        path: file.path.clone(),
                        bytes: Vec::new(),
                        digest: format!("{:x}", Sha256::digest([])),
                        mode: 0,
                        kind: "absent".into(),
                    });
                }
            }
        }
        entry_identity(root, file)
    }

    pub(super) fn preimages(&self, root: &Path, git_dir: &Path) -> Result<Vec<String>, String> {
        self.entries(root, git_dir)
            .into_iter()
            .map(|(base, file, blob, finalize)| {
                if finalize {
                    let staged = self
                        .staged
                        .iter()
                        .find(|f| f.path == file.path)
                        .ok_or("Missing staged directory")?;
                    expected_identity(staged)
                } else if blob {
                    self.identity(base, file)
                } else {
                    entry_identity(base, file)
                }
            })
            .collect()
    }

    pub(super) fn validate_entries(
        &self,
        root: &Path,
        git_dir: &Path,
        preimages: &[String],
        cursor: usize,
    ) -> Result<(), String> {
        let entries = self.entries(root, git_dir);
        if entries.len() != preimages.len() || cursor > entries.len() {
            return Err("Invalid restore progress".into());
        }
        for (i, (base, file, blob, finalize)) in entries.iter().enumerate() {
            if *finalize && i > cursor {
                continue;
            }
            if !*finalize
                && entries
                    .iter()
                    .enumerate()
                    .any(|(j, (_, other, _, final_mode))| {
                        *final_mode && other.path == file.path && j <= cursor
                    })
            {
                continue;
            }
            let actual = if *blob {
                self.identity(base, file)?
            } else {
                entry_identity(base, file)?
            };
            if actual != expected_identity(file)?
                && (i < cursor
                    || (actual != preimages[i]
                        && !(file.kind == "directory"
                            && !base.join(&file.path).exists()
                            && self.directory_intermediate(file, &preimages[i])?)))
            {
                return Err(format!(
                    "Restore path {} changed outside its known pre/post images",
                    file.path.display()
                ));
            }
            if *blob && file.kind == "directory" && base.join(&file.path).is_dir() {
                for entry in std::fs::read_dir(base.join(&file.path)).map_err(|e| e.to_string())? {
                    let entry = entry.map_err(|e| e.to_string())?;
                    let relative = entry
                        .path()
                        .strip_prefix(base)
                        .map_err(|e| e.to_string())?
                        .to_path_buf();
                    if !self.saved.worktree.iter().any(|f| f.path == relative) {
                        return Err("Restore directory has newer unknown child; retained".into());
                    }
                }
            }
        }
        Ok(())
    }

    fn directory_backup(&self, file: &SavedFile, preimage: &str) -> PathBuf {
        self.archive.with_extension("files").join(format!(
            "displaced-{:x}-{preimage}",
            Sha256::digest(file.path.to_string_lossy().as_bytes())
        ))
    }

    fn directory_intermediate(&self, file: &SavedFile, preimage: &str) -> Result<bool, String> {
        if file.kind != "directory" {
            return Ok(false);
        }
        let backup = self.directory_backup(file, preimage);
        if !backup.exists() {
            return Ok(false);
        }
        let mut files = Vec::new();
        capture_entry(
            backup.parent().ok_or("Missing backup parent")?,
            Path::new(backup.file_name().ok_or("Missing backup name")?),
            &mut files,
            &mut 0,
            false,
        )?;
        let Some(mut saved) = files.pop() else {
            return Ok(false);
        };
        if saved.kind != "file" {
            return Err("Invalid displaced directory preimage".into());
        }
        saved.path = file.path.clone();
        Ok(expected_identity(&saved)? == preimage)
    }

    fn entries<'a>(
        &'a self,
        root: &'a Path,
        git_dir: &'a Path,
    ) -> Vec<(&'a Path, &'a SavedFile, bool, bool)> {
        self.staged
            .iter()
            .map(|f| (root, f, true, false))
            .chain(
                self.saved
                    .metadata
                    .iter()
                    .filter(|f| f.path != Path::new("HEAD"))
                    .map(|f| (git_dir, f, false, false)),
            )
            .chain(
                self.saved
                    .worktree
                    .iter()
                    .rev()
                    .filter(|f| {
                        self.staged
                            .iter()
                            .any(|staged| staged.path == f.path && staged.mode != f.mode)
                    })
                    .map(|f| (root, f, true, true)),
            )
            .collect()
    }

    pub(super) fn step(
        &self,
        root: &Path,
        git_dir: &Path,
        cursor: usize,
        preimage: &str,
        validate: impl Fn() -> Result<(), String>,
    ) -> Result<bool, String> {
        let archive = &self.archive;
        let entry = self.entries(root, git_dir).get(cursor).copied();
        let Some((base, file, blob, finalize)) = entry else {
            return Ok(false);
        };
        validate()?;
        let actual = entry_identity(base, file)?;
        if file.kind == "directory" && base.join(&file.path).is_dir() {
            for entry in std::fs::read_dir(base.join(&file.path)).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                let relative = entry
                    .path()
                    .strip_prefix(base)
                    .map_err(|e| e.to_string())?
                    .to_path_buf();
                if !self.saved.worktree.iter().any(|f| f.path == relative) {
                    return Err("Restore directory has newer unknown child; retained".into());
                }
            }
        }
        if actual == expected_identity(file)? {
            return Ok(true);
        }
        if actual != preimage
            && !(file.kind == "directory"
                && !base.join(&file.path).exists()
                && self.directory_intermediate(file, preimage)?)
        {
            return Err(format!("Restore path {} changed outside its known pre/post images; force Abort preserves it before rollback", file.path.display()));
        }
        let path = safe_path(base, file.path.to_str().ok_or("Invalid recovery path")?)?;
        if !blob
            && ![
                "index",
                "ORIG_HEAD",
                "MERGE_HEAD",
                "MERGE_MSG",
                "MERGE_MODE",
                "AUTO_MERGE",
                "MERGE_AUTOSTASH",
            ]
            .iter()
            .any(|n| file.path == Path::new(n))
        {
            return Err("Unexpected recovery metadata path".into());
        }
        if finalize {
            validate()?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let directory = std::fs::File::open(&path).map_err(|e| e.to_string())?;
                directory
                    .set_permissions(std::fs::Permissions::from_mode(file.mode))
                    .map_err(|e| e.to_string())?;
                directory.sync_all().map_err(|e| e.to_string())?;
            }
            validate()?;
            return Ok(true);
        }
        match file.kind.as_str() {
            "file" => {
                let parent = path.parent().ok_or("Missing restore parent")?;
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                let mut temporary =
                    tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
                if blob {
                    if file_digest(&archive.with_extension("files").join(&file.digest))?
                        != file.digest
                    {
                        return Err("Recovery file digest mismatch".into());
                    }
                    let mut input =
                        std::fs::File::open(archive.with_extension("files").join(&file.digest))
                            .map_err(|e| e.to_string())?;
                    std::io::copy(&mut input, temporary.as_file_mut())
                        .map_err(|e| e.to_string())?;
                } else {
                    temporary
                        .write_all(&file.bytes)
                        .map_err(|e| e.to_string())?;
                }
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    temporary
                        .as_file()
                        .set_permissions(std::fs::Permissions::from_mode(file.mode))
                        .map_err(|e| e.to_string())?;
                }
                temporary.as_file().sync_all().map_err(|e| e.to_string())?;
                if file_digest(temporary.path())? != file.digest {
                    return Err("Prepared restore file digest mismatch".into());
                }
                validate()?;
                temporary.persist(&path).map_err(|e| e.to_string())?;
            }
            "absent" => {
                validate()?;
                match std::fs::remove_file(&path) {
                    Ok(()) => (),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                    Err(e) => return Err(e.to_string()),
                }
            }
            "directory" => {
                if path.is_dir() {
                    validate()?;
                    if entry_identity(base, file)? != actual {
                        return Err("Restore directory changed before staging".into());
                    }
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let directory = std::fs::File::open(&path).map_err(|e| e.to_string())?;
                        directory
                            .set_permissions(std::fs::Permissions::from_mode(file.mode))
                            .map_err(|e| e.to_string())?;
                        directory.sync_all().map_err(|e| e.to_string())?;
                    }
                    return Ok(true);
                }
                let parent = path.parent().ok_or("Missing restore parent")?;
                let temporary = tempfile::tempdir_in(parent).map_err(|e| e.to_string())?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(
                        temporary.path(),
                        std::fs::Permissions::from_mode(file.mode),
                    )
                    .map_err(|e| e.to_string())?;
                    std::fs::File::open(temporary.path())
                        .and_then(|f| f.sync_all())
                        .map_err(|e| e.to_string())?;
                }
                validate()?;
                if path.is_file() {
                    let backup = self.directory_backup(file, preimage);
                    let mut copy = tempfile::NamedTempFile::new_in(
                        backup.parent().ok_or("Missing recovery directory")?,
                    )
                    .map_err(|e| e.to_string())?;
                    std::io::copy(
                        &mut std::fs::File::open(&path).map_err(|e| e.to_string())?,
                        copy.as_file_mut(),
                    )
                    .map_err(|e| e.to_string())?;
                    copy.as_file()
                        .set_permissions(
                            std::fs::metadata(&path)
                                .map_err(|e| e.to_string())?
                                .permissions(),
                        )
                        .map_err(|e| e.to_string())?;
                    copy.as_file().sync_all().map_err(|e| e.to_string())?;
                    copy.persist(&backup).map_err(|e| e.to_string())?;
                    #[cfg(unix)]
                    std::fs::File::open(backup.parent().ok_or("Missing recovery directory")?)
                        .and_then(|f| f.sync_all())
                        .map_err(|e| e.to_string())?;
                    if !self.directory_intermediate(file, preimage)? {
                        return Err("Directory displacement preimage mismatch".into());
                    }
                    validate()?;
                    if entry_identity(base, file)? != preimage {
                        return Err("Directory parent changed while preparing displacement".into());
                    }
                    std::fs::remove_file(&path).map_err(|e| e.to_string())?;
                    #[cfg(unix)]
                    std::fs::File::open(parent)
                        .and_then(|f| f.sync_all())
                        .map_err(|e| e.to_string())?;
                }
                validate()?;
                std::fs::rename(temporary.path(), &path).map_err(|e| e.to_string())?;
            }
            "symlink" => {
                #[cfg(unix)]
                {
                    use std::os::unix::ffi::OsStringExt;
                    let parent = path.parent().ok_or("Missing restore parent")?;
                    let temporary = tempfile::NamedTempFile::new_in(parent)
                        .map_err(|e| e.to_string())?
                        .into_temp_path();
                    std::fs::remove_file(&temporary).map_err(|e| e.to_string())?;
                    std::os::unix::fs::symlink(
                        std::ffi::OsString::from_vec(file.bytes.clone()),
                        &temporary,
                    )
                    .map_err(|e| e.to_string())?;
                    validate()?;
                    std::fs::rename(&temporary, &path).map_err(|e| e.to_string())?;
                }
                #[cfg(not(unix))]
                return Err("Symlink restore unsupported".into());
            }
            _ => return Err("Invalid recovery entry type".into()),
        }
        #[cfg(unix)]
        std::fs::File::open(path.parent().ok_or("Missing restore parent")?)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())?;
        Ok(true)
    }
}

#[cfg(test)]
pub(super) fn restore_preimages(
    root: &Path,
    archive: &Path,
    git_dir: &Path,
) -> Result<Vec<String>, String> {
    CachedRestore::load(archive)?.preimages(root, git_dir)
}
#[cfg(test)]
pub(super) fn restore_step(
    root: &Path,
    archive: &Path,
    git_dir: &Path,
    cursor: usize,
    preimage: &str,
    validate: impl Fn() -> Result<(), String>,
) -> Result<bool, String> {
    CachedRestore::load(archive)?.step(root, git_dir, cursor, preimage, validate)
}
