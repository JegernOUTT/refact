//! Read-only git data for Python scripts (`git()`), on the Git rail's read functions.

use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use git2::{Delta, Repository};
use serde_json::{json, Map, Value};

use crate::read::{
    branches_of, commit_touches_path, head_status, literal_path_diff_options, exact_path,
    read_status_snapshot_with, ref_labels_by_oid, ref_labels_where, repo_state_name,
    resolve_commit_ref, validate_rev_input, GitRefLabel, StatusScan, MAX_FILE_BYTES,
    MAX_LOG_SCAN_COMMITS,
};

pub const SUBCOMMANDS: &[&str] = &["status", "diff", "log", "show", "branches"];
/// Keyword names the kernel gives the leading positional arguments of `git(...)`.
pub const POSITIONAL: &[&str] = &["subcommand", "#1", "#2"];
pub const MAX_DIFF_FILES: usize = 300;
pub const MAX_FILE_PATCH_BYTES: usize = 64 * 1024;
pub const MAX_TOTAL_PATCH_BYTES: usize = 512 * 1024;
/// Files larger than this are not loaded for a patch or a rename check; their patch is cut.
pub const MAX_DIFF_BLOB_BYTES: u64 = 1024 * 1024;
pub const DEFAULT_LOG_COMMITS: usize = 20;
pub const MAX_LOG_COMMITS: usize = 200;
pub const MAX_COMMIT_FILES: usize = 100;
pub const MAX_REFS: usize = 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitRequest {
    Status,
    Diff {
        rev: Option<String>,
        paths: Vec<String>,
        staged: bool,
    },
    Log {
        n: usize,
        rev: Option<String>,
        paths: Vec<String>,
    },
    Show {
        rev: String,
        path: Option<String>,
    },
    Branches,
}

/// How a call ends when it does not return data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitError {
    /// Bad arguments: raised as TypeError.
    Type(String),
    /// Refused by the privacy policy or engine rules: raised as PermissionError.
    Denied(String),
    /// The repository cannot answer: `ok: false` with this error.
    Failed(String),
    Cancelled,
}

pub struct GitOutput {
    pub data: Value,
    pub summary: String,
    /// Worktree paths of the files whose content (patch lines or text) the result carries, both
    /// sides of a rename included, for privacy records. Names alone add none, as for `tree`.
    pub paths: Vec<PathBuf>,
    pub truncated: bool,
}

pub struct GitContext<'a> {
    /// PROJECT_ROOT; the repository that holds it is read.
    pub root: &'a Path,
    /// The chat's workspace folders; nothing outside them is listed or read.
    pub folders: &'a [PathBuf],
    /// Whether a script may read the file at this worktree path.
    pub may_read: &'a (dyn Fn(&Path) -> bool + Sync),
    pub cancel: &'a AtomicBool,
}

impl GitRequest {
    pub fn subcommand(&self) -> &'static str {
        match self {
            Self::Status => "status",
            Self::Diff {
                ..
            } => "diff",
            Self::Log {
                ..
            } => "log",
            Self::Show {
                ..
            } => "show",
            Self::Branches => "branches",
        }
    }

    /// Checks the keyword arguments of one call; `Err` is the TypeError text.
    pub fn parse(kwargs: &Value) -> Result<Self, String> {
        let args = kwargs
            .as_object()
            .ok_or("git() arguments must be keywords")?;
        let subcommand = match args.get("subcommand") {
            None | Some(Value::Null) => {
                return Err("git() missing required argument: 'subcommand'".into())
            }
            Some(Value::String(name)) => name.as_str(),
            Some(_) => return Err("git() subcommand must be a str".into()),
        };
        if !SUBCOMMANDS.contains(&subcommand) {
            return Err(format!(
                "git(): unknown subcommand '{subcommand}'; read-only subcommands: {}",
                SUBCOMMANDS.join(", ")
            ));
        }
        let keywords: &[&str] = match subcommand {
            "diff" => &["rev", "path", "staged"],
            "log" => &["n", "rev", "path"],
            "show" => &["rev", "path"],
            _ => &[],
        };
        for key in args.keys().map(String::as_str) {
            if key == "subcommand" {
                continue;
            }
            if key.starts_with('#') {
                if subcommand != "show" {
                    return Err(format!(
                        "git('{subcommand}') takes no positional argument after the subcommand{}",
                        if keywords.is_empty() {
                            String::new()
                        } else {
                            format!("; pass {} as keywords", keywords.join("=, ") + "=")
                        }
                    ));
                }
                continue;
            }
            if !keywords.contains(&key) {
                return Err(format!(
                    "git('{subcommand}') got an unexpected keyword argument '{key}'"
                ));
            }
        }
        let value = |name: &str, positional: &str| -> Result<Option<&Value>, String> {
            match (
                args.get(name).filter(|v| !v.is_null()),
                args.get(positional).filter(|v| !v.is_null()),
            ) {
                (Some(_), Some(_)) => Err(format!(
                    "git('{subcommand}') got multiple values for argument '{name}'"
                )),
                (Some(value), None) | (None, Some(value)) => Ok(Some(value)),
                (None, None) => Ok(None),
            }
        };
        let rev = |allowed: RevForm| -> Result<Option<String>, String> {
            match value("rev", "#1")? {
                None => Ok(None),
                Some(Value::String(rev)) => check_rev(subcommand, rev, allowed).map(Some),
                Some(_) => Err("git(): rev must be a str".into()),
            }
        };
        let paths = || -> Result<Vec<String>, String> {
            let raw = match value("path", "#2")? {
                None => return Ok(Vec::new()),
                Some(Value::String(path)) => vec![path.as_str()],
                Some(Value::Array(items)) if !items.is_empty() => items
                    .iter()
                    .map(|item| item.as_str().ok_or(()))
                    .collect::<Result<Vec<_>, ()>>()
                    .map_err(|_| "git(): path must be a str or a list of str".to_string())?,
                Some(_) => return Err("git(): path must be a str or a list of str".into()),
            };
            let mut paths = Vec::new();
            for path in raw {
                let path = normalize_path(path)?;
                if path.is_empty() {
                    return Ok(Vec::new());
                }
                if !paths.contains(&path) {
                    paths.push(path);
                }
            }
            Ok(paths)
        };
        Ok(match subcommand {
            "status" => Self::Status,
            "branches" => Self::Branches,
            "diff" => {
                let staged = match args.get("staged") {
                    None | Some(Value::Null) => false,
                    Some(Value::Bool(staged)) => *staged,
                    Some(_) => return Err("git('diff'): staged must be a bool".into()),
                };
                let rev = rev(RevForm::Range {
                    symmetric: true,
                })?;
                if staged && rev.as_deref().is_some_and(|rev| rev.contains("..")) {
                    return Err("git('diff'): staged cannot be combined with a range".into());
                }
                Self::Diff {
                    rev,
                    paths: paths()?,
                    staged,
                }
            }
            "log" => {
                let n = match args.get("n") {
                    None | Some(Value::Null) => DEFAULT_LOG_COMMITS,
                    Some(Value::Number(n))
                        if n.as_u64()
                            .is_some_and(|n| (1..=MAX_LOG_COMMITS as u64).contains(&n)) =>
                    {
                        n.as_u64().unwrap() as usize
                    }
                    Some(_) => {
                        return Err(format!(
                            "git('log'): n must be an integer between 1 and {MAX_LOG_COMMITS}"
                        ))
                    }
                };
                Self::Log {
                    n,
                    rev: rev(RevForm::Range {
                        symmetric: false,
                    })?,
                    paths: paths()?,
                }
            }
            _ => {
                let rev =
                    rev(RevForm::Single)?.ok_or("git('show') missing required argument: 'rev'")?;
                let mut paths = paths()?;
                if paths.len() > 1 {
                    return Err("git('show'): path must be one str".into());
                }
                Self::Show {
                    rev,
                    path: paths.pop(),
                }
            }
        })
    }

    /// The call as hooks and the run card see it, after checking.
    pub fn to_kwargs(&self) -> Value {
        let mut args = Map::new();
        args.insert("subcommand".into(), json!(self.subcommand()));
        match self {
            Self::Diff {
                rev,
                paths,
                staged,
            } => {
                if let Some(rev) = rev {
                    args.insert("rev".into(), json!(rev));
                }
                if !paths.is_empty() {
                    args.insert("path".into(), json!(paths));
                }
                if *staged {
                    args.insert("staged".into(), json!(true));
                }
            }
            Self::Log {
                n,
                rev,
                paths,
            } => {
                args.insert("n".into(), json!(n));
                if let Some(rev) = rev {
                    args.insert("rev".into(), json!(rev));
                }
                if !paths.is_empty() {
                    args.insert("path".into(), json!(paths));
                }
            }
            Self::Show {
                rev,
                path,
            } => {
                args.insert("rev".into(), json!(rev));
                if let Some(path) = path {
                    args.insert("path".into(), json!(path));
                }
            }
            Self::Status | Self::Branches => {}
        }
        Value::Object(args)
    }

    /// A git-like command line for the run card, e.g. `diff --staged -- src/a.rs`.
    pub fn command_line(&self) -> String {
        let mut parts = vec![self.subcommand().to_string()];
        let mut paths: &[String] = &[];
        match self {
            Self::Diff {
                rev,
                paths: p,
                staged,
            } => {
                if *staged {
                    parts.push("--staged".into());
                }
                parts.extend(rev.clone());
                paths = p;
            }
            Self::Log {
                n,
                rev,
                paths: p,
            } => {
                parts.push(format!("-n {n}"));
                parts.extend(rev.clone());
                paths = p;
            }
            Self::Show {
                rev,
                path,
            } => match path {
                Some(path) => parts.push(format!("{rev}:{path}")),
                None => parts.push(rev.clone()),
            },
            Self::Status | Self::Branches => {}
        }
        if !paths.is_empty() {
            parts.push("--".into());
            parts.extend(paths.iter().cloned());
        }
        parts.join(" ")
    }
}

#[derive(Clone, Copy)]
enum RevForm {
    Single,
    Range { symmetric: bool },
}

fn check_rev(subcommand: &str, rev: &str, allowed: RevForm) -> Result<String, String> {
    validate_rev_input(rev).map_err(|error| format!("git(): {error}"))?;
    let sides: Vec<&str> = if let Some((a, b)) = rev.split_once("...") {
        match allowed {
            RevForm::Range {
                symmetric: true,
            } => vec![a, b],
            _ => return Err("git(): '...' ranges are accepted only by diff".into()),
        }
    } else if let Some((a, b)) = rev.split_once("..") {
        match allowed {
            RevForm::Range {
                ..
            } => vec![a, b],
            RevForm::Single => {
                return Err(format!(
                    "git('{subcommand}'): rev must name one commit, not a range"
                ))
            }
        }
    } else {
        vec![rev]
    };
    for side in sides {
        if !side.is_empty() {
            validate_rev_input(side).map_err(|error| format!("git(): {error}"))?;
        }
        if side.contains("..") || side.contains(':') || side.contains("@{") || side.contains("^{") {
            return Err(format!(
                "git(): rev '{rev}' must name a commit; blob, index, path, reflog and peel forms are not accepted"
            ));
        }
    }
    Ok(rev.to_string())
}

/// A literal path relative to the repository root, with `/` separators; "" for the whole tree.
fn normalize_path(raw: &str) -> Result<String, String> {
    let refused =
        || "git(): Git paths must be non-empty paths relative to the repository".to_string();
    #[cfg(windows)]
    let raw = raw.replace('\\', "/");
    #[cfg(windows)]
    let raw = raw.as_str();
    if raw.is_empty() || raw.contains('\0') {
        return Err(refused());
    }
    let path = Path::new(raw);
    if path.is_absolute() {
        return Err(refused());
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::CurDir => {}
            _ => return Err(refused()),
        }
    }
    Ok(parts.join("/"))
}

/// The part of the repository inside the chat's workspace folders.
struct Scope {
    prefixes: Vec<String>,
}

impl Scope {
    fn new(workdir: &Path, folders: &[PathBuf]) -> Self {
        let mut prefixes = Vec::new();
        for folder in folders {
            let folder = dunce::canonicalize(folder).unwrap_or_else(|_| folder.clone());
            if workdir.starts_with(&folder) {
                prefixes = vec![String::new()];
                break;
            }
            if let Ok(relative) = folder.strip_prefix(workdir) {
                let relative = relative
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                if !prefixes.contains(&relative) {
                    prefixes.push(relative);
                }
            }
        }
        Self {
            prefixes,
        }
    }
    fn whole(&self) -> bool {
        self.prefixes.iter().any(String::is_empty)
    }
    fn contains(&self, path: &str) -> bool {
        self.prefixes.iter().any(|prefix| {
            prefix.is_empty()
                || path == prefix
                || path
                    .strip_prefix(prefix.as_str())
                    .is_some_and(|rest| rest.starts_with('/'))
        })
    }
    /// Paths a diff or walk must stay within: the requested ones, or the scope when partial.
    fn limits(&self, requested: &[String]) -> Vec<String> {
        if !requested.is_empty() {
            requested.to_vec()
        } else if self.whole() {
            Vec::new()
        } else {
            self.prefixes.clone()
        }
    }
}

/// `path` with its longest existing part resolved through links; None when no part of it exists.
fn resolved(path: &Path) -> Option<PathBuf> {
    let mut rest = Vec::new();
    let mut existing = path;
    loop {
        if let Ok(mut target) = dunce::canonicalize(existing) {
            target.extend(rest.into_iter().rev());
            return Some(target);
        }
        rest.push(existing.file_name()?);
        existing = existing.parent()?;
    }
}

/// What a call has let through so far: the files whose content it shows and how many it withheld.
struct Gate<'a> {
    workdir: PathBuf,
    scope: Scope,
    context: &'a GitContext<'a>,
    shown: Vec<PathBuf>,
    withheld: usize,
}

impl Gate<'_> {
    fn cancelled(&self) -> Result<(), GitError> {
        if self.context.cancel.load(Ordering::Acquire) {
            Err(GitError::Cancelled)
        } else {
            Ok(())
        }
    }
    /// Whether a script may read the worktree file at the repository path `path`: the path as
    /// written and the file it resolves to now must both pass.
    fn may_read(&self, path: &str) -> bool {
        let lexical = self.workdir.join(path);
        (self.context.may_read)(&lexical)
            && resolved(&lexical)
                .is_none_or(|target| target == lexical || (self.context.may_read)(&target))
    }
    /// Admits a file whose sides are `paths`: false when a side is outside the workspace
    /// folders, or when one may not be read or has a name that was not valid UTF-8, which
    /// counts it as withheld.
    fn admit(&mut self, paths: &[&str]) -> bool {
        if !paths.iter().all(|path| self.scope.contains(path)) {
            return false;
        }
        if !paths
            .iter()
            .all(|path| !path.contains(char::REPLACEMENT_CHARACTER) && self.may_read(path))
        {
            self.withheld += 1;
            return false;
        }
        true
    }
    fn show(&mut self, paths: &[&str]) {
        for path in paths {
            let absolute = self.workdir.join(path);
            if !self.shown.contains(&absolute) {
                self.shown.push(absolute);
            }
        }
    }
    /// Path arguments: inside the workspace folders, and readable like `open()` would read them,
    /// whether or not anything exists at that path.
    fn check_requested(&self, paths: &[String]) -> Result<(), GitError> {
        for path in paths {
            if !self.scope.contains(path) {
                return Err(GitError::Type(format!(
                    "git(): path '{path}' is outside the chat's workspace folders"
                )));
            }
            if !self.may_read(path) {
                return Err(GitError::Denied(format!(
                    "git(): reading {path} is not allowed by the privacy policy or the engine's rules"
                )));
            }
        }
        Ok(())
    }
    fn output(self, mut data: Value, summary: String, truncated: bool) -> GitOutput {
        data["root"] = json!(self.workdir.to_string_lossy().replace('\\', "/"));
        data["withheld_count"] = json!(self.withheld);
        GitOutput {
            data,
            summary,
            paths: self.shown,
            truncated,
        }
    }
}

fn shallow_note(repo: &Repository) -> &'static str {
    if repo.is_shallow() {
        " (shallow repository: older history is missing)"
    } else {
        ""
    }
}

fn resolve<'r>(repo: &'r Repository, rev: &str) -> Result<git2::Commit<'r>, GitError> {
    let rev = if rev.is_empty() { "HEAD" } else { rev };
    resolve_commit_ref(repo, rev)
        .ok_or_else(|| GitError::Failed(format!("revision not found: {rev}{}", shallow_note(repo))))
}

pub fn run(request: &GitRequest, context: &GitContext) -> Result<GitOutput, GitError> {
    let repo = Repository::discover(context.root).map_err(|_| {
        GitError::Failed(format!(
            "not a git repository: {}",
            context.root.to_string_lossy()
        ))
    })?;
    if repo.is_bare() {
        return Err(GitError::Failed(format!(
            "bare repository without a working tree: {}",
            context.root.to_string_lossy()
        )));
    }
    if has_alternates(&repo) {
        return Err(GitError::Failed(format!(
            "the repository at {} uses an alternate object store (objects/info/alternates); git() does not read such repositories",
            context.root.to_string_lossy()
        )));
    }
    let workdir = repo
        .workdir()
        .map(|workdir| dunce::canonicalize(workdir).unwrap_or_else(|_| workdir.to_path_buf()))
        .ok_or_else(|| GitError::Failed("the repository has no working tree".into()))?;
    let scope = Scope::new(&workdir, context.folders);
    if scope.prefixes.is_empty() {
        return Err(GitError::Failed(format!(
            "the repository at {} is outside the chat's workspace folders",
            workdir.to_string_lossy()
        )));
    }
    let mut gate = Gate {
        workdir,
        scope,
        context,
        shown: Vec::new(),
        withheld: 0,
    };
    match request {
        GitRequest::Status => status(&repo, gate),
        GitRequest::Diff {
            rev,
            paths,
            staged,
        } => {
            gate.check_requested(paths)?;
            diff(&repo, gate, rev.as_deref(), paths, *staged)
        }
        GitRequest::Log {
            n,
            rev,
            paths,
        } => {
            gate.check_requested(paths)?;
            log(&repo, gate, *n, rev.as_deref(), paths)
        }
        GitRequest::Show {
            rev,
            path: None,
        } => show_commit(&repo, gate, rev),
        GitRequest::Show {
            rev,
            path: Some(path),
        } => {
            gate.check_requested(std::slice::from_ref(path))?;
            show_file(&repo, &mut gate, rev, path)
        }
        GitRequest::Branches => branches(&repo, gate),
    }
}

/// Whether the object database borrows objects from another repository; a file that cannot be
/// read counts as one.
fn has_alternates(repo: &Repository) -> bool {
    match std::fs::read(repo.commondir().join("objects/info/alternates")) {
        Ok(bytes) => bytes.iter().any(|byte| !byte.is_ascii_whitespace()),
        Err(error) => error.kind() != std::io::ErrorKind::NotFound,
    }
}

fn unborn(repo: &Repository) -> bool {
    matches!(repo.head(), Err(error) if error.code() == git2::ErrorCode::UnbornBranch)
}

fn unborn_branch(repo: &Repository) -> Option<String> {
    let head = repo.find_reference("HEAD").ok()?;
    head.symbolic_target()?
        .strip_prefix("refs/heads/")
        .map(str::to_string)
}

fn status(repo: &Repository, mut gate: Gate) -> Result<GitOutput, GitError> {
    let pathspecs = gate.scope.limits(&[]);
    let snapshot = read_status_snapshot_with(
        repo,
        &StatusScan {
            exclude_submodules: true,
            pathspecs: &pathspecs,
            exact_names: true,
        },
    )
    .map_err(GitError::Failed)?;
    gate.withheld += snapshot.inexact;
    let head = head_status(repo);
    let unborn = unborn(repo);
    let mut changes = Vec::new();
    for change in snapshot.changes {
        let mut sides = vec![change.relative_path.as_str()];
        sides.extend(change.old_path.as_deref());
        if !gate.admit(&sides) {
            continue;
        }
        let mut entry = Map::new();
        entry.insert("path".into(), json!(change.relative_path));
        if let Some(old) = &change.old_path {
            entry.insert("old_path".into(), json!(old));
        }
        entry.insert("status".into(), json!(change.status));
        entry.insert("staged".into(), json!(change.staged));
        entry.insert("unstaged".into(), json!(change.unstaged));
        changes.push(Value::Object(entry));
    }
    let mut untracked = Vec::new();
    for path in snapshot.untracked {
        if gate.admit(&[path.relative_path.as_str()]) {
            untracked.push(json!(path.relative_path));
        }
    }
    let mut conflicted = Vec::new();
    for conflict in snapshot.conflicted {
        if gate.admit(&[conflict.relative_path.as_str()]) {
            conflicted.push(json!({
                "path": conflict.relative_path,
                "ours": conflict.ours,
                "theirs": conflict.theirs,
                "ancestor": conflict.ancestor,
            }));
        }
    }
    let branch = head
        .branch
        .clone()
        .or_else(|| unborn.then(|| unborn_branch(repo)).flatten());
    let place = match (&branch, head.head_detached) {
        (Some(branch), _) => branch.clone(),
        (None, true) => format!(
            "detached at {}",
            head.head_oid.as_deref().map(short).unwrap_or_default()
        ),
        (None, false) => "no branch".into(),
    };
    let upstream = match (&head.upstream, head.ahead, head.behind) {
        (Some(upstream), Some(ahead), Some(behind)) => {
            format!("…{upstream} +{ahead}/−{behind}")
        }
        (Some(upstream), _, _) => format!("…{upstream}"),
        _ => String::new(),
    };
    let mut summary = format!(
        "{place}{upstream} · {} changed, {} untracked",
        changes.len(),
        untracked.len()
    );
    if !conflicted.is_empty() {
        summary.push_str(&format!(", {} conflicted", conflicted.len()));
    }
    if gate.withheld > 0 {
        summary.push_str(&format!(", {} withheld", gate.withheld));
    }
    let data = json!({
        "branch": branch,
        "head_oid": head.head_oid,
        "head_detached": head.head_detached,
        "unborn": unborn,
        "upstream": head.upstream,
        "ahead": head.ahead,
        "behind": head.behind,
        "repo_state": repo_state_name(repo.state()),
        "changes": changes,
        "untracked": untracked,
        "conflicted": conflicted,
        "truncated": snapshot.truncated,
    });
    Ok(gate.output(data, summary, snapshot.truncated))
}

fn short(oid: &str) -> String {
    oid.chars().take(12).collect()
}

fn delta_status(status: Delta) -> Option<&'static str> {
    Some(match status {
        Delta::Added | Delta::Copied => "ADDED",
        Delta::Deleted => "DELETED",
        Delta::Modified => "MODIFIED",
        Delta::Renamed => "RENAMED",
        Delta::Typechange => "TYPECHANGE",
        Delta::Conflicted => "CONFLICTED",
        _ => return None,
    })
}

fn is_submodule(delta: &git2::DiffDelta<'_>) -> bool {
    delta.old_file().mode() == git2::FileMode::Commit
        || delta.new_file().mode() == git2::FileMode::Commit
}

fn is_link(delta: &git2::DiffDelta<'_>) -> bool {
    delta.old_file().mode() == git2::FileMode::Link
        || delta.new_file().mode() == git2::FileMode::Link
}

/// The repository paths of a delta, or None when a side has a name that cannot be checked
/// as it is: not UTF-8, or with a backslash, which the Git rail's names turn into a separator.
fn delta_paths(delta: &git2::DiffDelta<'_>) -> Option<(String, Option<String>)> {
    let exact = |file: git2::DiffFile<'_>| match file.path_bytes() {
        None => Ok(None),
        Some(bytes) => exact_path(bytes)
            .map(|path| Some(path.to_string()))
            .ok_or(()),
    };
    let (new, old) = (exact(delta.new_file()).ok()?, exact(delta.old_file()).ok()?);
    Some(match delta.status() {
        Delta::Deleted => (old.or(new).unwrap_or_default(), None),
        Delta::Renamed | Delta::Copied => {
            let path = new.clone().or(old.clone()).unwrap_or_default();
            (path.clone(), old.filter(|old| *old != path))
        }
        _ => (new.or(old).unwrap_or_default(), None),
    })
}

/// The larger side of a delta in bytes, from the object database header or the worktree file,
/// without loading content; None when a side's size cannot be read.
fn delta_size(repo: &Repository, workdir: &Path, delta: &git2::DiffDelta<'_>) -> Option<u64> {
    let odb = repo.odb().ok()?;
    let mut largest = 0;
    for file in [delta.old_file(), delta.new_file()] {
        if !file.exists() {
            continue;
        }
        let size = match odb.read_header(file.id()) {
            Ok((size, _)) if !file.id().is_zero() => size as u64,
            _ => std::fs::symlink_metadata(workdir.join(file.path()?))
                .ok()?
                .len(),
        };
        largest = largest.max(size);
    }
    Some(largest)
}

fn over_blob_budget(repo: &Repository, workdir: &Path, delta: &git2::DiffDelta<'_>) -> bool {
    delta_size(repo, workdir, delta).is_none_or(|size| size > MAX_DIFF_BLOB_BYTES)
}

/// Diff options for script diffs: literal paths, submodules and untracked files left out, and
/// files over the blob budget treated as binary before their content is loaded.
fn script_diff_options(limits: &[String]) -> git2::DiffOptions {
    let paths = limits.iter().map(PathBuf::from).collect::<Vec<_>>();
    let mut options = literal_path_diff_options(&paths);
    options
        .ignore_submodules(true)
        .include_untracked(false)
        .max_size(MAX_DIFF_BLOB_BYTES as i64);
    options
}

/// Rename detection reads added and deleted files whatever `.gitattributes` says, so it runs
/// only when there are at most 200 of them and none is over the blob budget.
fn find_renames(repo: &Repository, workdir: &Path, diff: &mut git2::Diff<'_>) {
    const RENAME_LIMIT: usize = 200;
    let mut candidates = 0;
    for delta in diff.deltas() {
        if !matches!(delta.status(), Delta::Added | Delta::Deleted) {
            continue;
        }
        candidates += 1;
        if candidates > RENAME_LIMIT || over_blob_budget(repo, workdir, &delta) {
            return;
        }
    }
    let mut find = git2::DiffFindOptions::new();
    find.renames(true)
        .rename_threshold(50)
        .rename_limit(RENAME_LIMIT);
    let _ = diff.find_similar(Some(&mut find));
}

#[derive(Default)]
struct DiffTotals {
    patch_bytes: usize,
    additions: usize,
    deletions: usize,
    truncated: bool,
}

/// One entry per changed file the scope and the privacy check admit, with its patch cut to the
/// per-file and total budgets; a file's content is loaded only while patch budget remains.
fn diff_files(
    repo: &Repository,
    gate: &mut Gate,
    diff: &git2::Diff<'_>,
    totals: &mut DiffTotals,
) -> Result<Vec<Value>, GitError> {
    let mut files = Vec::new();
    for (index, delta) in diff.deltas().enumerate() {
        gate.cancelled()?;
        let Some(status) = delta_status(delta.status()) else {
            continue;
        };
        if is_submodule(&delta) {
            continue;
        }
        let Some((path, old_path)) = delta_paths(&delta) else {
            gate.withheld += 1;
            continue;
        };
        let mut sides = vec![path.as_str()];
        sides.extend(old_path.as_deref());
        if !gate.admit(&sides) {
            continue;
        }
        if files.len() >= MAX_DIFF_FILES {
            totals.truncated = true;
            break;
        }
        let mut entry = Map::new();
        entry.insert("path".into(), json!(path));
        if let Some(old) = &old_path {
            entry.insert("old_path".into(), json!(old));
        }
        entry.insert("status".into(), json!(status));
        let link = is_link(&delta);
        entry.insert("link".into(), json!(link));
        let remaining = MAX_TOTAL_PATCH_BYTES.saturating_sub(totals.patch_bytes);
        let mut binary = delta.old_file().is_binary() || delta.new_file().is_binary();
        let mut patch_value = Value::Null;
        let mut patch_truncated = false;
        if status != "CONFLICTED" && !binary && !link {
            if remaining == 0 || over_blob_budget(repo, &gate.workdir, &delta) {
                patch_truncated = true;
                totals.truncated = true;
            } else {
                let patch = git2::Patch::from_diff(diff, index).map_err(|error| {
                    GitError::Failed(format!("cannot build the patch of {path}: {error}"))
                })?;
                match patch {
                    Some(patch) if !patch.delta().flags().is_binary() => {
                        let (_, added, deleted) = patch.line_stats().map_err(|error| {
                            GitError::Failed(format!("cannot count the lines of {path}: {error}"))
                        })?;
                        totals.additions += added;
                        totals.deletions += deleted;
                        entry.insert("additions".into(), json!(added));
                        entry.insert("deletions".into(), json!(deleted));
                        let (text, cut) = patch_text(&patch, MAX_FILE_PATCH_BYTES.min(remaining))?;
                        totals.patch_bytes += text.len();
                        if cut {
                            patch_truncated = true;
                            totals.truncated = true;
                        }
                        gate.show(&sides);
                        patch_value = json!(text);
                    }
                    _ => binary = true,
                }
            }
        }
        entry.insert("binary".into(), json!(binary));
        entry.insert("patch".into(), patch_value);
        entry.insert("patch_truncated".into(), json!(patch_truncated));
        files.push(Value::Object(entry));
    }
    Ok(files)
}

/// Appends as much of `piece` as fits in `max` bytes; false when it did not fit whole.
fn push_within(text: &mut String, piece: &str, max: usize) -> bool {
    let room = max.saturating_sub(text.len());
    if piece.len() <= room {
        text.push_str(piece);
        return true;
    }
    let mut end = room;
    while !piece.is_char_boundary(end) {
        end -= 1;
    }
    text.push_str(&piece[..end]);
    false
}

/// Hunks of one file patch as `@@` headers and `+`/`-`/` ` lines, cut at `max` bytes.
fn patch_text(patch: &git2::Patch<'_>, max: usize) -> Result<(String, bool), GitError> {
    let failed = |error: git2::Error| GitError::Failed(format!("cannot read a patch: {error}"));
    let mut text = String::new();
    for hunk_index in 0..patch.num_hunks() {
        let (hunk, lines) = patch.hunk(hunk_index).map_err(failed)?;
        if !push_within(&mut text, &String::from_utf8_lossy(hunk.header()), max) {
            return Ok((text, true));
        }
        for line_index in 0..lines {
            let line = patch.line_in_hunk(hunk_index, line_index).map_err(failed)?;
            let content = String::from_utf8_lossy(line.content());
            let piece = match line.origin() {
                origin @ ('+' | '-' | ' ') => format!("{origin}{content}"),
                _ => content.into_owned(),
            };
            if !push_within(&mut text, &piece, max) {
                return Ok((text, true));
            }
        }
    }
    Ok((text, false))
}

fn tree_of<'r>(commit: &git2::Commit<'r>) -> Result<git2::Tree<'r>, GitError> {
    commit.tree().map_err(|error| {
        GitError::Failed(format!("cannot read the tree of {}: {error}", commit.id()))
    })
}

fn first_parent_tree<'r>(commit: &git2::Commit<'r>) -> Result<Option<git2::Tree<'r>>, GitError> {
    match commit.parent(0) {
        Ok(parent) => tree_of(&parent).map(Some),
        Err(_) => Ok(None),
    }
}

fn diff_failed(error: git2::Error) -> GitError {
    GitError::Failed(format!("cannot compute the diff: {error}"))
}

fn diff(
    repo: &Repository,
    mut gate: Gate,
    rev: Option<&str>,
    paths: &[String],
    staged: bool,
) -> Result<GitOutput, GitError> {
    let mut options = script_diff_options(&gate.scope.limits(paths));
    let head_tree = || repo.head().ok().and_then(|head| head.peel_to_tree().ok());
    let (mut diff, base) = match rev {
        None if staged => (
            repo.diff_tree_to_index(head_tree().as_ref(), None, Some(&mut options))
                .map_err(diff_failed)?,
            "HEAD..index".to_string(),
        ),
        None => (
            repo.diff_index_to_workdir(None, Some(&mut options))
                .map_err(diff_failed)?,
            "index..worktree".to_string(),
        ),
        Some(rev) => {
            let range = rev
                .split_once("...")
                .map(|(a, b)| (a, b, true))
                .or_else(|| rev.split_once("..").map(|(a, b)| (a, b, false)));
            match range {
                Some((from, to, symmetric)) => {
                    let to = resolve(repo, to)?;
                    let mut from = resolve(repo, from)?;
                    if symmetric {
                        let base = repo.merge_base(from.id(), to.id()).map_err(|_| {
                            GitError::Failed(format!(
                                "no merge base between {} and {}{}",
                                short(&from.id().to_string()),
                                short(&to.id().to_string()),
                                shallow_note(repo)
                            ))
                        })?;
                        from = repo.find_commit(base).map_err(diff_failed)?;
                    }
                    let base = format!(
                        "{}..{}",
                        short(&from.id().to_string()),
                        short(&to.id().to_string())
                    );
                    let (old, new) = (tree_of(&from)?, tree_of(&to)?);
                    (
                        repo.diff_tree_to_tree(Some(&old), Some(&new), Some(&mut options))
                            .map_err(diff_failed)?,
                        base,
                    )
                }
                None => {
                    let commit = resolve(repo, rev)?;
                    let tree = tree_of(&commit)?;
                    let id = short(&commit.id().to_string());
                    if staged {
                        (
                            repo.diff_tree_to_index(Some(&tree), None, Some(&mut options))
                                .map_err(diff_failed)?,
                            format!("{id}..index"),
                        )
                    } else {
                        (
                            repo.diff_tree_to_workdir_with_index(Some(&tree), Some(&mut options))
                                .map_err(diff_failed)?,
                            format!("{id}..worktree"),
                        )
                    }
                }
            }
        }
    };
    find_renames(repo, &gate.workdir, &mut diff);
    let mut totals = DiffTotals::default();
    let files = diff_files(repo, &mut gate, &diff, &mut totals)?;
    let mut summary = format!(
        "{base}: {} files, +{} −{}",
        files.len(),
        totals.additions,
        totals.deletions
    );
    if totals.truncated {
        summary.push_str(" (truncated)");
    }
    if gate.withheld > 0 {
        summary.push_str(&format!(", {} withheld", gate.withheld));
    }
    let data = json!({"base": base, "files": files, "truncated": totals.truncated});
    Ok(gate.output(data, summary, totals.truncated))
}

fn iso_date(time: git2::Time) -> String {
    chrono::FixedOffset::east_opt(time.offset_minutes() * 60)
        .and_then(|offset| {
            chrono::DateTime::from_timestamp(time.seconds(), 0)
                .map(|utc| utc.with_timezone(&offset).to_rfc3339())
        })
        .unwrap_or_default()
}

fn commit_value(
    commit: &git2::Commit<'_>,
    labels: &std::collections::HashMap<git2::Oid, Vec<GitRefLabel>>,
) -> Map<String, Value> {
    let hash = commit.id().to_string();
    let author = commit.author();
    let message = commit.message().unwrap_or_default().to_string();
    let mut value = Map::new();
    value.insert("short".into(), json!(short(&hash)));
    value.insert("hash".into(), json!(hash));
    value.insert("author".into(), json!(author.name().unwrap_or_default()));
    value.insert("email".into(), json!(author.email().unwrap_or_default()));
    value.insert("date".into(), json!(iso_date(author.when())));
    value.insert(
        "subject".into(),
        json!(message.lines().next().unwrap_or_default()),
    );
    value.insert("message".into(), json!(message));
    value.insert(
        "parents".into(),
        json!(commit
            .parent_ids()
            .map(|oid| oid.to_string())
            .collect::<Vec<_>>()),
    );
    value.insert(
        "refs".into(),
        json!(labels
            .get(&commit.id())
            .map(|labels| labels
                .iter()
                .map(|label| label.name.clone())
                .collect::<Vec<_>>())
            .unwrap_or_default()),
    );
    value
}

fn walk_failed(error: git2::Error) -> GitError {
    GitError::Failed(format!("cannot walk the history: {error}"))
}

fn log(
    repo: &Repository,
    mut gate: Gate,
    n: usize,
    rev: Option<&str>,
    paths: &[String],
) -> Result<GitOutput, GitError> {
    if rev.is_none() && unborn(repo) {
        return Err(GitError::Failed("the repository has no commits yet".into()));
    }
    let mut walk = repo.revwalk().map_err(walk_failed)?;
    walk.set_sorting(git2::Sort::TIME | git2::Sort::TOPOLOGICAL)
        .map_err(walk_failed)?;
    let shown_rev = match rev {
        None => {
            walk.push(resolve(repo, "HEAD")?.id())
                .map_err(walk_failed)?;
            None
        }
        Some(rev) => match rev.split_once("..") {
            Some((from, to)) => {
                let (from, to) = (resolve(repo, from)?.id(), resolve(repo, to)?.id());
                walk.push(to).map_err(walk_failed)?;
                walk.hide(from).map_err(walk_failed)?;
                Some(format!("{from}..{to}"))
            }
            None => {
                let commit = resolve(repo, rev)?.id();
                walk.push(commit).map_err(walk_failed)?;
                Some(commit.to_string())
            }
        },
    };
    let limits = gate
        .scope
        .limits(paths)
        .into_iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    let mut listed = Vec::new();
    let mut scanned = 0usize;
    let mut scan_truncated = false;
    for oid in walk {
        gate.cancelled()?;
        if listed.len() >= n {
            break;
        }
        if scanned >= MAX_LOG_SCAN_COMMITS {
            scan_truncated = true;
            break;
        }
        scanned += 1;
        let commit = repo
            .find_commit(oid.map_err(walk_failed)?)
            .map_err(walk_failed)?;
        if !limits.is_empty() && !limits.iter().any(|path| commit_touches_path(&commit, path)) {
            continue;
        }
        let before = gate.withheld;
        let (files, files_truncated) = commit_files(repo, &mut gate, &commit, &limits)?;
        if !limits.is_empty() && files.is_empty() {
            gate.withheld = before;
            continue;
        }
        listed.push((commit, files, files_truncated, gate.withheld - before));
    }
    let ids = listed
        .iter()
        .map(|(commit, ..)| commit.id())
        .collect::<std::collections::HashSet<_>>();
    let labels = ref_labels_where(repo, |oid| ids.contains(&oid)).map_err(GitError::Failed)?;
    let commits = listed
        .into_iter()
        .map(|(commit, files, files_truncated, withheld)| {
            let mut value = commit_value(&commit, &labels);
            value.insert("files".into(), json!(files));
            value.insert("files_truncated".into(), json!(files_truncated));
            value.insert("withheld_count".into(), json!(withheld));
            Value::Object(value)
        })
        .collect::<Vec<_>>();
    let summary = format!(
        "{} commit{}{}",
        commits.len(),
        if commits.len() == 1 { "" } else { "s" },
        if scan_truncated {
            " (scan stopped early)"
        } else {
            ""
        }
    );
    let data = json!({
        "rev": shown_rev,
        "commits": commits,
        "scan_truncated": scan_truncated,
        "shallow": repo.is_shallow(),
    });
    Ok(gate.output(data, summary, scan_truncated))
}

/// Files a commit changed against its first parent, names only, within `limits`.
fn commit_files(
    repo: &Repository,
    gate: &mut Gate,
    commit: &git2::Commit<'_>,
    limits: &[PathBuf],
) -> Result<(Vec<Value>, bool), GitError> {
    let new = tree_of(commit)?;
    let old = first_parent_tree(commit)?;
    let mut options = literal_path_diff_options(limits);
    options.ignore_submodules(true);
    let diff = repo
        .diff_tree_to_tree(old.as_ref(), Some(&new), Some(&mut options))
        .map_err(diff_failed)?;
    let mut files = Vec::new();
    let mut truncated = false;
    for delta in diff.deltas() {
        let Some(status) = delta_status(delta.status()) else {
            continue;
        };
        if is_submodule(&delta) {
            continue;
        }
        let Some((path, old_path)) = delta_paths(&delta) else {
            gate.withheld += 1;
            continue;
        };
        let mut sides = vec![path.as_str()];
        sides.extend(old_path.as_deref());
        if !gate.admit(&sides) {
            continue;
        }
        if files.len() >= MAX_COMMIT_FILES {
            truncated = true;
            break;
        }
        files.push(json!({"path": path, "status": status}));
    }
    Ok((files, truncated))
}

fn show_commit(repo: &Repository, mut gate: Gate, rev: &str) -> Result<GitOutput, GitError> {
    let commit = resolve(repo, rev)?;
    let id = commit.id().to_string();
    if !gate.scope.whole()
        && !gate
            .scope
            .prefixes
            .iter()
            .any(|prefix| commit_touches_path(&commit, Path::new(prefix)))
    {
        return Err(GitError::Failed(format!(
            "commit {} changes no path inside the chat's workspace folders",
            short(&id)
        )));
    }
    let labels = ref_labels_by_oid(repo).map_err(GitError::Failed)?;
    let value = commit_value(&commit, &labels);
    let new = tree_of(&commit)?;
    let old = first_parent_tree(&commit)?;
    let mut options = script_diff_options(&gate.scope.limits(&[]));
    let mut diff = repo
        .diff_tree_to_tree(old.as_ref(), Some(&new), Some(&mut options))
        .map_err(diff_failed)?;
    find_renames(repo, &gate.workdir, &mut diff);
    let mut totals = DiffTotals::default();
    let files = diff_files(repo, &mut gate, &diff, &mut totals)?;
    let mut summary = format!(
        "{} {} · {} files, +{} −{}",
        short(&id),
        value["subject"].as_str().unwrap_or_default(),
        files.len(),
        totals.additions,
        totals.deletions
    );
    if totals.truncated {
        summary.push_str(" (truncated)");
    }
    if gate.withheld > 0 {
        summary.push_str(&format!(", {} withheld", gate.withheld));
    }
    let data = json!({
        "rev": id,
        "commit": value,
        "files": files,
        "truncated": totals.truncated,
    });
    Ok(gate.output(data, summary, totals.truncated))
}

fn show_file(
    repo: &Repository,
    gate: &mut Gate,
    rev: &str,
    path: &str,
) -> Result<GitOutput, GitError> {
    let absolute = gate.workdir.join(path);
    let failed = |error: git2::Error| GitError::Failed(format!("cannot read {path}: {error}"));
    let commit = resolve(repo, rev)?;
    let id = commit.id().to_string();
    let entry = match tree_of(&commit)?.get_path(Path::new(path)) {
        Ok(entry) => Some((
            entry.id(),
            entry.kind(),
            entry.filemode() == i32::from(git2::FileMode::Link),
        )),
        Err(error) if error.code() == git2::ErrorCode::NotFound => None,
        Err(error) => return Err(failed(error)),
    };
    let mut data = json!({
        "rev": id,
        "path": path,
        "exists": entry.is_some(),
        "link": false,
        "binary": false,
        "size": 0,
        "text": "",
        "truncated": false,
    });
    let summary = match entry {
        None => format!("{path} @ {}: not found", short(&id)),
        Some((_, Some(git2::ObjectType::Tree), _)) => {
            return Err(GitError::Failed(format!(
                "{path} is a directory at {}; give a file path",
                short(&id)
            )))
        }
        Some((_, Some(git2::ObjectType::Commit), _)) => {
            return Err(GitError::Failed(format!(
                "{path} is a submodule; git() does not read submodules"
            )))
        }
        Some((_, _, true)) => {
            data["link"] = json!(true);
            format!(
                "{path} @ {}: a symbolic link; git() does not show its target",
                short(&id)
            )
        }
        Some((blob_id, _, false)) => {
            let (size, _) = repo
                .odb()
                .and_then(|odb| odb.read_header(blob_id))
                .map_err(failed)?;
            data["size"] = json!(size);
            if size > MAX_FILE_BYTES {
                data["truncated"] = json!(true);
                format!(
                    "{path} @ {}: {size} bytes, over the {MAX_FILE_BYTES}-byte limit; not loaded",
                    short(&id)
                )
            } else {
                let blob = repo.find_blob(blob_id).map_err(failed)?;
                let (binary, truncated, text) =
                    crate::read::file_payload(blob.content().to_vec(), size as u64);
                let lines = text.lines().count();
                data["binary"] = json!(binary);
                data["truncated"] = json!(truncated);
                data["text"] = json!(text);
                if binary {
                    format!("{path} @ {}: binary, {size} bytes", short(&id))
                } else {
                    gate.shown.push(absolute);
                    format!("{path} @ {}: {lines} lines", short(&id))
                }
            }
        }
    };
    let truncated = data["truncated"] == true;
    data["root"] = json!(gate.workdir.to_string_lossy().replace('\\', "/"));
    data["withheld_count"] = json!(0);
    Ok(GitOutput {
        data,
        summary,
        paths: std::mem::take(&mut gate.shown),
        truncated,
    })
}

fn branches(repo: &Repository, gate: Gate) -> Result<GitOutput, GitError> {
    let (all, truncated) = branches_of(repo, &gate.workdir, MAX_REFS).map_err(GitError::Failed)?;
    let summary = format!(
        "{} branches, {} remote, {} tags{}{}",
        all.branches.len(),
        all.remote_branches.len(),
        all.tags.len(),
        if truncated { " (truncated)" } else { "" },
        all.current
            .as_deref()
            .map(|current| format!("; current {current}"))
            .unwrap_or_default()
    );
    let data = json!({
        "current": all.current,
        "head_detached": all.head_detached,
        "head_oid": all.head_oid,
        "branches": all.branches,
        "remote_branches": all.remote_branches,
        "tags": all.tags,
        "remotes": all.remotes,
        "truncated": truncated,
    });
    Ok(gate.output(data, summary, truncated))
}

#[cfg(test)]
#[path = "script_tests.rs"]
mod tests;
