//! Local ownership ledger, never an execution queue. No executable authority is stored.
//! Retention has no count ceiling: every identity is kept for dedup, and neither active
//! nor unacknowledged records are evicted. Exact native payloads remain available locally.
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use super::runtime::{JobSnapshot, JobStatus};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Entry {
    pub snapshot: JobSnapshot,
    pub consumed: bool,
    pub inline: bool,
    #[serde(default)]
    pub internal_readiness: bool,
    /// Inert original-domain provenance; never execution authority.
    #[serde(default)]
    pub scope_keys: Vec<super::ScopeKey>,
}

pub(super) struct Journal {
    connection: Mutex<Connection>,
    // The holder thread owns fd-lock's borrowed guard until this sender is dropped.
    _owner: JournalOwner,
    #[cfg(test)]
    pub admission_gate: Mutex<
        Option<(
            tokio::sync::oneshot::Sender<()>,
            tokio::sync::oneshot::Receiver<()>,
        )>,
    >,
    #[cfg(test)]
    pub inline_write_gate: Mutex<
        Option<(
            tokio::sync::oneshot::Sender<()>,
            std::sync::mpsc::Receiver<()>,
        )>,
    >,
}

struct JournalOwner {
    release: Option<std::sync::mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Drop for JournalOwner {
    fn drop(&mut self) {
        self.release.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn private_file(path: &std::path::Path) -> Result<std::fs::File, String> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("tool job journal storage is not a regular file".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if file
            .metadata()
            .map_err(|e| e.to_string())?
            .permissions()
            .mode()
            & 0o077
            != 0
        {
            return Err("tool job journal permissions are not private".into());
        }
    }
    Ok(file)
}

fn own(file: std::fs::File) -> Result<JournalOwner, String> {
    let (ready_tx, ready) = std::sync::mpsc::sync_channel(1);
    let (release, wait) = std::sync::mpsc::channel();
    let thread = std::thread::spawn(move || {
        let mut lock = fd_lock::RwLock::new(file);
        match lock.try_write() {
            Ok(_guard) => {
                let _ = ready_tx.send(Ok(()));
                let _ = wait.recv();
            }
            Err(e) => {
                let _ = ready_tx.send(Err(format!(
                    "tool job journal already owned or lock unavailable: {e}"
                )));
            }
        };
    });
    let owner = JournalOwner {
        release: Some(release),
        thread: Some(thread),
    };
    ready.recv().map_err(|e| e.to_string())??;
    Ok(owner)
}

impl Journal {
    pub async fn open(path: PathBuf) -> Result<(Arc<Self>, Vec<Entry>), String> {
        tokio::task::spawn_blocking(move || {
            let parent = path.parent().ok_or("journal parent unavailable")?;
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            // Like the operator store, reject symlink storage and use private local files.
            for target in [parent, path.as_path()] {
                match std::fs::symlink_metadata(target) {
                    Ok(meta) if meta.file_type().is_symlink() => return Err("tool job journal symlink refused".into()),
                    Ok(_) => {},
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {},
                    Err(e) => return Err(e.to_string()),
                }
            }
            let owner = own(private_file(&path.with_extension("lock"))?)?;
            let _file = private_file(&path)?;
            let mut connection = Connection::open(path).map_err(|e| e.to_string())?;
            connection.busy_timeout(std::time::Duration::from_secs(5)).map_err(|e| e.to_string())?;
            connection.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;").map_err(|e| e.to_string())?;
            let version: i64 = connection.query_row("PRAGMA user_version", [], |r| r.get(0)).map_err(|e| e.to_string())?;
            if version != 0 && version != 1 { return Err("unsupported tool job journal schema".into()); }
            connection.execute_batch("CREATE TABLE IF NOT EXISTS tool_jobs (job_id TEXT PRIMARY KEY, invocation TEXT NOT NULL UNIQUE, payload TEXT NOT NULL); PRAGMA user_version=1;").map_err(|e| e.to_string())?;
            let tx = connection.transaction().map_err(|e| e.to_string())?;
            let mut entries = Vec::new();
            {
                let mut query = tx.prepare("SELECT job_id, invocation, payload FROM tool_jobs").map_err(|e| e.to_string())?;
                let rows = query.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))).map_err(|e| e.to_string())?;
                for row in rows {
                    let (id, invocation, payload) = row.map_err(|e| e.to_string())?;
                    let entry: Entry = serde_json::from_str(&payload).map_err(|e| format!("malformed tool job journal: {e}"))?;
                    if entry.snapshot.receipt.job_id != id || identity(&entry.snapshot)? != invocation || entry.snapshot.revision == 0 {
                        return Err("inconsistent tool job journal identity".into());
                    }
                    entries.push(entry);
                }
            }
            // Recovery changes facts only: no leases, futures or scope authority restored.
            for entry in &mut entries {
                let pending_inline = entry.inline && !entry.consumed;
                let interrupted = matches!(entry.snapshot.status, JobStatus::Accepted | JobStatus::Running | JobStatus::CancelRequested);
                if interrupted {
                    entry.snapshot.status = JobStatus::Unknown;
                    entry.snapshot.revision += 1;
                    entry.snapshot.result_unavailable_reason = Some("process restarted before terminal result was durably recorded; effects may have happened; never replay".into());
                    entry.inline = false;
                    entry.consumed = false;
                }
                if pending_inline {
                    entry.inline = false;
                }
                if entry.internal_readiness { entry.consumed = true; }
                if pending_inline || interrupted || entry.internal_readiness {
                    tx.execute("UPDATE tool_jobs SET payload=?1 WHERE job_id=?2", params![serde_json::to_string(entry).map_err(|e| e.to_string())?, entry.snapshot.receipt.job_id]).map_err(|e| e.to_string())?;
                }
            }
            tx.commit().map_err(|e| e.to_string())?;
            Ok((Arc::new(Self { connection: Mutex::new(connection), _owner: owner,
                #[cfg(test)] admission_gate: Mutex::new(None),
                #[cfg(test)] inline_write_gate: Mutex::new(None),
            }), entries))
        }).await.map_err(|e| e.to_string())?
    }

    pub async fn save(self: &Arc<Self>, entry: Entry) -> Result<(), String> {
        #[cfg(test)]
        {
            let gate = self.admission_gate.lock().unwrap().take();
            if let Some((entered, release)) = gate {
                let _ = entered.send(());
                let _ = release.await;
            }
        }
        let journal = self.clone();
        tokio::task::spawn_blocking(move || {
            let invocation = identity(&entry.snapshot)?;
            let payload = serde_json::to_string(&entry).map_err(|e| e.to_string())?;
            let connection = journal.connection.lock().map_err(|_| "journal mutex poisoned")?;
            #[cfg(test)]
            if entry.inline {
                if let Some((entered, release)) = journal.inline_write_gate.lock().unwrap().take() {
                    let _ = entered.send(());
                    let _ = release.recv();
                }
            }
            connection.execute(
                "INSERT INTO tool_jobs(job_id, invocation, payload) VALUES(?1, ?2, ?3) ON CONFLICT(job_id) DO UPDATE SET payload=excluded.payload",
                params![entry.snapshot.receipt.job_id, invocation, payload],
            ).map_err(|e| e.to_string())?;
            Ok(())
        }).await.map_err(|e| e.to_string())?
    }

    pub async fn load(self: &Arc<Self>, job_id: String) -> Result<Entry, String> {
        let journal = self.clone();
        tokio::task::spawn_blocking(move || {
            let payload: String = journal
                .connection
                .lock()
                .map_err(|_| "journal mutex poisoned")?
                .query_row(
                    "SELECT payload FROM tool_jobs WHERE job_id=?1",
                    [job_id],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            serde_json::from_str(&payload).map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())?
    }
}

pub(super) fn identity(snapshot: &JobSnapshot) -> Result<String, String> {
    // scope_keys are deliberately serde-skipped: restored identities convey no authority.
    serde_json::to_string(&snapshot.receipt.binding).map_err(|e| e.to_string())
}
