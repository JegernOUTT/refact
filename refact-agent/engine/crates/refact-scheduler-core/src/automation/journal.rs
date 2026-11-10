use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::collections::HashMap;
use std::path::PathBuf;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde_json::Value;
use super::catalog::{child, err, regular, root_dir};
use super::types::*;
use cap_fs_ext::DirExt;

/// Eight failed delivery attempts; delivery retries never re-execute actions.
const MAX_DELIVERY_ATTEMPTS: u32 = 8;
/// Preserve dead-letter control metadata; fail admission rather than lose work.
const MAX_DELIVERY_RECORDS: i64 = 16384;

#[derive(Clone)]
pub struct AutomationJournal {
    observer: bool,
    root: PathBuf,
    owner: Arc<Mutex<Option<Arc<super::ProjectAutomationLease>>>>,
    catalog_gate: Arc<Mutex<()>>,
    catalog: Arc<super::AutomationCatalog>,
    connection: Arc<Mutex<Connection>>,
    _directory: Arc<cap_std::fs::Dir>,
}
pub fn legacy_job_digest(job: &crate::types::Job) -> Result<String, String> {
    digest_json(
        &serde_json::json!({"id":job.id,"enabled":job.enabled,"paused_at_ms":job.paused_at_ms,"trigger":job.trigger,"action":job.action,"delivery":job.delivery,"push":job.push,"recurring":job.recurring}),
    )
}
fn copy_legacy_runtime(target: &mut crate::types::Job, source: &crate::types::Job) {
    target.fire_count = source.fire_count;
    target.last_fired_at_ms = source.last_fired_at_ms;
    target.last_status = source.last_status.clone();
    target.last_error = source.last_error.clone();
    target.last_delivery_error = source.last_delivery_error.clone();
    target.recent_runs = source.recent_runs.clone();
    target.retry_attempts = source.retry_attempts;
    target.trigger_at_ms = source.trigger_at_ms;
}
fn json<T: serde::Serialize>(v: &T) -> Result<String, String> {
    serde_json::to_string(v).map_err(err)
}
/// Local runtime state is intentionally outside the project-controlled tree.
/// This boundary excludes an attacker with arbitrary write access to the user's private cache.
pub fn journal_path(root: impl AsRef<Path>) -> Result<PathBuf, String> {
    let _ = root_dir(root.as_ref())?;
    let root = std::fs::canonicalize(root.as_ref()).map_err(err)?;
    let scope = digest_json(&serde_json::json!({"canonical_root":root.to_string_lossy()}))?;
    let base = if let Some(cache) = std::env::var_os("XDG_CACHE_HOME") {
        PathBuf::from(cache)
    } else {
        PathBuf::from(
            std::env::var_os("HOME")
                .ok_or("HOME or XDG_CACHE_HOME required for private automation runtime")?,
        )
        .join(".cache")
    };
    if !base.is_absolute() {
        return Err("private runtime cache must be absolute".into());
    }
    Ok(base
        .join("refact-automation-runtime")
        .join("v1")
        .join(scope)
        .join("runtime.sqlite"))
}
pub(crate) fn private_runtime_directory(
    path: &Path,
    create: bool,
) -> Result<cap_std::fs::Dir, String> {
    let base = path
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .ok_or("runtime base")?;
    if create {
        std::fs::create_dir_all(base).map_err(err)?;
    }
    let mut directory = root_dir(base)?;
    for name in path.strip_prefix(base).map_err(err)?.components() {
        let std::path::Component::Normal(name) = name else {
            return Err("unsafe runtime path".into());
        };
        if create {
            #[cfg(unix)]
            {
                use std::os::fd::AsRawFd;
                use std::os::unix::ffi::OsStrExt;
                let name = std::ffi::CString::new(name.as_bytes()).map_err(err)?;
                let rc = unsafe { libc::mkdirat(directory.as_raw_fd(), name.as_ptr(), 0o700) };
                if rc != 0 {
                    let error = std::io::Error::last_os_error();
                    if error.kind() != std::io::ErrorKind::AlreadyExists {
                        return Err(err(error));
                    }
                }
            }
            #[cfg(not(unix))]
            return Err("private automation runtime permissions unsupported".into());
        }
        directory = directory.open_dir_nofollow(name).map_err(err)?;
        #[cfg(unix)]
        {
            use cap_std::fs::MetadataExt;
            let meta = directory.dir_metadata().map_err(err)?;
            if meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0 {
                return Err("automation runtime directory must be owner-only 0700".into());
            }
        }
    }
    Ok(directory)
}
impl AutomationJournal {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, String> {
        Self::open_mode(root.as_ref(), false)
    }
    pub fn open_observer(root: impl AsRef<Path>) -> Result<Self, String> {
        Self::open_mode(root.as_ref(), true)
    }
    fn open_mode(root: &Path, observer: bool) -> Result<Self, String> {
        let root_handle = root_dir(root)?;
        let canonical = std::fs::canonicalize(root).map_err(err)?;
        static GATES: OnceLock<Mutex<HashMap<PathBuf, Weak<Mutex<()>>>>> = OnceLock::new();
        let catalog_gate = {
            let mut gates = GATES.get_or_init(Default::default).lock().map_err(err)?;
            if let Some(gate) = gates.get(&canonical).and_then(Weak::upgrade) {
                gate
            } else {
                let gate = Arc::new(Mutex::new(()));
                gates.insert(canonical.clone(), Arc::downgrade(&gate));
                gate
            }
        };
        let refact = child(&root_handle, ".refact", !observer)?;
        let path = journal_path(root)?;
        let parent = path.parent().ok_or("invalid runtime storage path")?;
        let directory = private_runtime_directory(parent, !observer)?;
        for name in [
            "runtime.sqlite",
            "runtime.sqlite-journal",
            "runtime.sqlite-wal",
            "runtime.sqlite-shm",
        ] {
            match directory.symlink_metadata(name) {
                Ok(_) => {
                    regular(&directory, name, false, !observer)?;
                }
                Err(e)
                    if e.kind() == std::io::ErrorKind::NotFound
                        && (name != "runtime.sqlite" || !observer) => {}
                Err(e) => return Err(err(e)),
            }
        }
        if !observer {
            regular(&directory, "runtime.sqlite", true, true)?;
        }
        let flags = if observer {
            OpenFlags::SQLITE_OPEN_READ_ONLY
        } else {
            OpenFlags::SQLITE_OPEN_READ_WRITE
        } | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW;
        let connection = Connection::open_with_flags(path, flags).map_err(err)?;
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(err)?;
        if !observer {
            connection.execute_batch(SCHEMA).map_err(err)?;
            let outbox_columns: Vec<String> = {
                let mut stmt = connection
                    .prepare("PRAGMA table_info(outbox)")
                    .map_err(err)?;
                let rows = stmt.query_map([], |r| r.get(1)).map_err(err)?;
                rows.collect::<Result<_, _>>().map_err(err)?
            };
            if !outbox_columns.iter().any(|name| name == "failed") {
                connection
                    .execute(
                        "ALTER TABLE outbox ADD COLUMN failed INTEGER NOT NULL DEFAULT 0",
                        [],
                    )
                    .map_err(err)?;
            }
            if !outbox_columns
                .iter()
                .any(|name| name == "failed_definition")
            {
                connection
                    .execute("ALTER TABLE outbox ADD COLUMN failed_definition TEXT", [])
                    .map_err(err)?;
            }
            let has_approved: bool = {
                let mut stmt = connection
                    .prepare("PRAGMA table_info(definitions)")
                    .map_err(err)?;
                let rows = stmt.query_map([], |r| r.get::<_, String>(1)).map_err(err)?;
                rows.collect::<Result<Vec<_>, _>>()
                    .map_err(err)?
                    .iter()
                    .any(|name| name == "approved")
            };
            let columns: Vec<String> = {
                let mut stmt = connection
                    .prepare("PRAGMA table_info(definitions)")
                    .map_err(err)?;
                let rows = stmt.query_map([], |r| r.get(1)).map_err(err)?;
                rows.collect::<Result<_, _>>().map_err(err)?
            };
            if !columns.iter().any(|name| name == "created_at_ms") {
                connection.execute("ALTER TABLE definitions ADD COLUMN created_at_ms INTEGER NOT NULL DEFAULT 0",[]).map_err(err)?;
            }
            if !columns.iter().any(|name| name == "last_fired_at_ms") {
                connection
                    .execute(
                        "ALTER TABLE definitions ADD COLUMN last_fired_at_ms INTEGER",
                        [],
                    )
                    .map_err(err)?;
            }
            if !has_approved {
                connection
                    .execute(
                        "ALTER TABLE definitions ADD COLUMN approved INTEGER NOT NULL DEFAULT 0",
                        [],
                    )
                    .map_err(err)?;
            }
            connection
                .execute(
                    "INSERT OR IGNORE INTO metadata(key,value) VALUES('project_root',?1)",
                    [canonical.to_string_lossy().as_ref()],
                )
                .map_err(err)?;
        }
        let bound: String = connection
            .query_row(
                "SELECT value FROM metadata WHERE key='project_root'",
                [],
                |r| r.get(0),
            )
            .map_err(err)?;
        if bound != canonical.to_string_lossy() {
            return Err("runtime journal project binding mismatch".into());
        }
        Ok(Self {
            observer,
            root: canonical,
            owner: Arc::new(Mutex::new(None)),
            catalog_gate,
            catalog: Arc::new(super::AutomationCatalog::from_directory(refact)),
            connection: Arc::new(Mutex::new(connection)),
            _directory: Arc::new(directory),
        })
    }
    fn db(&self) -> Result<MutexGuard<'_, Connection>, String> {
        self.connection.lock().map_err(err)
    }
    fn writable(&self) -> Result<(), String> {
        if self.observer {
            Err("read-only automation observer".into())
        } else {
            Ok(())
        }
    }
    pub fn activate_owner(&self, lease: &Arc<super::ProjectAutomationLease>) -> Result<(), String> {
        self.writable()?;
        if lease.root != self.root {
            return Err("lease project mismatch".into());
        }
        self.db()?.execute("INSERT INTO metadata(key,value) VALUES('owner_epoch',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[lease.epoch()]).map_err(err)?;
        *self.owner.lock().map_err(err)? = Some(lease.clone());
        // Migrate old infinite-retry records only under execution authority.
        let mut db = self.db()?;
        let tx = self.transaction(&mut db)?;
        tx.execute("UPDATE outbox SET failed=1,error='delivery failed (details redacted)',failed_definition=(SELECT source FROM events WHERE seq=outbox.event_seq AND app='cron') WHERE acked=0 AND failed=0 AND attempts>=?1", [MAX_DELIVERY_ATTEMPTS]).map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(())
    }
    fn check_owner(&self, expected: Option<&str>) -> Result<String, String> {
        self.writable()?;
        let lease = self
            .owner
            .lock()
            .map_err(err)?
            .as_ref()
            .cloned()
            .ok_or("journal execution owner not activated")?;
        if expected.is_some_and(|e| e != lease.epoch()) {
            return Err("stale execution owner epoch".into());
        }
        let current: Option<String> = self
            .db()?
            .query_row(
                "SELECT value FROM metadata WHERE key='owner_epoch'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        if current.as_deref() != Some(lease.epoch()) {
            return Err("stale execution owner".into());
        }
        Ok(lease.epoch().into())
    }
    fn owner_epoch_for_write(&self) -> Result<String, String> {
        self.writable()?;
        let existing = self.owner.lock().map_err(err)?.as_ref().cloned();
        if existing.is_some() {
            return self.check_owner(None);
        }
        let lease = super::ProjectAutomationLease::acquire(&self.root)?;
        self.activate_owner(&lease)?;
        Ok(lease.epoch().into())
    }
    fn transaction<'a>(&self, db: &'a mut Connection) -> Result<rusqlite::Transaction<'a>, String> {
        // Get/retain authority before locking the database; callers initialize outside db().
        let epoch = self
            .owner
            .lock()
            .map_err(err)?
            .as_ref()
            .ok_or("owner not bound before transaction")?
            .epoch()
            .to_string();
        let tx = db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(err)?;
        verify_transaction_owner(&tx, &epoch)?;
        Ok(tx)
    }
    fn write_execute<P: rusqlite::Params>(&self, sql: &str, params: P) -> Result<usize, String> {
        self.owner_epoch_for_write()?;
        let mut db = self.db()?;
        let tx = self.transaction(&mut db)?;
        let count = tx.execute(sql, params).map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(count)
    }
    pub fn load_legacy_jobs(&self) -> Result<Vec<crate::types::Job>, String> {
        if !self.observer {
            self.reconcile_legacy_catalog()?;
        }
        let db = self.db()?;
        let mut stmt = db
            .prepare("SELECT body FROM cron_jobs WHERE session_epoch IS NULL ORDER BY id")
            .map_err(err)?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(err)?;
        rows.map(|r| serde_json::from_str(&r.map_err(err)?).map_err(err))
            .collect()
    }
    pub fn put_legacy_job(&self, job: &crate::types::Job) -> Result<(), String> {
        self.owner_epoch_for_write()?;
        if self.observer {
            return Err("read-only automation observer".into());
        }
        let _gate = self.catalog_gate.lock().map_err(err)?;
        let mut doc = self.catalog.read()?;
        if let Some(existing) = doc.legacy_jobs.iter_mut().find(|j| j.id == job.id) {
            *existing = job.clone();
        } else {
            doc.legacy_jobs.push(job.clone());
        }
        self.catalog
            .compare_and_swap_legacy_jobs(doc.revision, doc.legacy_jobs)?;
        self.put_job(job, None)
    }
    fn reconcile_legacy_catalog(&self) -> Result<(), String> {
        self.writable()?;
        let _gate = self.catalog_gate.lock().map_err(err)?;
        let doc = self.catalog.read()?;
        self.reconcile_legacy_document(&doc)
    }
    pub fn mutate_legacy_job(
        &self,
        id: &str,
        mutation: impl FnOnce(&mut crate::types::Job) -> Result<bool, String>,
    ) -> Result<Option<crate::types::Job>, String> {
        self.owner_epoch_for_write()?;
        let _gate = self.catalog_gate.lock().map_err(err)?;
        let mut doc = self.catalog.read()?;
        self.reconcile_legacy_document(&doc)?;
        let Some(index) = doc.legacy_jobs.iter().position(|job| job.id == id) else {
            return Ok(None);
        };
        let body: String = self
            .db()?
            .query_row(
                "SELECT body FROM cron_jobs WHERE id=?1 AND session_epoch IS NULL",
                [id],
                |row| row.get(0),
            )
            .map_err(err)?;
        let mut job: crate::types::Job = serde_json::from_str(&body).map_err(err)?;
        let before = job.clone();
        let keep = mutation(&mut job)?;
        if job.id != id {
            return Err("scheduled task mutation cannot change identity".into());
        }
        if keep && job == before {
            return Ok(Some(job));
        }
        if json(&job)?.len() > 1048576 {
            return Err("job size limit".into());
        }
        let mut desired = job.clone();
        copy_legacy_runtime(&mut desired, &before);
        if keep && desired == before {
            self.put_job(&job, None)?;
            return Ok(Some(job));
        }
        if keep {
            copy_legacy_runtime(&mut desired, &doc.legacy_jobs[index]);
            doc.legacy_jobs[index] = desired;
        } else {
            doc.legacy_jobs.remove(index);
        }
        self.catalog
            .compare_and_swap_legacy_jobs(doc.revision, doc.legacy_jobs)?;
        if keep {
            self.put_job(&job, None)?;
        } else {
            self.write_execute(
                "DELETE FROM cron_jobs WHERE id=?1 AND session_epoch IS NULL",
                [id],
            )?;
        }
        Ok(Some(job))
    }
    fn reconcile_legacy_document(&self, doc: &CatalogDocument) -> Result<(), String> {
        self.owner_epoch_for_write()?;
        let mut db = self.db()?;
        let tx = self.transaction(&mut db)?;
        let mut desired = std::collections::HashSet::new();
        for job in &doc.legacy_jobs {
            desired.insert(job.id.clone());
            let digest = legacy_job_digest(job)?;
            let current: Option<(String, String)> = tx
                .query_row(
                    "SELECT body,digest FROM cron_jobs WHERE id=?1 AND session_epoch IS NULL",
                    [&job.id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(err)?;
            let security_changed = current.as_ref().is_none_or(|(_, d)| d != &digest);
            let mut next = job.clone();
            if let Some((body, _)) = current {
                let previous: crate::types::Job = serde_json::from_str(&body).map_err(err)?;
                copy_legacy_runtime(&mut next, &previous);
                if next == previous {
                    continue;
                }
            }
            tx.execute("INSERT INTO cron_jobs(id,body,digest,enabled) VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET body=excluded.body,digest=excluded.digest,enabled=excluded.enabled,session_epoch=NULL",params![next.id,json(&next)?,digest,!next.is_paused()]).map_err(err)?;
            // An external portable edit is desired state, never a new grant.
            if security_changed {
                tx.execute("INSERT OR IGNORE INTO runs(run_id,job_id,occurrence,digest,owner_epoch,idempotent,status,effect_started) VALUES(?1,?2,?3,?4,'catalog',0,'needs_review',0)",params![uuid::Uuid::new_v4().to_string(),next.id,format!("catalog-review:{}",doc.revision),digest]).map_err(err)?;
            }
        }
        let existing: Vec<String> = {
            let mut stmt = tx
                .prepare("SELECT id FROM cron_jobs WHERE session_epoch IS NULL")
                .map_err(err)?;
            let rows = stmt.query_map([], |r| r.get(0)).map_err(err)?;
            rows.collect::<Result<_, _>>().map_err(err)?
        };
        for id in existing {
            if !desired.contains(&id) {
                tx.execute("DELETE FROM cron_jobs WHERE id=?1", [id])
                    .map_err(err)?;
            }
        }
        tx.commit().map_err(err)?;
        Ok(())
    }
    pub fn put_session_job(
        &self,
        job: &crate::types::Job,
        owner_epoch: &str,
    ) -> Result<(), String> {
        self.writable()?;
        self.put_job(job, Some(owner_epoch))
    }
    fn put_job(&self, job: &crate::types::Job, epoch: Option<&str>) -> Result<(), String> {
        self.writable()?;
        identifier(&job.id)?;
        let body = json(job)?;
        if body.len() > 1048576 {
            return Err("job size limit".into());
        }
        let changed=self.write_execute("INSERT INTO cron_jobs(id,body,digest,enabled,session_epoch) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET body=excluded.body,digest=excluded.digest,enabled=excluded.enabled,session_epoch=excluded.session_epoch WHERE (?5 IS NULL OR cron_jobs.session_epoch=?5)", params![job.id,body,legacy_job_digest(job)?,!job.is_paused(),epoch]).map_err(err)?;
        if changed == 0 {
            return Err("session job cannot replace another authority".into());
        }
        Ok(())
    }
    pub fn remove_legacy_job(&self, id: &str) -> Result<bool, String> {
        self.owner_epoch_for_write()?;
        if self.observer {
            return Err("read-only automation observer".into());
        }
        let _gate = self.catalog_gate.lock().map_err(err)?;
        let mut doc = self.catalog.read()?;
        let before = doc.legacy_jobs.len();
        doc.legacy_jobs.retain(|job| job.id != id);
        if before != doc.legacy_jobs.len() {
            self.catalog
                .compare_and_swap_legacy_jobs(doc.revision, doc.legacy_jobs)?;
        }
        Ok(self
            .write_execute("DELETE FROM cron_jobs WHERE id=?1", [id])
            .map_err(err)?
            > 0)
    }
    pub fn claim_run(
        &self,
        job_id: &str,
        occurrence_key: &str,
        config_digest: &str,
        owner_epoch: &str,
        idempotent: bool,
    ) -> Result<Option<RunClaim>, String> {
        self.claim_run_explicit(
            job_id,
            occurrence_key,
            config_digest,
            owner_epoch,
            idempotent,
            false,
        )
    }
    pub fn claim_run_explicit(
        &self,
        job_id: &str,
        occurrence_key: &str,
        config_digest: &str,
        owner_epoch: &str,
        idempotent: bool,
        allow_paused: bool,
    ) -> Result<Option<RunClaim>, String> {
        self.check_owner(Some(owner_epoch))?;
        self.reconcile_legacy_catalog()?;
        identifier(job_id)?;
        identifier(owner_epoch)?;
        if occurrence_key.len() > 512 {
            return Err("occurrence key limit".into());
        }
        self.owner_epoch_for_write()?;
        let mut db = self.db()?;
        let tx = self.transaction(&mut db)?;
        let eligible: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM cron_jobs WHERE id=?1 AND digest=?2 AND (enabled=1 OR ?4=1) AND (session_epoch IS NULL OR session_epoch=?3) AND NOT EXISTS(SELECT 1 FROM runs WHERE job_id=?1 AND status IN ('unknown','needs_review')))", params![job_id,config_digest,owner_epoch,allow_paused], |r| r.get(0)).map_err(err)?;
        if !eligible {
            return Ok(None);
        }
        let pending: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM runs WHERE status IN ('claimed','unknown','needs_review')",
                [],
                |r| r.get(0),
            )
            .map_err(err)?;
        if pending >= 4096 {
            return Err("run control record capacity exhausted".into());
        }
        tx.execute("DELETE FROM runs WHERE run_id IN (SELECT run_id FROM runs WHERE status IN ('succeeded','failed') ORDER BY rowid DESC LIMIT -1 OFFSET 4096)",[]).map_err(err)?;
        let claim = RunClaim {
            run_id: uuid::Uuid::new_v4().to_string(),
            job_id: job_id.into(),
            occurrence_key: occurrence_key.into(),
            config_digest: config_digest.into(),
            owner_epoch: owner_epoch.into(),
        };
        let count = tx.execute("INSERT OR IGNORE INTO runs(run_id,job_id,occurrence,digest,owner_epoch,idempotent,status,effect_started) VALUES(?1,?2,?3,?4,?5,?6,'claimed',0)", params![claim.run_id,job_id,occurrence_key,config_digest,owner_epoch,idempotent]).map_err(err)?;
        tx.commit().map_err(err)?;
        Ok((count > 0).then_some(claim))
    }
    pub fn claim_definition_run(
        &self,
        id: &str,
        occurrence_key: &str,
        owner_epoch: &str,
    ) -> Result<Option<RunClaim>, String> {
        self.check_owner(Some(owner_epoch))?;
        identifier(id)?;
        if occurrence_key.is_empty() || occurrence_key.len() > 512 {
            return Err("occurrence key bounds".into());
        }
        let definitions = self.definitions()?;
        let Some(definition) = definitions.into_iter().find(|d| d.id == id) else {
            return Ok(None);
        };
        if !definition.enabled {
            return Ok(None);
        }
        let digest = definition.security_digest()?;
        let mut db = self.db()?;
        let tx = self.transaction(&mut db)?;
        let eligible:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM definitions WHERE id=?1 AND digest=?2 AND approved=1) AND NOT EXISTS(SELECT 1 FROM runs WHERE job_id=?1 AND status IN ('unknown','needs_review'))",params![id,digest],|r|r.get(0)).map_err(err)?;
        if !eligible {
            return Ok(None);
        }
        let pending: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM runs WHERE status IN ('claimed','unknown','needs_review')",
                [],
                |r| r.get(0),
            )
            .map_err(err)?;
        if pending >= 4096 {
            return Err("run control record capacity exhausted".into());
        }
        let claim = RunClaim {
            run_id: uuid::Uuid::new_v4().to_string(),
            job_id: id.into(),
            occurrence_key: occurrence_key.into(),
            config_digest: digest,
            owner_epoch: owner_epoch.into(),
        };
        let count=tx.execute("INSERT OR IGNORE INTO runs(run_id,job_id,occurrence,digest,owner_epoch,idempotent,status,effect_started) VALUES(?1,?2,?3,?4,?5,?6,'claimed',0)",params![claim.run_id,id,occurrence_key,claim.config_digest,owner_epoch,definition.policy.idempotent]).map_err(err)?;
        prune_completed(&tx)?;
        tx.commit().map_err(err)?;
        Ok((count > 0).then_some(claim))
    }
    pub fn mark_effect_started(&self, claim: &RunClaim) -> Result<bool, String> {
        self.check_owner(Some(&claim.owner_epoch))?;
        self.reconcile_legacy_catalog()?;
        self.definitions()?;
        self.owner_epoch_for_write()?;
        let mut db = self.db()?;
        let tx = self.transaction(&mut db)?;
        let reserved:i64=tx.query_row("SELECT (SELECT COUNT(*) FROM outbox WHERE acked=0 AND failed=0)+(SELECT COUNT(*) FROM runs WHERE status='claimed' AND effect_started=1)",[],|r|r.get(0)).map_err(err)?;
        reserve_delivery_records(&tx, 1, true)?;
        if reserved >= 4096 {
            return Err("delivery reservation capacity exhausted".into());
        }
        let changed=tx.execute("UPDATE runs SET effect_started=1 WHERE run_id=?1 AND owner_epoch=?2 AND status='claimed' AND effect_started=0 AND (EXISTS(SELECT 1 FROM cron_jobs WHERE id=runs.job_id AND digest=runs.digest) OR EXISTS(SELECT 1 FROM definitions WHERE id=runs.job_id AND digest=runs.digest AND approved=1))",params![claim.run_id,claim.owner_epoch]).map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(changed > 0)
    }
    /// Recognize an already accepted occurrence without redelivering its prompt.
    pub fn is_run_succeeded(
        &self,
        job_id: &str,
        occurrence: &str,
        digest: &str,
        owner_epoch: &str,
    ) -> Result<bool, String> {
        self.check_owner(Some(owner_epoch))?;
        self.reconcile_legacy_catalog()?;
        let mut db = self.db()?;
        let tx = self.transaction(&mut db)?;
        let succeeded = tx.query_row("SELECT EXISTS(SELECT 1 FROM runs JOIN cron_jobs ON cron_jobs.id=runs.job_id AND cron_jobs.digest=runs.digest WHERE runs.job_id=?1 AND runs.occurrence=?2 AND runs.digest=?3 AND runs.status='succeeded' AND cron_jobs.enabled=1 AND (cron_jobs.session_epoch IS NULL OR cron_jobs.session_epoch=?4) AND NOT EXISTS(SELECT 1 FROM runs AS held WHERE held.job_id=?1 AND held.status IN ('unknown','needs_review')))", params![job_id,occurrence,digest,owner_epoch], |r| r.get(0)).map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(succeeded)
    }
    /// Only an explicit host enqueue rejection proves this agent effect never
    /// happened. Errors/cancellation/unknown outcomes must never use this port.
    pub fn release_rejected_agent_enqueue(&self, claim: &RunClaim) -> Result<bool, String> {
        self.check_owner(Some(&claim.owner_epoch))?;
        self.reconcile_legacy_catalog()?;
        let mut db = self.db()?;
        let tx = self.transaction(&mut db)?;
        let body: Option<String> = tx
            .query_row(
                "SELECT body FROM cron_jobs WHERE id=?1 AND digest=?2",
                params![claim.job_id, claim.config_digest],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        let Some(body) = body else {
            return Ok(false);
        };
        let job: crate::types::Job = serde_json::from_str(&body).map_err(err)?;
        if !matches!(job.action, crate::types::Action::AgentTurn { .. }) {
            return Ok(false);
        }
        let changed = tx.execute("DELETE FROM runs WHERE run_id=?1 AND job_id=?2 AND owner_epoch=?3 AND digest=?4 AND status='claimed'", params![claim.run_id,claim.job_id,claim.owner_epoch,claim.config_digest]).map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(changed > 0)
    }
    pub fn finish_run(
        &self,
        claim: &RunClaim,
        status: RunStatus,
        result: Option<&Value>,
        delivery_error: Option<&str>,
    ) -> Result<bool, String> {
        self.complete_run_and_enqueue_delivery(claim, status, result, delivery_error, None)
    }
    /// Completion and prepared delivery are one durable transaction. No effect is repeated for delivery retry.
    pub fn complete_run_and_enqueue_delivery(
        &self,
        claim: &RunClaim,
        status: RunStatus,
        result: Option<&Value>,
        delivery_error: Option<&str>,
        delivery: Option<(&Value, &str)>,
    ) -> Result<bool, String> {
        self.check_owner(Some(&claim.owner_epoch))?;
        let prepared = delivery
            .map(|(payload, target)| {
                identifier(target)?;
                let body = json(payload)?;
                Ok::<_, String>((body, target))
            })
            .transpose()?;
        self.owner_epoch_for_write()?;
        let mut db = self.db()?;
        let tx = self.transaction(&mut db)?;
        let changed=tx.execute("UPDATE runs SET status=?3,result=?4,delivery_error=?5 WHERE run_id=?1 AND owner_epoch=?2 AND status='claimed' AND digest=?6",params![claim.run_id,claim.owner_epoch,status.as_str(),result.map(json).transpose()?,delivery_error,claim.config_digest]).map_err(err)?;
        if changed == 0 {
            return Ok(false);
        }
        if let Some((data, target)) = prepared {
            reserve_delivery_records(&tx, 1, true)?;
            let pending: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM outbox WHERE acked=0 AND failed=0",
                    [],
                    |r| r.get(0),
                )
                .map_err(err)?;
            if pending >= 4096 {
                return Err("outbox capacity exhausted during completion".into());
            }
            tx.execute("INSERT OR IGNORE INTO events(app,source,digest,producer,event_id,data,at_ms) VALUES('cron',?1,?2,'run',?3,?4,0)",params![claim.job_id,claim.config_digest,claim.run_id,data]).map_err(err)?;
            let seq: i64 = tx
                .query_row(
                    "SELECT seq FROM events WHERE app='cron' AND source=?1 AND event_id=?2",
                    params![claim.job_id, claim.run_id],
                    |r| r.get(0),
                )
                .map_err(err)?;
            tx.execute(
                "INSERT OR IGNORE INTO outbox(event_seq,target,data,next_ms) VALUES(?1,?2,?3,0)",
                params![seq, target, data],
            )
            .map_err(err)?;
        }
        prune_completed(&tx)?;
        tx.commit().map_err(err)?;
        Ok(true)
    }
    /// Only never-started claims are released. External effects are never replayed automatically.
    pub fn recover_inflight(&self, new_owner_epoch: &str) -> Result<usize, String> {
        self.check_owner(Some(new_owner_epoch))?;
        self.owner_epoch_for_write()?;
        let mut db = self.db()?;
        let tx = self.transaction(&mut db)?;
        tx.execute(
            "DELETE FROM cron_jobs WHERE session_epoch IS NOT NULL AND session_epoch<>?1",
            [new_owner_epoch],
        )
        .map_err(err)?;
        tx.execute(
            "DELETE FROM runs WHERE status='claimed' AND effect_started=0 AND owner_epoch<>?1",
            [new_owner_epoch],
        )
        .map_err(err)?;
        let count = tx
            .execute(
                "UPDATE runs SET status='unknown' WHERE status='claimed' AND owner_epoch<>?1",
                [new_owner_epoch],
            )
            .map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(count)
    }
    pub fn reconcile_app_source(
        &self,
        app: &str,
        source: &str,
        digest: &str,
        spec: &Value,
    ) -> Result<ReconcileStatus, String> {
        self.writable()?;
        identifier(app)?;
        identifier(source)?;
        let body = json(spec)?;
        if body.len() > 1048576 || digest.len() > 256 {
            return Err("compiled spec bounds".into());
        }
        self.owner_epoch_for_write()?;
        let mut db = self.db()?;
        let tx = self.transaction(&mut db)?;
        let previous: Option<String> = tx
            .query_row(
                "SELECT digest FROM sources WHERE app=?1 AND source=?2 AND tombstoned=0",
                params![app, source],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        if previous.as_deref() == Some(digest) {
            return Ok(ReconcileStatus::Unchanged);
        }
        let control = json(&SourceControl::default())?;
        tx.execute("INSERT INTO sources(app,source,digest,spec,epoch,approved,control) VALUES(?1,?2,?3,?4,?5,0,?6) ON CONFLICT(app,source) DO UPDATE SET digest=excluded.digest,spec=excluded.spec,epoch=excluded.epoch,approved=0,tombstoned=0,control=excluded.control,grant_json=NULL,cursor=NULL,latest=NULL",params![app,source,digest,body,uuid::Uuid::new_v4().to_string(),control]).map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(ReconcileStatus::NeedsApproval)
    }
    pub fn source_state(&self, app: &str, source: &str) -> Result<Option<SourceRuntime>, String> {
        let db = self.db()?;
        db.query_row("SELECT digest,epoch,approved,control,grant_json FROM sources WHERE app=?1 AND source=?2",params![app,source], runtime_row).optional().map_err(err)?.map(parse_runtime).transpose()
    }
    pub fn remove_source(&self, app: &str, source: &str) -> Result<bool, String> {
        self.writable()?;
        Ok(self.write_execute("UPDATE sources SET tombstoned=1,approved=0,grant_json=NULL,control=?3,epoch=?4 WHERE app=?1 AND source=?2",params![app,source,json(&SourceControl::default())?,uuid::Uuid::new_v4().to_string()]).map_err(err)?>0)
    }
    pub fn list_sources(&self) -> Result<Vec<SourceRecord>, String> {
        let db = self.db()?;
        let mut stmt=db.prepare("SELECT app,source,spec,digest,epoch,approved,control,grant_json FROM sources WHERE tombstoned=0 ORDER BY app,source").map_err(err)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    (r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?),
                ))
            })
            .map_err(err)?;
        rows.map(|r| {
            let (app_id, source_id, spec, runtime) = r.map_err(err)?;
            Ok(SourceRecord {
                app_id,
                source_id,
                compiled_spec: serde_json::from_str(&spec).map_err(err)?,
                runtime: parse_runtime(runtime)?,
            })
        })
        .collect()
    }
    pub fn grant_source(&self, grant: &GrantRecord) -> Result<(), String> {
        self.writable()?;
        if grant.credential_refs.len() > 32 {
            return Err("credential reference bounds".into());
        }
        let body = json(grant)?;
        if body.len() > 16384 {
            return Err("grant bounds".into());
        }
        let count=self.write_execute("UPDATE sources SET approved=?4,grant_json=?5,epoch=?6 WHERE app=?1 AND source=?2 AND digest=?3 AND tombstoned=0",params![grant.app_id,grant.source_id,grant.config_digest,grant.granted,body,uuid::Uuid::new_v4().to_string()]).map_err(err)?;
        if count == 0 {
            return Err("source configuration changed".into());
        }
        Ok(())
    }
    pub fn set_source_control(
        &self,
        app: &str,
        source: &str,
        control: &SourceControl,
    ) -> Result<(), String> {
        self.writable()?;
        let count = self
            .write_execute(
                "UPDATE sources SET control=?3,epoch=?4 WHERE app=?1 AND source=?2",
                params![
                    app,
                    source,
                    json(control)?,
                    uuid::Uuid::new_v4().to_string()
                ],
            )
            .map_err(err)?;
        if count == 0 {
            return Err("source not configured".into());
        }
        Ok(())
    }
    pub fn ingest(&self, request: &IngestRequest) -> Result<IngestOutcome, String> {
        self.check_owner(None)?;
        identifier(&request.producer_id)?;
        if request.event_id.len() > 512
            || request.deliveries.len() > 32
            || request.cursor.as_ref().is_some_and(|c| c.len() > 4096)
        {
            return Err("ingest bounds".into());
        }
        let event_id = if request.event_id.is_empty() {
            uuid::Uuid::new_v4().to_string()
        } else {
            request.event_id.clone()
        };
        let producer = json(&(
            request.producer_id.as_str(),
            request.producer_epoch.as_str(),
        ))?;
        let data = json(&request.data)?;
        self.owner_epoch_for_write()?;
        let mut db = self.db()?;
        let tx = self.transaction(&mut db)?;
        let runtime=tx.query_row("SELECT digest,epoch,approved,control,grant_json FROM sources WHERE app=?1 AND source=?2",params![request.app_id,request.source_id],runtime_row).optional().map_err(err)?.ok_or_else(||"source not configured".to_string()).and_then(parse_runtime)?;
        let c = &runtime.control;
        if runtime.config_digest != request.source_config_digest
            || runtime.source_generation != request.source_generation
            || !runtime.approved
            || c.paused
            || (!c.demand && !c.background)
            || c.expires_at_ms.is_some_and(|t| t <= request.at_ms)
        {
            return Err("stale or unauthorized source ingest".into());
        }
        let duplicate: bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM events WHERE app=?1 AND source=?2 AND digest=?3 AND producer=?4 AND event_id=?5)",params![request.app_id,request.source_id,request.source_config_digest,producer,event_id],|r|r.get(0)).map_err(err)?;
        if duplicate {
            return Ok(IngestOutcome::Duplicate);
        }
        reserve_delivery_records(&tx, request.deliveries.len() as i64, true)?;
        let pending: i64 = tx
            .query_row("SELECT (SELECT COUNT(*) FROM outbox WHERE acked=0 AND failed=0)+(SELECT COUNT(*) FROM runs WHERE status='claimed' AND effect_started=1)", [], |r| {
                r.get(0)
            })
            .map_err(err)?;
        if pending + request.deliveries.len() as i64 > 4096 {
            return Err("outbox capacity exhausted; checkpoint not advanced".into());
        }
        // Terminal payloads are bounded; pending delivery is never pruned.
        tx.execute("DELETE FROM events WHERE seq IN (SELECT seq FROM events WHERE NOT EXISTS(SELECT 1 FROM outbox WHERE event_seq=events.seq AND acked=0 AND failed=0) ORDER BY seq DESC LIMIT -1 OFFSET 4095)",[]).map_err(err)?;
        tx.execute("DELETE FROM outbox WHERE acked=1 AND NOT EXISTS(SELECT 1 FROM events WHERE seq=outbox.event_seq)",[]).map_err(err)?;
        clear_retired_failed_payloads(&tx)?;
        let count: i64 = tx
            .query_row("SELECT COUNT(*) FROM events", [], |r| r.get(0))
            .map_err(err)?;
        if count >= 8192 {
            return Err("event capacity exhausted".into());
        }
        tx.execute("INSERT INTO events(app,source,digest,producer,event_id,data,at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![request.app_id,request.source_id,request.source_config_digest,producer,event_id,data,request.at_ms]).map_err(err)?;
        let seq = tx.last_insert_rowid();
        for target in &request.deliveries {
            identifier(target)?;
            tx.execute(
                "INSERT OR IGNORE INTO outbox(event_seq,target,data,next_ms) VALUES(?1,?2,?3,?4)",
                params![seq, target, data, request.at_ms],
            )
            .map_err(err)?;
        }
        tx.execute(
            "UPDATE sources SET cursor=?3,latest=?4 WHERE app=?1 AND source=?2",
            params![request.app_id, request.source_id, request.cursor, data],
        )
        .map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(IngestOutcome::Inserted {
            event_seq: seq,
        })
    }
    pub fn latest(&self, app: &str, source: &str) -> Result<Option<Value>, String> {
        self.source_text(app, source, "latest")?
            .map(|s| serde_json::from_str(&s).map_err(err))
            .transpose()
    }
    pub fn checkpoint(&self, app: &str, source: &str) -> Result<Option<String>, String> {
        self.source_text(app, source, "cursor")
    }
    fn source_text(&self, app: &str, source: &str, column: &str) -> Result<Option<String>, String> {
        self.db()?
            .query_row(
                &format!("SELECT {column} FROM sources WHERE app=?1 AND source=?2"),
                params![app, source],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)
            .map(Option::flatten)
    }
    pub fn pending_deliveries(
        &self,
        limit: usize,
        now_ms: u64,
    ) -> Result<Vec<OutboxDelivery>, String> {
        let db = self.db()?;
        let mut stmt=db.prepare("SELECT event_seq,target,data,attempts,next_ms FROM outbox WHERE acked=0 AND failed=0 AND next_ms<=?1 ORDER BY next_ms,event_seq LIMIT ?2").map_err(err)?;
        let rows = stmt
            .query_map(params![now_ms, limit.min(256)], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, u32>(3)?,
                    r.get::<_, u64>(4)?,
                ))
            })
            .map_err(err)?;
        rows.map(|r| {
            let (event_seq, target, data, attempts, next_attempt_ms) = r.map_err(err)?;
            Ok(OutboxDelivery {
                event_seq,
                target,
                data: serde_json::from_str(&data).map_err(err)?,
                attempts,
                next_attempt_ms,
            })
        })
        .collect()
    }
    pub fn acknowledge_delivery(&self, id: i64, target: &str) -> Result<bool, String> {
        self.owner_epoch_for_write()?;
        let mut db = self.db()?;
        let tx = self.transaction(&mut db)?;
        let count = tx
            .execute(
                "UPDATE outbox SET acked=1 WHERE event_seq=?1 AND target=?2 AND acked=0 AND failed=0",
                params![id, target],
            )
            .map_err(err)?;
        prune_completed(&tx)?;
        tx.commit().map_err(err)?;
        Ok(count > 0)
    }
    pub fn retry_delivery(
        &self,
        id: i64,
        target: &str,
        next_ms: u64,
        error: &str,
    ) -> Result<bool, String> {
        self.owner_epoch_for_write()?;
        // Driver errors may contain credentials, response bodies or URLs.
        let _ = error;
        let mut db = self.db()?;
        let tx = self.transaction(&mut db)?;
        let changed = tx.execute("UPDATE outbox SET attempts=MIN(attempts+1,?5),next_ms=?3,error=?4,failed=(attempts+1>=?5),failed_definition=CASE WHEN attempts+1>=?5 THEN (SELECT source FROM events WHERE seq=outbox.event_seq AND app='cron') ELSE NULL END WHERE event_seq=?1 AND target=?2 AND acked=0 AND failed=0",params![id,target,next_ms,"delivery failed (details redacted)",MAX_DELIVERY_ATTEMPTS]).map_err(err)?;
        prune_completed(&tx)?;
        tx.commit().map_err(err)?;
        Ok(changed > 0)
    }
    /// Cheap per-definition status, including failures whose payload aged out.
    pub fn failed_delivery_count_for_definition(&self, id: &str) -> Result<usize, String> {
        identifier(id)?;
        self.db()?
            .query_row(
                "SELECT COUNT(*) FROM outbox WHERE failed=1 AND failed_definition=?1",
                [id],
                |r| r.get(0),
            )
            .map_err(err)
    }
    /// Bounded metadata-only operator inspection; no delivery payload is exposed.
    pub fn failed_deliveries(&self, limit: usize) -> Result<Vec<FailedDelivery>, String> {
        let db = self.db()?;
        let mut stmt = db.prepare("SELECT event_seq,target,attempts,failed_definition,EXISTS(SELECT 1 FROM events WHERE seq=outbox.event_seq) FROM outbox WHERE failed=1 ORDER BY event_seq DESC,target LIMIT ?1").map_err(err)?;
        let rows = stmt
            .query_map([limit.min(256)], |r| {
                Ok(FailedDelivery {
                    event_seq: r.get(0)?,
                    target: r.get(1)?,
                    attempts: r.get(2)?,
                    definition_id: r.get(3)?,
                    payload_retained: r.get(4)?,
                })
            })
            .map_err(err)?;
        rows.collect::<Result<_, _>>().map_err(err)
    }
    /// Host-operator-only recovery of one exact dead letter. Requires retained
    /// payload; callers must revalidate current target authority at delivery.
    /// This resets only delivery budget and never replays the original action.
    pub fn retry_failed_delivery(
        &self,
        id: i64,
        target: &str,
        next_ms: u64,
    ) -> Result<bool, String> {
        identifier(target)?;
        self.owner_epoch_for_write()?;
        let mut db = self.db()?;
        let tx = self.transaction(&mut db)?;
        let pending: i64 = tx.query_row("SELECT (SELECT COUNT(*) FROM outbox WHERE acked=0 AND failed=0)+(SELECT COUNT(*) FROM runs WHERE status='claimed' AND effect_started=1)", [], |r| r.get(0)).map_err(err)?;
        if pending >= 4096 {
            return Err("outbox capacity exhausted".into());
        }
        let changed = tx.execute("UPDATE outbox SET failed=0,failed_definition=NULL,attempts=0,next_ms=?3,error=NULL WHERE event_seq=?1 AND target=?2 AND failed=1 AND acked=0 AND EXISTS(SELECT 1 FROM events WHERE seq=outbox.event_seq)", params![id,target,next_ms]).map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(changed > 0)
    }
    pub fn run_delivery_id(&self, claim: &RunClaim) -> Result<Option<i64>, String> {
        self.check_owner(Some(&claim.owner_epoch))?;
        self.db()?.query_row("SELECT seq FROM events WHERE app='cron' AND source=?1 AND event_id=?2 AND digest=?3",params![claim.job_id,claim.run_id,claim.config_digest],|r|r.get(0)).optional().map_err(err)
    }
    pub fn enqueue_run_delivery(
        &self,
        claim: &RunClaim,
        payload: &Value,
        target: &str,
    ) -> Result<i64, String> {
        self.check_owner(Some(&claim.owner_epoch))?;
        self.writable()?;
        identifier(target)?;
        let data = json(payload)?;
        self.owner_epoch_for_write()?;
        let mut db = self.db()?;
        let tx = self.transaction(&mut db)?;
        let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM runs WHERE run_id=?1 AND owner_epoch=?2 AND status IN ('claimed','succeeded','failed'))",params![claim.run_id,claim.owner_epoch],|r|r.get(0)).map_err(err)?;
        if !valid {
            return Err("stale run claim".into());
        }
        reserve_delivery_records(&tx, 1, true)?;
        let pending: i64 = tx
            .query_row("SELECT (SELECT COUNT(*) FROM outbox WHERE acked=0 AND failed=0)+(SELECT COUNT(*) FROM runs WHERE status='claimed' AND effect_started=1)", [], |r| {
                r.get(0)
            })
            .map_err(err)?;
        if pending >= 4096 {
            return Err("outbox quota".into());
        }
        tx.execute("INSERT OR IGNORE INTO events(app,source,digest,producer,event_id,data,at_ms) VALUES('cron',?1,?2,'run',?3,?4,0)",params![claim.job_id,claim.config_digest,claim.run_id,data]).map_err(err)?;
        let seq: i64 = tx
            .query_row(
                "SELECT seq FROM events WHERE app='cron' AND source=?1 AND event_id=?2",
                params![claim.job_id, claim.run_id],
                |r| r.get(0),
            )
            .map_err(err)?;
        tx.execute(
            "INSERT OR IGNORE INTO outbox(event_seq,target,data,next_ms) VALUES(?1,?2,?3,0)",
            params![seq, target, data],
        )
        .map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(seq)
    }

    pub fn legacy_authority_retired(&self) -> Result<bool, String> {
        Ok(self
            .db()?
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM metadata WHERE key='legacy_migration' AND value='1')",
                [],
                |r| r.get(0),
            )
            .map_err(err)?)
    }
    pub fn migration_complete(&self) -> Result<bool, String> {
        self.legacy_authority_retired()
    }
    /// Import is SQL-idempotent. The caller must retain the exclusive project lease.
    /// Rename the legacy authority before SQL import; a crash resumes from the backup.
    pub fn migrate_legacy_jobs(
        &self,
        root: impl AsRef<Path>,
        _lease: &super::ProjectAutomationLease,
    ) -> Result<usize, String> {
        self.owner_epoch_for_write()?;
        self.writable()?;
        if _lease.root != self.root
            || std::fs::canonicalize(root.as_ref()).map_err(err)? != self.root
        {
            return Err("lease project mismatch".into());
        }
        use std::io::Read;
        let dir = child(&root_dir(root.as_ref())?, ".refact", false)?;
        if self.legacy_authority_retired()? {
            return Ok(0);
        }
        let original = "scheduled_tasks.json";
        let backup = "scheduled_tasks.migrated.json";
        let exists = |name: &str| -> Result<bool, String> {
            match dir.symlink_metadata(name) {
                Ok(_) => {
                    regular(&dir, name, false, false)?;
                    Ok(true)
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
                Err(e) => Err(err(e)),
            }
        };
        let has_backup = exists(backup)?;
        if exists(original)? {
            if has_backup {
                return Err(
                    "both legacy authority and migration backup exist; review required".into(),
                );
            }
            dir.rename(original, &dir, backup).map_err(err)?;
            dir.open(".").map_err(err)?.sync_all().map_err(err)?;
        }
        let mut jobs = Vec::<crate::types::Job>::new();
        if exists(backup)? {
            let file = regular(&dir, backup, false, false)?;
            if file.metadata().map_err(err)?.len() > 16 * 1024 * 1024 {
                return Err("legacy migration size limit".into());
            }
            let mut bytes = Vec::new();
            file.take(16 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(err)?;
            jobs = serde_json::from_slice(&bytes).map_err(err)?;
        }
        let mut ids = std::collections::HashSet::new();
        for job in &jobs {
            identifier(&job.id)?;
            if !ids.insert(&job.id) {
                return Err("duplicate legacy job ids".into());
            }
        }
        if self.observer {
            return Err("read-only automation observer".into());
        }
        let _gate = self.catalog_gate.lock().map_err(err)?;
        let mut doc = self.catalog.read()?;
        for job in &jobs {
            if !doc.legacy_jobs.iter().any(|j| j.id == job.id) {
                doc.legacy_jobs.push(job.clone());
            }
        }
        self.catalog
            .compare_and_swap_legacy_jobs(doc.revision, doc.legacy_jobs)?;
        self.owner_epoch_for_write()?;
        let mut db = self.db()?;
        let tx = self.transaction(&mut db)?;
        for job in &jobs {
            tx.execute(
                "INSERT OR IGNORE INTO cron_jobs(id,body,digest,enabled) VALUES(?1,?2,?3,?4)",
                params![
                    job.id,
                    json(job)?,
                    legacy_job_digest(job)?,
                    !job.is_paused()
                ],
            )
            .map_err(err)?;
            if job.retry_attempts > 0
                || job
                    .last_status
                    .as_deref()
                    .is_some_and(|s| matches!(s, "running" | "started" | "error"))
            {
                tx.execute("INSERT OR IGNORE INTO runs(run_id,job_id,occurrence,digest,owner_epoch,idempotent,status,effect_started) VALUES(?1,?2,'migration-review',?3,'migration',0,'needs_review',1)",params![uuid::Uuid::new_v4().to_string(),job.id,legacy_job_digest(job)?]).map_err(err)?;
            }
        }
        tx.execute("INSERT INTO metadata(key,value) VALUES('legacy_migration','1') ON CONFLICT(key) DO UPDATE SET value='1'",[]).map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(jobs.len())
    }
    pub fn definition_schedule_state(
        &self,
        id: &str,
    ) -> Result<Option<DefinitionScheduleState>, String> {
        self.db()?
            .query_row(
                "SELECT created_at_ms,last_fired_at_ms FROM definitions WHERE id=?1",
                [id],
                |r| {
                    Ok(DefinitionScheduleState {
                        created_at_ms: r.get(0)?,
                        last_fired_at_ms: r.get(1)?,
                    })
                },
            )
            .optional()
            .map_err(err)
    }
    pub fn advance_definition_schedule(
        &self,
        id: &str,
        security_digest: &str,
        last_fired_at_ms: u64,
    ) -> Result<bool, String> {
        self.owner_epoch_for_write()?;
        self.definitions()?;
        Ok(self.write_execute("UPDATE definitions SET last_fired_at_ms=?3 WHERE id=?1 AND digest=?2 AND approved=1 AND (last_fired_at_ms IS NULL OR last_fired_at_ms<?3)",params![id,security_digest,last_fired_at_ms])?>0)
    }
    pub fn definitions(&self) -> Result<Vec<Definition>, String> {
        let document = self.catalog.read()?;
        if !self.observer {
            self.reconcile_catalog(&document)?;
        }
        Ok(document.definitions)
    }
    /// Host fail-closed port: discard compiled project authority even when the
    /// canonical catalog is unreadable. This port can only revoke, never grant.
    pub fn revoke_compiled_definitions(&self) -> Result<(), String> {
        self.owner_epoch_for_write()?;
        self.writable()?;
        self.write_execute("UPDATE definitions SET approved=0", [])?;
        Ok(())
    }

    pub fn revoke_compiled_definition(&self, id: &str, digest: &str) -> Result<(), String> {
        self.owner_epoch_for_write()?;
        self.writable()?;
        self.write_execute(
            "UPDATE definitions SET approved=0 WHERE id=?1 AND digest=?2",
            params![id, digest],
        )?;
        Ok(())
    }

    pub fn approve_definition(
        &self,
        id: &str,
        security_digest: &str,
        approved: bool,
    ) -> Result<(), String> {
        self.owner_epoch_for_write()?;
        self.writable()?;
        let document = self.catalog.read()?;
        self.reconcile_catalog(&document)?;
        if !document
            .definitions
            .iter()
            .any(|d| d.id == id && d.security_digest().ok().as_deref() == Some(security_digest))
        {
            return Err("canonical definition changed".into());
        }
        let count = self
            .write_execute(
                "UPDATE definitions SET approved=?3 WHERE id=?1 AND digest=?2",
                params![id, security_digest, approved],
            )
            .map_err(err)?;
        if count == 0 {
            return Err("definition not found or digest changed".into());
        }
        Ok(())
    }
    pub fn definition_status(&self, id: &str) -> Result<Option<ReconcileStatus>, String> {
        let document = self.catalog.read()?;
        if !self.observer {
            self.reconcile_catalog(&document)?;
        }
        self.db()?
            .query_row("SELECT approved FROM definitions WHERE id=?1", [id], |r| {
                r.get::<_, bool>(0)
            })
            .optional()
            .map_err(err)
            .map(|status| {
                status.map(|approved| {
                    if approved {
                        ReconcileStatus::Approved
                    } else {
                        ReconcileStatus::NeedsApproval
                    }
                })
            })
    }
    /// Reconcile only canonical project definitions. Removed definitions lose compiled authority.
    pub fn reconcile_catalog(&self, document: &CatalogDocument) -> Result<(), String> {
        self.owner_epoch_for_write()?;
        self.writable()?;
        let _gate = self.catalog_gate.lock().map_err(err)?;
        let canonical = self.catalog.read()?;
        if serde_json::to_value(&canonical).map_err(err)?
            != serde_json::to_value(document).map_err(err)?
        {
            return Err("stale or noncanonical catalog reconciliation".into());
        }
        self.owner_epoch_for_write()?;
        let mut db = self.db()?;
        let tx = self.transaction(&mut db)?;
        let mut ids = std::collections::HashSet::new();
        for definition in &document.definitions {
            definition.validate()?;
            if !matches!(definition.owner, DefinitionOwner::Project) || !ids.insert(&definition.id)
            {
                return Err("invalid project catalog ownership/identity".into());
            }
            tx.execute("INSERT INTO definitions(id,revision,digest,body,created_at_ms) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,approved=CASE WHEN definitions.digest=excluded.digest THEN definitions.approved ELSE 0 END,digest=excluded.digest,body=excluded.body",params![definition.id,definition.revision,definition.security_digest()?,json(definition)?,std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_err(err)?.as_millis() as u64]).map_err(err)?;
        }
        let existing: Vec<String> = {
            let mut statement = tx.prepare("SELECT id FROM definitions").map_err(err)?;
            let rows = statement.query_map([], |r| r.get(0)).map_err(err)?;
            rows.collect::<Result<_, _>>().map_err(err)?
        };
        for id in existing {
            if !ids.contains(&id) {
                tx.execute("DELETE FROM definitions WHERE id=?1", [id])
                    .map_err(err)?;
            }
        }
        tx.execute("INSERT INTO metadata(key,value) VALUES('catalog_revision',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[document.revision.to_string()]).map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(())
    }
    pub fn pending_deliveries_for_targets(
        &self,
        targets: &[String],
        limit: usize,
        now_ms: u64,
    ) -> Result<Vec<OutboxDelivery>, String> {
        if targets.len() > 128 {
            return Err("delivery target bounds".into());
        }
        let mut all = Vec::new();
        for target in targets {
            identifier(target)?;
            let db = self.db()?;
            let mut statement=db.prepare("SELECT event_seq,data,attempts,next_ms FROM outbox WHERE acked=0 AND failed=0 AND target=?1 AND next_ms<=?2 ORDER BY next_ms,event_seq LIMIT ?3").map_err(err)?;
            let rows = statement
                .query_map(params![target, now_ms, limit.min(256)], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, u32>(2)?,
                        r.get::<_, u64>(3)?,
                    ))
                })
                .map_err(err)?;
            for row in rows {
                let (event_seq, data, attempts, next_attempt_ms) = row.map_err(err)?;
                all.push(OutboxDelivery {
                    event_seq,
                    target: target.clone(),
                    data: serde_json::from_str(&data).map_err(err)?,
                    attempts,
                    next_attempt_ms,
                });
            }
        }
        all.sort_by_key(|d| (d.next_attempt_ms, d.event_seq));
        all.truncate(limit.min(256));
        Ok(all)
    }
    pub fn wake_summary(&self) -> Result<WakeSummary, String> {
        self.wake_summary_in_timezone(chrono_tz::UTC)
    }
    pub fn wake_summary_in_timezone(&self, tz: chrono_tz::Tz) -> Result<WakeSummary, String> {
        self.wake_summary_at_in_timezone(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(err)?
                .as_millis() as u64,
            tz,
        )
    }
    pub fn wake_summary_at(&self, now_ms: u64) -> Result<WakeSummary, String> {
        self.wake_summary_at_in_timezone(now_ms, chrono_tz::UTC)
    }
    pub fn wake_summary_at_in_timezone(
        &self,
        now_ms: u64,
        tz: chrono_tz::Tz,
    ) -> Result<WakeSummary, String> {
        let db = self.db()?;
        let mut summary = WakeSummary::default();
        summary.failed_delivery = db
            .query_row("SELECT COUNT(*) FROM outbox WHERE failed=1", [], |r| {
                r.get(0)
            })
            .map_err(err)?;
        summary.pending_delivery = db
            .query_row(
                "SELECT COUNT(*) FROM outbox WHERE acked=0 AND failed=0",
                [],
                |r| r.get(0),
            )
            .map_err(err)?;
        summary.recovery = db
            .query_row(
                "SELECT COUNT(*) FROM runs WHERE status IN ('unknown','needs_review')",
                [],
                |r| r.get(0),
            )
            .map_err(err)?;
        summary.next_wake_ms = db
            .query_row(
                "SELECT MIN(next_ms) FROM outbox WHERE acked=0 AND failed=0",
                [],
                |r| r.get(0),
            )
            .map_err(err)?;
        let mut jobs=db.prepare("SELECT body FROM cron_jobs WHERE session_epoch IS NULL AND NOT EXISTS(SELECT 1 FROM runs WHERE job_id=cron_jobs.id AND status IN ('unknown','needs_review'))").map_err(err)?;
        for row in jobs.query_map([], |r| r.get::<_, String>(0)).map_err(err)? {
            let job: crate::types::Job = serde_json::from_str(&row.map_err(err)?).map_err(err)?;
            if job.auto_expire_after_ms > 0
                && now_ms >= job.created_at_ms.saturating_add(job.auto_expire_after_ms)
            {
                continue;
            }
            let explicit = job.trigger_at_ms;
            let scheduled = if job.is_paused() {
                None
            } else {
                let from = job.last_fired_at_ms.unwrap_or(job.created_at_ms);
                if job.recurring {
                    crate::schedule::recurring_missed_grace_state(
                        &job,
                        from,
                        now_ms,
                        tz,
                        Default::default(),
                    )
                    .map(|s| {
                        if s.should_fire {
                            now_ms
                        } else {
                            s.next_future_ms
                        }
                    })
                } else {
                    crate::schedule::next_run_ms(&job, from, tz)
                }
            };
            for next in [explicit, scheduled].into_iter().flatten() {
                summary.next_wake_ms = Some(summary.next_wake_ms.map_or(next, |old| old.min(next)));
            }
        }
        let mut stmt = db
            .prepare("SELECT control FROM sources WHERE approved=1 AND tombstoned=0")
            .map_err(err)?;
        for row in stmt.query_map([], |r| r.get::<_, String>(0)).map_err(err)? {
            let c: SourceControl = serde_json::from_str(&row.map_err(err)?).map_err(err)?;
            if c.background && !c.paused && !c.expires_at_ms.is_some_and(|t| t <= now_ms) {
                summary.background_active = true;
                if c.resume_after_restart {
                    summary.background_restart = true;
                    summary.next_wake_ms = match (summary.next_wake_ms, c.next_wake_ms) {
                        (Some(a), Some(b)) => Some(a.min(b)),
                        (a, b) => a.or(b),
                    };
                }
            }
        }
        Ok(summary)
    }
}
fn verify_transaction_owner(tx: &Connection, epoch: &str) -> Result<(), String> {
    let current: Option<String> = tx
        .query_row(
            "SELECT value FROM metadata WHERE key='owner_epoch'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(err)?;
    if current.as_deref() != Some(epoch) {
        return Err("stale execution owner inside transaction".into());
    }
    Ok(())
}
fn prune_completed(tx: &Connection) -> Result<(), String> {
    tx.execute("DELETE FROM events WHERE seq IN (SELECT seq FROM events WHERE NOT EXISTS(SELECT 1 FROM outbox WHERE event_seq=events.seq AND acked=0 AND failed=0) ORDER BY seq DESC LIMIT -1 OFFSET 4096)",[]).map_err(err)?;
    tx.execute("DELETE FROM outbox WHERE acked=1 AND NOT EXISTS(SELECT 1 FROM events WHERE seq=outbox.event_seq)",[]).map_err(err)?;
    clear_retired_failed_payloads(tx)?;
    tx.execute("DELETE FROM runs WHERE run_id IN (SELECT run_id FROM runs WHERE status IN ('succeeded','failed') ORDER BY rowid DESC LIMIT -1 OFFSET 4096)",[]).map_err(err)?;
    Ok(())
}
fn clear_retired_failed_payloads(tx: &Connection) -> Result<(), String> {
    tx.execute("UPDATE outbox SET data='null' WHERE failed=1 AND data<>'null' AND NOT EXISTS(SELECT 1 FROM events WHERE seq=outbox.event_seq)", []).map_err(err)?;
    Ok(())
}
fn reserve_delivery_records(
    tx: &Connection,
    additional: i64,
    include_reservations: bool,
) -> Result<(), String> {
    let count: i64 = tx.query_row("SELECT (SELECT COUNT(*) FROM outbox)+(CASE WHEN ?1 THEN (SELECT COUNT(*) FROM runs WHERE status='claimed' AND effect_started=1) ELSE 0 END)", [include_reservations], |r| r.get(0)).map_err(err)?;
    if count + additional > MAX_DELIVERY_RECORDS {
        return Err("delivery control record capacity exhausted; operator review required".into());
    }
    Ok(())
}
type RuntimeRow = (String, String, bool, String, Option<String>);
fn runtime_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<RuntimeRow> {
    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
}
fn parse_runtime(
    (config_digest, source_generation, approved, control, grant): RuntimeRow,
) -> Result<SourceRuntime, String> {
    Ok(SourceRuntime {
        config_digest,
        source_generation,
        approved,
        control: serde_json::from_str(&control).map_err(err)?,
        grant: grant
            .map(|s| serde_json::from_str(&s).map_err(err))
            .transpose()?,
    })
}
const SCHEMA:&str="PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;
CREATE TABLE IF NOT EXISTS cron_jobs(id TEXT PRIMARY KEY,body TEXT NOT NULL,digest TEXT NOT NULL,enabled INTEGER NOT NULL,session_epoch TEXT);
CREATE TABLE IF NOT EXISTS runs(run_id TEXT PRIMARY KEY,job_id TEXT NOT NULL,occurrence TEXT NOT NULL,digest TEXT NOT NULL,owner_epoch TEXT NOT NULL,idempotent INTEGER NOT NULL,status TEXT NOT NULL,effect_started INTEGER NOT NULL,result TEXT,delivery_error TEXT,UNIQUE(job_id,occurrence));
CREATE TABLE IF NOT EXISTS sources(app TEXT NOT NULL,source TEXT NOT NULL,digest TEXT NOT NULL,spec TEXT NOT NULL,epoch TEXT NOT NULL,approved INTEGER NOT NULL,control TEXT NOT NULL,tombstoned INTEGER NOT NULL DEFAULT 0,grant_json TEXT,cursor TEXT,latest TEXT,PRIMARY KEY(app,source));
CREATE TABLE IF NOT EXISTS events(seq INTEGER PRIMARY KEY AUTOINCREMENT,app TEXT NOT NULL,source TEXT NOT NULL,digest TEXT NOT NULL,producer TEXT NOT NULL,event_id TEXT NOT NULL,data TEXT NOT NULL,at_ms INTEGER NOT NULL,UNIQUE(app,source,digest,producer,event_id));
CREATE TABLE IF NOT EXISTS outbox(event_seq INTEGER NOT NULL,target TEXT NOT NULL,data TEXT NOT NULL,next_ms INTEGER NOT NULL,attempts INTEGER NOT NULL DEFAULT 0,acked INTEGER NOT NULL DEFAULT 0,failed INTEGER NOT NULL DEFAULT 0,failed_definition TEXT,error TEXT,PRIMARY KEY(event_seq,target));
CREATE TABLE IF NOT EXISTS definitions(id TEXT PRIMARY KEY,revision INTEGER NOT NULL,digest TEXT NOT NULL,body TEXT NOT NULL,approved INTEGER NOT NULL DEFAULT 0,created_at_ms INTEGER NOT NULL DEFAULT 0,last_fired_at_ms INTEGER);
CREATE TABLE IF NOT EXISTS metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS runs_status ON runs(status);
CREATE INDEX IF NOT EXISTS runs_job_status ON runs(job_id,status);
CREATE INDEX IF NOT EXISTS outbox_pending ON outbox(acked,event_seq);";

#[cfg(test)]
mod transaction_fence_tests {
    use super::*;
    #[test]
    fn runtime_mutation_rolls_back_without_catalog_changes_and_reconcile_keeps_trigger() {
        let root = tempfile::tempdir().unwrap();
        let journal = AutomationJournal::open(root.path()).unwrap();
        let lease = super::super::ProjectAutomationLease::acquire(root.path()).unwrap();
        journal.activate_owner(&lease).unwrap();
        let mut job = crate::types::Job::new_cron_agent_chat(
            "*/5 * * * *".into(),
            "check".into(),
            "check".into(),
            true,
            true,
            123,
        );
        job.id = "mutation_job".into();
        job.trigger_at_ms = Some(456);
        journal.put_legacy_job(&job).unwrap();
        let catalog_path = root.path().join(".refact/automations.json");
        let catalog_before = std::fs::read(&catalog_path).unwrap();
        journal.db().unwrap().execute_batch(
            "CREATE TRIGGER fail_mutation BEFORE UPDATE ON cron_jobs BEGIN SELECT RAISE(ABORT, 'injected mutation failure'); END;",
        ).unwrap();
        let error = journal
            .mutate_legacy_job(&job.id, |stored| {
                stored.fire_count = 7;
                stored.last_fired_at_ms = Some(789);
                stored.trigger_at_ms = None;
                Ok(true)
            })
            .unwrap_err();
        assert!(error.contains("injected mutation failure"));
        assert_eq!(std::fs::read(&catalog_path).unwrap(), catalog_before);
        assert_eq!(journal.load_legacy_jobs().unwrap(), vec![job.clone()]);
        journal
            .db()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_mutation")
            .unwrap();
        assert!(journal
            .mutate_legacy_job(&job.id, |stored| {
                stored.description = "x".repeat(1048576);
                Ok(true)
            })
            .is_err());
        assert_eq!(std::fs::read(&catalog_path).unwrap(), catalog_before);
        journal
            .mutate_legacy_job(&job.id, |stored| {
                stored.fire_count = 7;
                stored.trigger_at_ms = None;
                Ok(true)
            })
            .unwrap();
        assert_eq!(std::fs::read(&catalog_path).unwrap(), catalog_before);
        let catalog = super::super::AutomationCatalog::open(root.path()).unwrap();
        let mut doc = catalog.read().unwrap();
        doc.legacy_jobs[0].enabled = false;
        doc.legacy_jobs[0].description = "portable edit".into();
        assert_eq!(doc.legacy_jobs[0].trigger_at_ms, Some(456));
        catalog
            .compare_and_swap_legacy_jobs(doc.revision, doc.legacy_jobs)
            .unwrap();
        let stored = journal.load_legacy_jobs().unwrap().remove(0);
        assert!(!stored.enabled);
        assert_eq!(stored.description, "portable edit");
        assert_eq!(stored.fire_count, 7);
        assert_eq!(stored.trigger_at_ms, None);
        assert_eq!(journal.wake_summary().unwrap().recovery, 1);
        let observer = AutomationJournal::open_observer(root.path()).unwrap();
        assert_eq!(observer.load_legacy_jobs().unwrap(), vec![stored]);
    }

    #[test]
    fn epoch_changed_after_precheck_cannot_enter_mutation_transaction() {
        let root = tempfile::tempdir().unwrap();
        let journal = AutomationJournal::open(root.path()).unwrap();
        let lease = super::super::ProjectAutomationLease::acquire(root.path()).unwrap();
        journal.activate_owner(&lease).unwrap();
        assert_eq!(journal.check_owner(None).unwrap(), lease.epoch());
        let other = Connection::open(journal_path(root.path()).unwrap()).unwrap();
        other
            .execute(
                "UPDATE metadata SET value='replacement-owner' WHERE key='owner_epoch'",
                [],
            )
            .unwrap();
        let mut db = journal.db().unwrap();
        assert!(journal.transaction(&mut db).is_err());
        assert_eq!(
            other
                .query_row("SELECT COUNT(*) FROM cron_jobs", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}
