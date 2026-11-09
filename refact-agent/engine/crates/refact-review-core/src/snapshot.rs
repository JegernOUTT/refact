use crate::*;
use std::{collections::BTreeMap, path::Path, io::Read};
type Files = BTreeMap<String, (Vec<u8>, u32)>;
impl ReviewStore {
    pub fn capture(&self, repository: &Path, request: CaptureRequest) -> Result<Candidate> {
        let root = std::fs::canonicalize(repository)?;
        let repo = git2::Repository::open(&root)?;
        let base = repo.find_commit(git2::Oid::from_str(&request.base_oid)?)?;
        let head = repo.find_commit(git2::Oid::from_str(&request.head_oid)?)?;
        let mut base_files = Files::new();
        let mut head_files = Files::new();
        tree_files(&repo, &base.tree()?, "", &mut base_files)?;
        tree_files(&repo, &head.tree()?, "", &mut head_files)?;
        if request.policy.staged || request.policy.working || request.policy.untracked {
            let checkout = repo
                .workdir()
                .ok_or_else(|| invalid("workdir required"))?
                .canonicalize()?;
            if checkout != root {
                return Err(invalid("capture requires repository workdir root"));
            }
            if repo.head()?.target() != Some(head.id()) {
                return Err(invalid("overlay head must equal checkout HEAD"));
            }
            let index = repo.index()?;
            if index.has_conflicts() {
                return Err(invalid("unmerged index"));
            }
            if request.policy.staged {
                head_files.clear();
                for e in index.iter() {
                    let p = std::str::from_utf8(&e.path).map_err(|_| invalid("non UTF8 path"))?;
                    if p.split('/')
                        .any(|part| part.eq_ignore_ascii_case(".refact"))
                    {
                        continue;
                    }
                    relative(p)?;
                    mode(e.mode)?;
                    let b = repo.find_blob(e.id)?;
                    insert(&mut head_files, p, b.content().to_vec(), e.mode)?;
                }
            }
            if request.policy.working {
                let mut paths: std::collections::BTreeSet<String> =
                    head_files.keys().cloned().collect();
                for e in index.iter() {
                    paths.insert(
                        std::str::from_utf8(&e.path)
                            .map_err(|_| invalid("non UTF8 path"))?
                            .to_owned(),
                    );
                }
                for p in paths {
                    if p.split('/')
                        .any(|part| part.eq_ignore_ascii_case(".refact"))
                    {
                        continue;
                    }
                    match read_working(&root, &p)? {
                        Some((b, m)) => insert(&mut head_files, &p, b, m)?,
                        None => {
                            head_files.remove(&p);
                        }
                    }
                }
            }
            if request.policy.untracked {
                let mut options = git2::StatusOptions::new();
                options
                    .include_untracked(true)
                    .recurse_untracked_dirs(true)
                    .include_ignored(false);
                for entry in repo.statuses(Some(&mut options))?.iter() {
                    if !entry.status().contains(git2::Status::WT_NEW) {
                        continue;
                    }
                    let p = entry.path().ok_or_else(|| invalid("non UTF8 path"))?;
                    if p.split('/').any(|x| x.eq_ignore_ascii_case(".refact")) {
                        continue;
                    }
                    if let Some((b, m)) = read_working(&root, p)? {
                        insert(&mut head_files, p, b, m)?;
                    }
                }
            }
            if index_identity(&repo.index()?) != index_identity(&index)
                || repo.head()?.target() != Some(head.id())
            {
                return Err(invalid("checkout changed during capture; retry"));
            }
        }
        let total: u64 = base_files
            .values()
            .chain(head_files.values())
            .map(|(b, _)| b.len() as u64)
            .sum();
        if total > MAX_CANDIDATE_BYTES {
            return Err(Error::Limit("candidate exceeds 256MiB".into()));
        }
        let paths: std::collections::BTreeSet<_> = base_files
            .keys()
            .chain(head_files.keys())
            .cloned()
            .collect();
        if paths.len() > MAX_FILES {
            return Err(Error::Limit("candidate exceeds 20000 files".into()));
        }
        let mut files = Vec::new();
        for path in paths {
            let b = base_files.remove(&path);
            let h = head_files.remove(&path);
            files.push(CandidateFile {
                path,
                base: b.as_ref().map(|(b, _)| self.put_blob(b)).transpose()?,
                head: h.as_ref().map(|(b, _)| self.put_blob(b)).transpose()?,
                base_mode: b.map(|(_, m)| m),
                head_mode: h.map(|(_, m)| m),
            });
        }
        let content_digest = content_hash(&serde_json::to_vec(&files)?);
        let candidate = Candidate {
            id: uuid::Uuid::new_v4().to_string(),
            base_oid: base.id().to_string(),
            head_oid: head.id().to_string(),
            content_digest,
            snapshot_scope: "full regular-file base/head trees excluding case-insensitive .refact components; symlinks and submodules refused".into(),
            created_at_ms: now_ms(),
            repository: root.to_string_lossy().into_owned(),
            origin: request.origin,
            policy: request.policy,
            files,
        };
        self.connection
            .lock()
            .map_err(|_| invalid("poisoned store"))?
            .execute(
                "INSERT INTO captures VALUES(?1,?2)",
                rusqlite::params![candidate.id, serde_json::to_string(&candidate)?],
            )?;
        Ok(candidate)
    }
    pub fn file(&self, candidate: &Candidate, path: &str) -> Result<FileView> {
        relative(path)?;
        let file = candidate
            .files
            .iter()
            .find(|f| f.path == path)
            .ok_or(Error::NotFound)?;
        let base = file.base.as_ref().map(|b| self.read_blob(b)).transpose()?;
        let head = file.head.as_ref().map(|b| self.read_blob(b)).transpose()?;
        let diff = match (
            std::str::from_utf8(base.as_deref().unwrap_or_default()),
            std::str::from_utf8(head.as_deref().unwrap_or_default()),
        ) {
            (Ok(b), Ok(h)) if !b.contains('\0') && !h.contains('\0') => {
                similar::TextDiff::from_lines(b, h)
                    .unified_diff()
                    .header(&format!("a/{path}"), &format!("b/{path}"))
                    .to_string()
            }
            _ => "Binary content differs; full bytes available".to_owned(),
        };
        Ok(FileView {
            path: path.to_owned(),
            base,
            head,
            diff,
        })
    }
}
fn index_identity(index: &git2::Index) -> Vec<(Vec<u8>, String, u32, u16)> {
    index
        .iter()
        .map(|e| (e.path, e.id.to_string(), e.mode, e.flags))
        .collect()
}
fn mode(m: u32) -> Result<()> {
    if m == 0o100644 || m == 0o100755 {
        Ok(())
    } else {
        Err(invalid("symlinks/submodules/special modes unsupported"))
    }
}
fn insert(files: &mut Files, p: &str, b: Vec<u8>, m: u32) -> Result<()> {
    relative(p)?;
    mode(m)?;
    if b.len() > MAX_BLOB_BYTES {
        return Err(Error::Limit("file exceeds 16MiB".into()));
    }
    files.insert(p.to_owned(), (b, m));
    if files.len() > MAX_FILES
        || files.values().map(|(b, _)| b.len() as u64).sum::<u64>() > MAX_CANDIDATE_BYTES
    {
        return Err(Error::Limit("snapshot bounds exceeded".into()));
    }
    Ok(())
}
fn tree_files(
    repo: &git2::Repository,
    tree: &git2::Tree<'_>,
    prefix: &str,
    files: &mut Files,
) -> Result<()> {
    for entry in tree.iter() {
        let name = entry.name().ok_or_else(|| invalid("non UTF8 path"))?;
        let p = format!("{prefix}{name}");
        if name.eq_ignore_ascii_case(".refact") {
            continue;
        }
        relative(&p)?;
        if entry.kind() == Some(git2::ObjectType::Tree) {
            tree_files(repo, &repo.find_tree(entry.id())?, &format!("{p}/"), files)?;
        } else {
            mode(entry.filemode() as u32)?;
            let blob = repo.find_blob(entry.id())?;
            if blob.size() > MAX_BLOB_BYTES {
                return Err(Error::Limit("file exceeds 16MiB".into()));
            }
            insert(files, &p, blob.content().to_vec(), entry.filemode() as u32)?;
        }
    }
    Ok(())
}
fn read_working(root: &Path, p: &str) -> Result<Option<(Vec<u8>, u32)>> {
    relative(p)?;
    #[cfg(target_os = "linux")]
    let file = {
        use std::os::fd::{AsRawFd, FromRawFd};
        let mut dir = std::fs::File::open(root)?;
        let parts: Vec<_> = p.split('/').collect();
        for (i, part) in parts.iter().enumerate() {
            let name = std::ffi::CString::new(*part).map_err(|_| invalid("path NUL"))?;
            let flags = libc::O_RDONLY
                | libc::O_NOFOLLOW
                | libc::O_CLOEXEC
                | if i + 1 < parts.len() {
                    libc::O_DIRECTORY
                } else {
                    libc::O_NONBLOCK
                };
            let fd = unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), flags) };
            if fd < 0 {
                let e = std::io::Error::last_os_error();
                if e.kind() == std::io::ErrorKind::NotFound {
                    return Ok(None);
                }
                return Err(e.into());
            }
            dir = unsafe { std::fs::File::from_raw_fd(fd) };
        }
        dir
    };
    #[cfg(not(target_os = "linux"))]
    let file = {
        let mut full = root.to_owned();
        for part in p.split('/') {
            full.push(part);
            match std::fs::symlink_metadata(&full) {
                Ok(m) if m.file_type().is_symlink() => return Err(invalid("symlink path")),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(e) => return Err(e.into()),
                _ => {}
            }
        }
        std::fs::File::open(full)?
    };
    let before = file.metadata()?;
    if !before.is_file() {
        return Err(invalid("not regular file"));
    }
    if before.len() > MAX_BLOB_BYTES as u64 {
        return Err(Error::Limit("file exceeds 16MiB".into()));
    }
    let mut bytes = vec![];
    (&file)
        .take(MAX_BLOB_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    let after = file.metadata()?;
    if bytes.len() > MAX_BLOB_BYTES
        || before.len() != after.len()
        || before.modified()? != after.modified()?
    {
        return Err(invalid("file changed during capture"));
    }
    #[cfg(unix)]
    let executable = {
        use std::os::unix::fs::PermissionsExt;
        before.permissions().mode() & 0o111 != 0
    };
    #[cfg(not(unix))]
    let executable = false;
    Ok(Some((bytes, if executable { 0o100755 } else { 0o100644 })))
}
