use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use regex::Regex;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

pub const DEFAULT_GIT_CLI_TIMEOUT: Duration = Duration::from_secs(60);
pub const NETWORK_GIT_CLI_TIMEOUT: Duration = Duration::from_secs(120);
const DEFAULT_MAX_OUTPUT_BYTES: usize = 1024 * 1024;
const REPOSITORY_SELECTING_ENV: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
    "GIT_PREFIX",
    "GIT_QUARANTINE_PATH",
];

#[derive(Debug, Clone)]
pub struct GitCliOptions {
    pub timeout: Duration,
    pub stdin: Option<Vec<u8>>,
    pub literal_pathspecs: bool,
    pub max_output_bytes: usize,
}

impl Default for GitCliOptions {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_GIT_CLI_TIMEOUT,
            stdin: None,
            literal_pathspecs: false,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
        }
    }
}

impl GitCliOptions {
    pub fn with_paths(paths: &[String]) -> Self {
        Self {
            stdin: Some(pathspec_nul_input(paths)),
            literal_pathspecs: true,
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone)]
pub struct GitCliOutput {
    pub exit_code: Option<i32>,
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    pub truncated: bool,
}

impl GitCliOutput {
    pub fn combined_tail(&self, max_bytes: usize) -> String {
        let combined = format!("{}{}", self.stdout, self.stderr);
        let combined = combined.trim();
        if combined.len() <= max_bytes {
            return combined.to_string();
        }
        let mut start = combined.len() - max_bytes;
        while !combined.is_char_boundary(start) {
            start += 1;
        }
        combined[start..].to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitCliError {
    Unavailable(String),
    Timeout(Duration),
    Io(String),
}

impl std::fmt::Display for GitCliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GitCliError::Unavailable(message) => write!(f, "git is unavailable: {}", message),
            GitCliError::Timeout(timeout) => {
                write!(f, "git timed out after {}s", timeout.as_secs())
            }
            GitCliError::Io(message) => write!(f, "git failed: {}", message),
        }
    }
}

pub fn pathspec_nul_input(paths: &[String]) -> Vec<u8> {
    let mut input = Vec::new();
    for path in paths {
        input.extend_from_slice(path.as_bytes());
        input.push(0);
    }
    input
}

pub fn redact_git_output(text: &str) -> String {
    static USERINFO: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)\b([a-z][a-z0-9+.\-]*://)[^/\s@]+@").unwrap());
    let text = USERINFO.replace_all(text, "${1}[REDACTED]@");
    refact_core::string_utils::redact_sensitive(&text)
}

pub fn resolve_git_executable() -> Result<PathBuf, GitCliError> {
    which::which("git").map_err(|error| GitCliError::Unavailable(error.to_string()))
}

fn ssh_command_configured(root: &Path) -> bool {
    if std::env::var_os("GIT_SSH_COMMAND").is_some() || std::env::var_os("GIT_SSH").is_some() {
        return true;
    }
    git2::Repository::open(root)
        .ok()
        .and_then(|repository| repository.config().ok())
        .and_then(|config| config.get_string("core.sshCommand").ok())
        .is_some_and(|value| !value.trim().is_empty())
}

async fn read_capped<R: AsyncRead + Unpin>(mut reader: R, cap: usize) -> (Vec<u8>, bool) {
    let mut kept = Vec::new();
    let mut truncated = false;
    let mut buffer = [0u8; 8192];
    loop {
        match reader.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                let room = cap.saturating_sub(kept.len());
                if read > room {
                    truncated = true;
                }
                kept.extend_from_slice(&buffer[..read.min(room)]);
            }
        }
    }
    (kept, truncated)
}

pub async fn run_git_cli(
    root: &Path,
    args: &[OsString],
    options: GitCliOptions,
) -> Result<GitCliOutput, GitCliError> {
    let git = resolve_git_executable()?;
    let mut command = tokio::process::Command::new(git);
    command
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .env("LANGUAGE", "C")
        .env("LC_MESSAGES", "C")
        .stdin(if options.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // `-C root` must select the repository: inherited repository-selecting variables (e.g. when
    // the engine itself runs inside a git hook) would otherwise redirect commands elsewhere.
    for name in REPOSITORY_SELECTING_ENV {
        command.env_remove(name);
    }
    // Callers map git's English diagnostics to error codes; keep messages untranslated while
    // preserving the caller's character-set locale (an LC_ALL override would win otherwise).
    if let Some(all) = std::env::var_os("LC_ALL").filter(|value| !value.is_empty()) {
        command.env_remove("LC_ALL").env("LANG", all);
    }
    if options.literal_pathspecs {
        command.env("GIT_LITERAL_PATHSPECS", "1");
    }
    if !ssh_command_configured(root) {
        command.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
    }
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            GitCliError::Unavailable(error.to_string())
        } else {
            GitCliError::Io(error.to_string())
        }
    })?;
    let stdin_task = match (child.stdin.take(), options.stdin) {
        (Some(mut stdin), Some(input)) => Some(tokio::spawn(async move {
            let _ = stdin.write_all(&input).await;
            let _ = stdin.shutdown().await;
        })),
        _ => None,
    };
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let cap = options.max_output_bytes;
    let run = async move {
        let stdout_task = async move {
            match stdout {
                Some(stdout) => read_capped(stdout, cap).await,
                None => (Vec::new(), false),
            }
        };
        let stderr_task = async move {
            match stderr {
                Some(stderr) => read_capped(stderr, cap).await,
                None => (Vec::new(), false),
            }
        };
        let ((stdout, stdout_truncated), (stderr, stderr_truncated), status) =
            tokio::join!(stdout_task, stderr_task, child.wait());
        (stdout, stderr, stdout_truncated || stderr_truncated, status)
    };
    let (stdout, stderr, truncated, status) = tokio::time::timeout(options.timeout, run)
        .await
        .map_err(|_| GitCliError::Timeout(options.timeout))?;
    if let Some(task) = stdin_task {
        let _ = task.await;
    }
    let status = status.map_err(|error| GitCliError::Io(error.to_string()))?;
    Ok(GitCliOutput {
        exit_code: status.code(),
        success: status.success(),
        stdout: redact_git_output(&String::from_utf8_lossy(&stdout)),
        stderr: redact_git_output(&String::from_utf8_lossy(&stderr)),
        truncated,
    })
}

pub fn git_args<I, S>(args: I) -> Vec<OsString>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    args.into_iter().map(Into::into).collect()
}

pub fn repo_mutation_lock(root: &Path) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: LazyLock<Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));
    let key = dunce::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let mut locks = LOCKS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    locks.retain(|path, lock| path == &key || Arc::strong_count(lock) > 1);
    locks
        .entry(key)
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
        .clone()
}
