pub mod filesystem;
pub mod records;
pub mod schema;

use crate::contracts::{WorkOrigin, WorkPhase};
use crate::identity::{CheckoutKey, ProjectKey, RunKey, RuntimeKind, WorkKey};
use crate::settings::{CoordinationSettings, SettingsError};
use filesystem::FilesystemKind;
use records::{
    AccessKind, ActivityObservationRecord, DecompositionOutcomeRecord, DeliveryOutboxRecord,
    DeliveryState, EngineInstanceRecord, IntentState, LeaseAcquisition, LeaseAuthority,
    LeaseRecord, PresenceMark, PruneReport, QueuedWorkRecord, RunIntentRecord,
};
use rusqlite::{params, OptionalExtension};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio_rusqlite::Connection;

const MEMORY_PATH: &str = ":memory:";

const WAL_SWITCH_POLL: Duration = Duration::from_millis(20);

const ACTIVITY_COLUMNS: &str = "project, record_id, runtime, work_id, path, access, \
     content_hash, observed_at_ms, file_key, file_alias, instance_id, tool_call, \
     content_hash_before, checkout";

const DECOMPOSITION_COLUMNS: &str = "project, record_id, runtime, work_id, work_class, \
     fan_out_group, sibling_count, wall_clock_ms, tokens, turns, tool_calls, succeeded, \
     reworked, recorded_at_ms, paths";

static NEXT_STORE_ID: AtomicU64 = AtomicU64::new(1);

pub fn default_store_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join("coordination").join("coordination.sqlite")
}

#[derive(Debug)]
struct DecodeError(String);

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DecodeError {}

fn decode(message: String) -> tokio_rusqlite::Error {
    tokio_rusqlite::Error::Other(Box::new(DecodeError(message)))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    Io(String),
    Sqlite(String),
    Decode(String),
    Corrupt {
        path: String,
        detail: String,
    },
    SchemaTooNew {
        found: i64,
        supported: i64,
    },
    NonLocalFilesystem {
        fs_type: String,
        mount_point: String,
        path: String,
    },
    Unauthorized {
        requested: String,
        authorized: String,
    },
    ForeignAccessToken,
    NotFound {
        kind: &'static str,
        id: String,
    },
    GenerationConflict {
        expected: i64,
        found: i64,
    },
    /// The presented owner token holds no such lease, so it has no authority.
    LeaseNotHeld {
        lease_key: String,
    },
    /// The lease exists and is still the caller's, but its term ran out.
    LeaseExpired {
        lease_key: String,
        expires_at_ms: i64,
        now_ms: i64,
    },
    /// The lease moved on to a newer generation: this writer was fenced out.
    LeaseFenced {
        lease_key: String,
        presented: i64,
        held: i64,
    },
    Settings(SettingsError),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Io(detail) => write!(f, "coordination store io: {detail}"),
            StoreError::Sqlite(detail) => write!(f, "coordination store sqlite: {detail}"),
            StoreError::Decode(detail) => write!(f, "coordination store decode: {detail}"),
            StoreError::Corrupt { path, detail } => write!(
                f,
                "coordination store at {path} is unreadable and was left untouched: {detail}"
            ),
            StoreError::SchemaTooNew { found, supported } => write!(
                f,
                "coordination store schema version {found} is newer than this binary supports \
                 ({supported}); refusing to read it"
            ),
            StoreError::NonLocalFilesystem {
                fs_type,
                mount_point,
                path,
            } => write!(
                f,
                "coordination store path {path} lives on `{fs_type}` mounted at {mount_point}; \
                 SQLite WAL locking is not dependable there and this store is local-only"
            ),
            StoreError::Unauthorized {
                requested,
                authorized,
            } => write!(
                f,
                "project `{requested}` is outside the authorized project `{authorized}`"
            ),
            StoreError::ForeignAccessToken => {
                write!(
                    f,
                    "access token was issued by a different coordination store"
                )
            }
            StoreError::NotFound { kind, id } => write!(f, "{kind} `{id}` not found"),
            StoreError::GenerationConflict { expected, found } => {
                write!(f, "generation conflict: expected {expected}, found {found}")
            }
            StoreError::LeaseNotHeld { lease_key } => write!(
                f,
                "lease `{lease_key}` is not held by this owner token; it carries no authority"
            ),
            StoreError::LeaseExpired {
                lease_key,
                expires_at_ms,
                now_ms,
            } => write!(
                f,
                "lease `{lease_key}` expired at {expires_at_ms} and it is now {now_ms}; \
                 authority was lost and must be acquired again"
            ),
            StoreError::LeaseFenced {
                lease_key,
                presented,
                held,
            } => write!(
                f,
                "lease `{lease_key}` moved to generation {held}; a write stamped with \
                 generation {presented} is fenced out and was rejected"
            ),
            StoreError::Settings(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for StoreError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreWarning {
    NonLocalFilesystem {
        fs_type: String,
        mount_point: String,
        path: String,
    },
    /// The store could not be opened or written. Whatever depended on it for
    /// durability ran without it and must not be reported as persisted.
    DurabilityUnavailable { path: String, reason: String },
}

impl fmt::Display for StoreWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreWarning::NonLocalFilesystem {
                fs_type,
                mount_point,
                path,
            } => write!(
                f,
                "coordination store path {path} lives on `{fs_type}` mounted at {mount_point}; \
                 opened anyway because store_allow_non_local_filesystem is set"
            ),
            StoreWarning::DurabilityUnavailable { path, reason } => write!(
                f,
                "coordination store {path} is unavailable ({reason}); work continues \
                 without durability and will not survive a restart"
            ),
        }
    }
}

fn from_rusqlite(path: &Path, err: rusqlite::Error) -> StoreError {
    if let rusqlite::Error::SqliteFailure(failure, ref message) = err {
        if matches!(
            failure.code,
            rusqlite::ErrorCode::NotADatabase | rusqlite::ErrorCode::DatabaseCorrupt
        ) {
            return StoreError::Corrupt {
                path: path.display().to_string(),
                detail: message.clone().unwrap_or_else(|| failure.to_string()),
            };
        }
    }
    StoreError::Sqlite(err.to_string())
}

fn from_tokio(path: &Path, err: tokio_rusqlite::Error) -> StoreError {
    match err {
        tokio_rusqlite::Error::Rusqlite(inner) => from_rusqlite(path, inner),
        tokio_rusqlite::Error::Close((_, inner)) => from_rusqlite(path, inner),
        tokio_rusqlite::Error::Other(other) => match other.downcast::<DecodeError>() {
            Ok(decoded) => StoreError::Decode(decoded.0),
            Err(other) => StoreError::Sqlite(other.to_string()),
        },
        other => StoreError::Sqlite(other.to_string()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectAccess {
    store_id: u64,
    project: ProjectKey,
}

impl ProjectAccess {
    pub fn project(&self) -> &ProjectKey {
        &self.project
    }
}

enum CasOutcome {
    Updated(Box<RunIntentRecord>),
    Missing,
    Conflict(i64),
}

impl CasOutcome {
    fn landed(&self) -> bool {
        matches!(self, CasOutcome::Updated(_))
    }
}

/// Why a presented lease authority carries no weight. Checked inside the same
/// transaction as the write it guards, so a takeover can never slip between
/// the check and the write.
enum LeaseRefusal {
    NotHeld,
    Expired(i64),
    Fenced(i64),
}

enum LeaseWrite {
    Wrote(Box<LeaseRecord>),
    NotHeld,
    Expired(i64),
    Fenced(i64),
}

pub struct CoordinationStore {
    conn: Connection,
    path: PathBuf,
    store_id: u64,
    settings: CoordinationSettings,
    warnings: Vec<StoreWarning>,
}

impl fmt::Debug for CoordinationStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CoordinationStore")
            .field("path", &self.path)
            .field("store_id", &self.store_id)
            .finish()
    }
}

impl CoordinationStore {
    pub async fn open(path: &Path, settings: &CoordinationSettings) -> Result<Self, StoreError> {
        let kind = filesystem::classify_path(path);
        CoordinationStore::open_on_filesystem(path, settings, kind).await
    }

    pub async fn open_on_filesystem(
        path: &Path,
        settings: &CoordinationSettings,
        kind: FilesystemKind,
    ) -> Result<Self, StoreError> {
        settings.validate().map_err(StoreError::Settings)?;
        let mut warnings = Vec::new();
        if let FilesystemKind::NonLocal {
            fs_type,
            mount_point,
        } = &kind
        {
            if !settings.store_allow_non_local_filesystem {
                return Err(StoreError::NonLocalFilesystem {
                    fs_type: fs_type.clone(),
                    mount_point: mount_point.clone(),
                    path: path.display().to_string(),
                });
            }
            warnings.push(StoreWarning::NonLocalFilesystem {
                fs_type: fs_type.clone(),
                mount_point: mount_point.clone(),
                path: path.display().to_string(),
            });
        }
        prepare_parent_dir(path)?;
        let conn = Connection::open(path)
            .await
            .map_err(|err| from_tokio(path, err))?;
        CoordinationStore::bootstrap(conn, path.to_path_buf(), settings.clone(), warnings).await
    }

    pub async fn open_in_memory(settings: &CoordinationSettings) -> Result<Self, StoreError> {
        settings.validate().map_err(StoreError::Settings)?;
        let path = PathBuf::from(MEMORY_PATH);
        let conn = Connection::open_in_memory()
            .await
            .map_err(|err| from_tokio(&path, err))?;
        CoordinationStore::bootstrap(conn, path, settings.clone(), Vec::new()).await
    }

    async fn bootstrap(
        conn: Connection,
        path: PathBuf,
        settings: CoordinationSettings,
        warnings: Vec<StoreWarning>,
    ) -> Result<Self, StoreError> {
        let busy_timeout_ms = settings.store_busy_timeout_ms;
        let persistent = path != Path::new(MEMORY_PATH);
        conn.call(move |conn| {
            conn.busy_timeout(Duration::from_millis(busy_timeout_ms))?;
            Ok(())
        })
        .await
        .map_err(|err| from_tokio(&path, err))?;

        let version = conn
            .call(|conn| {
                let version: i64 =
                    conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
                Ok(version)
            })
            .await
            .map_err(|err| from_tokio(&path, err))?;
        if version > schema::SCHEMA_VERSION {
            return Err(StoreError::SchemaTooNew {
                found: version,
                supported: schema::SCHEMA_VERSION,
            });
        }

        conn.call(move |conn| {
            if persistent {
                enable_wal(conn, busy_timeout_ms)?;
            }
            conn.pragma_update(None, "synchronous", "NORMAL")?;
            conn.pragma_update(None, "foreign_keys", "ON")?;
            Ok(())
        })
        .await
        .map_err(|err| from_tokio(&path, err))?;

        conn.call(move |conn| {
            for (index, migration) in schema::MIGRATIONS.iter().enumerate() {
                let target = index as i64 + 1;
                let tx =
                    conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                let current: i64 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
                if current >= target {
                    tx.rollback()?;
                    continue;
                }
                tx.execute_batch(migration)?;
                tx.pragma_update(None, "user_version", target)?;
                tx.commit()?;
            }
            Ok(())
        })
        .await
        .map_err(|err| from_tokio(&path, err))?;

        Ok(CoordinationStore {
            conn,
            path,
            store_id: NEXT_STORE_ID.fetch_add(1, Ordering::Relaxed),
            settings,
            warnings,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn settings(&self) -> &CoordinationSettings {
        &self.settings
    }

    pub fn warnings(&self) -> &[StoreWarning] {
        &self.warnings
    }

    pub fn authorize(&self, project: ProjectKey) -> ProjectAccess {
        ProjectAccess {
            store_id: self.store_id,
            project,
        }
    }

    pub async fn schema_version(&self) -> Result<i64, StoreError> {
        self.conn
            .call(|conn| {
                let version: i64 =
                    conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
                Ok(version)
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))
    }

    pub async fn close(self) -> Result<(), StoreError> {
        let path = self.path.clone();
        self.conn
            .close()
            .await
            .map_err(|err| from_tokio(&path, err))
    }

    fn scope(&self, access: &ProjectAccess) -> Result<String, StoreError> {
        if access.store_id != self.store_id {
            return Err(StoreError::ForeignAccessToken);
        }
        Ok(access.project.0.clone())
    }

    fn scope_for(
        &self,
        access: &ProjectAccess,
        project: &ProjectKey,
    ) -> Result<String, StoreError> {
        let scope = self.scope(access)?;
        if project.0 != scope {
            return Err(StoreError::Unauthorized {
                requested: project.0.clone(),
                authorized: scope,
            });
        }
        Ok(scope)
    }

    pub async fn put_engine_instance(
        &self,
        access: &ProjectAccess,
        record: &EngineInstanceRecord,
    ) -> Result<(), StoreError> {
        let scope = self.scope_for(access, &record.project)?;
        let record = record.clone();
        self.conn
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO engine_instances (project, instance_id, pid, boot_id, \
                     process_started_at_ms, host_id, heartbeat_at_ms, presence_mark) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
                     ON CONFLICT(project, instance_id) DO UPDATE SET pid = excluded.pid, \
                     boot_id = excluded.boot_id, \
                     process_started_at_ms = excluded.process_started_at_ms, \
                     host_id = excluded.host_id, heartbeat_at_ms = excluded.heartbeat_at_ms, \
                     presence_mark = excluded.presence_mark",
                    params![
                        scope,
                        record.instance_id,
                        record.pid,
                        record.boot_id,
                        record.process_started_at_ms,
                        record.host_id,
                        record.heartbeat_at_ms,
                        record.presence_mark.as_str()
                    ],
                )?;
                Ok(())
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))
    }

    pub async fn get_engine_instance(
        &self,
        access: &ProjectAccess,
        instance_id: &str,
    ) -> Result<Option<EngineInstanceRecord>, StoreError> {
        let scope = self.scope(access)?;
        let instance_id = instance_id.to_string();
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT project, instance_id, pid, boot_id, process_started_at_ms, host_id, \
                     heartbeat_at_ms, presence_mark FROM engine_instances \
                     WHERE project = ?1 AND instance_id = ?2",
                )?;
                let row = stmt
                    .query_row(params![scope, instance_id], engine_instance_columns)
                    .optional()?;
                match row {
                    Some(row) => Ok(Some(decode_engine_instance(row)?)),
                    None => Ok(None),
                }
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))
    }

    pub async fn list_engine_instances(
        &self,
        access: &ProjectAccess,
    ) -> Result<Vec<EngineInstanceRecord>, StoreError> {
        let scope = self.scope(access)?;
        let limit = self.settings.store_max_engine_instances as i64;
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT project, instance_id, pid, boot_id, process_started_at_ms, host_id, \
                     heartbeat_at_ms, presence_mark FROM engine_instances WHERE project = ?1 \
                     ORDER BY heartbeat_at_ms DESC, instance_id ASC LIMIT ?2",
                )?;
                let rows = stmt.query_map(params![scope, limit], engine_instance_columns)?;
                let mut records = Vec::new();
                for row in rows {
                    records.push(decode_engine_instance(row?)?);
                }
                Ok(records)
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))
    }

    pub async fn delete_engine_instance(
        &self,
        access: &ProjectAccess,
        instance_id: &str,
    ) -> Result<bool, StoreError> {
        let scope = self.scope(access)?;
        let instance_id = instance_id.to_string();
        self.conn
            .call(move |conn| {
                let affected = conn.execute(
                    "DELETE FROM engine_instances WHERE project = ?1 AND instance_id = ?2",
                    params![scope, instance_id],
                )?;
                Ok(affected > 0)
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))
    }

    pub async fn put_run_intent(
        &self,
        access: &ProjectAccess,
        record: &RunIntentRecord,
    ) -> Result<(), StoreError> {
        let scope = self.scope_for(access, &record.project)?;
        let record = record.clone();
        self.conn
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO run_intents (project, runtime, work_id, attempt, generation, \
                     state, updated_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
                     ON CONFLICT(project, runtime, work_id, attempt) DO UPDATE SET \
                     generation = excluded.generation, state = excluded.state, \
                     updated_at_ms = excluded.updated_at_ms",
                    params![
                        scope,
                        record.run.work.runtime.as_str(),
                        record.run.work.id,
                        record.run.attempt as i64,
                        record.generation,
                        record.state.as_str(),
                        record.updated_at_ms
                    ],
                )?;
                Ok(())
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))
    }

    pub async fn get_run_intent(
        &self,
        access: &ProjectAccess,
        run: &RunKey,
    ) -> Result<Option<RunIntentRecord>, StoreError> {
        let scope = self.scope(access)?;
        let runtime = run.work.runtime.as_str();
        let work_id = run.work.id.clone();
        let attempt = run.attempt as i64;
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT project, runtime, work_id, attempt, generation, state, updated_at_ms \
                     FROM run_intents WHERE project = ?1 AND runtime = ?2 AND work_id = ?3 \
                     AND attempt = ?4",
                )?;
                let row = stmt
                    .query_row(
                        params![scope, runtime, work_id, attempt],
                        run_intent_columns,
                    )
                    .optional()?;
                match row {
                    Some(row) => Ok(Some(decode_run_intent(row)?)),
                    None => Ok(None),
                }
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))
    }

    pub async fn list_run_intents(
        &self,
        access: &ProjectAccess,
    ) -> Result<Vec<RunIntentRecord>, StoreError> {
        let scope = self.scope(access)?;
        let limit = self.settings.store_max_run_intents as i64;
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT project, runtime, work_id, attempt, generation, state, updated_at_ms \
                     FROM run_intents WHERE project = ?1 \
                     ORDER BY updated_at_ms DESC, work_id ASC, attempt ASC LIMIT ?2",
                )?;
                let rows = stmt.query_map(params![scope, limit], run_intent_columns)?;
                let mut records = Vec::new();
                for row in rows {
                    records.push(decode_run_intent(row?)?);
                }
                Ok(records)
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))
    }

    pub async fn compare_and_set_run_intent(
        &self,
        access: &ProjectAccess,
        run: &RunKey,
        expected_generation: i64,
        next_state: IntentState,
        updated_at_ms: i64,
    ) -> Result<RunIntentRecord, StoreError> {
        let scope = self.scope(access)?;
        let runtime = run.work.runtime.as_str();
        let work_id = run.work.id.clone();
        let attempt = run.attempt as i64;
        let run_label = run.to_string();
        let outcome = self
            .conn
            .call(move |conn| {
                let tx =
                    conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                let outcome = cas_run_intent(
                    &tx,
                    &scope,
                    runtime,
                    &work_id,
                    attempt,
                    expected_generation,
                    next_state,
                    updated_at_ms,
                )?;
                if outcome.landed() {
                    tx.commit()?;
                } else {
                    tx.rollback()?;
                }
                Ok(outcome)
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))?;
        cas_result(outcome, run_label, expected_generation)
    }

    /// Apply a run-intent transition only while the presented lease still has
    /// authority. A stale writer's update is rejected without touching the row.
    /// authority. A stale writer's update is rejected without touching the row.
    #[allow(clippy::too_many_arguments)]
    pub async fn compare_and_set_run_intent_fenced(
        &self,
        access: &ProjectAccess,
        authority: &LeaseAuthority,
        now_at_ms: i64,
        run: &RunKey,
        expected_generation: i64,
        next_state: IntentState,
        updated_at_ms: i64,
    ) -> Result<RunIntentRecord, StoreError> {
        let scope = self.scope(access)?;
        let runtime = run.work.runtime.as_str();
        let work_id = run.work.id.clone();
        let attempt = run.attempt as i64;
        let run_label = run.to_string();
        let lease_key = authority.lease_key.clone();
        let presented = authority.fencing_generation;
        let authority = authority.clone();
        let outcome = self
            .conn
            .call(move |conn| {
                let tx =
                    conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                let outcome = match lease_refusal(&tx, &scope, &authority, now_at_ms)? {
                    Some(refusal) => Err(refusal),
                    None => Ok(cas_run_intent(
                        &tx,
                        &scope,
                        runtime,
                        &work_id,
                        attempt,
                        expected_generation,
                        next_state,
                        updated_at_ms,
                    )?),
                };
                match &outcome {
                    Ok(cas) if cas.landed() => tx.commit()?,
                    _ => tx.rollback()?,
                }
                Ok(outcome)
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))?;
        match outcome {
            Ok(cas) => cas_result(cas, run_label, expected_generation),
            Err(refusal) => Err(lease_refused(refusal, lease_key, presented, now_at_ms)),
        }
    }

    /// Record the first launch boundary under lease authority: the intent
    /// exists and nothing has happened yet.
    ///
    /// An existing row is returned exactly as it stands. Replaying this
    /// boundary after a crash must never reset a run that already moved past
    /// it, so this inserts or reports — it never overwrites.
    pub async fn insert_run_intent_fenced(
        &self,
        access: &ProjectAccess,
        authority: &LeaseAuthority,
        now_at_ms: i64,
        run: &RunKey,
        updated_at_ms: i64,
    ) -> Result<RunIntentRecord, StoreError> {
        let scope = self.scope(access)?;
        let project = access.project().clone();
        let run_key = run.clone();
        let runtime = run.work.runtime.as_str();
        let work_id = run.work.id.clone();
        let attempt = run.attempt as i64;
        let lease_key = authority.lease_key.clone();
        let presented = authority.fencing_generation;
        let authority = authority.clone();
        let outcome = self
            .conn
            .call(move |conn| {
                let tx =
                    conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                let outcome = match lease_refusal(&tx, &scope, &authority, now_at_ms)? {
                    Some(refusal) => Err(refusal),
                    None => {
                        let held: Option<(i64, String, i64)> = tx
                            .query_row(
                                "SELECT generation, state, updated_at_ms FROM run_intents \
                                 WHERE project = ?1 AND runtime = ?2 AND work_id = ?3 \
                                 AND attempt = ?4",
                                params![scope, runtime, work_id, attempt],
                                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                            )
                            .optional()?;
                        match held {
                            Some((generation, state, seen_at_ms)) => {
                                let state = IntentState::parse(&state).ok_or_else(|| {
                                    decode(format!("unknown intent state `{state}` in run_intents"))
                                })?;
                                Ok((
                                    false,
                                    RunIntentRecord {
                                        project,
                                        run: run_key,
                                        generation,
                                        state,
                                        updated_at_ms: seen_at_ms,
                                    },
                                ))
                            }
                            None => {
                                tx.execute(
                                    "INSERT INTO run_intents (project, runtime, work_id, \
                                     attempt, generation, state, updated_at_ms) \
                                     VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6)",
                                    params![
                                        scope,
                                        runtime,
                                        work_id,
                                        attempt,
                                        IntentState::Persisted.as_str(),
                                        updated_at_ms
                                    ],
                                )?;
                                Ok((
                                    true,
                                    RunIntentRecord {
                                        project,
                                        run: run_key,
                                        generation: 0,
                                        state: IntentState::Persisted,
                                        updated_at_ms,
                                    },
                                ))
                            }
                        }
                    }
                };
                match &outcome {
                    Ok((true, _)) => tx.commit()?,
                    _ => tx.rollback()?,
                }
                Ok(outcome)
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))?;
        match outcome {
            Ok((_, record)) => Ok(record),
            Err(refusal) => Err(lease_refused(refusal, lease_key, presented, now_at_ms)),
        }
    }

    /// Delete run intents that reached a terminal outcome before
    /// `settled_before_ms`. A run whose outcome nobody recorded is never
    /// deleted by age: reconciliation has to be able to surface it.
    pub async fn prune_settled_run_intents(
        &self,
        access: &ProjectAccess,
        settled_before_ms: i64,
    ) -> Result<usize, StoreError> {
        let scope = self.scope(access)?;
        self.conn
            .call(move |conn| {
                let removed = conn.execute(
                    "DELETE FROM run_intents WHERE project = ?1 AND state = ?2 \
                     AND updated_at_ms < ?3",
                    params![scope, IntentState::Terminal.as_str(), settled_before_ms],
                )?;
                Ok(removed)
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))
    }

    pub async fn put_queued_work(
        &self,
        access: &ProjectAccess,
        record: &QueuedWorkRecord,
    ) -> Result<(), StoreError> {
        let scope = self.scope_for(access, &record.project)?;
        let record = record.clone();
        self.conn
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO queued_work (project, runtime, work_id, root, origin, phase, \
                     enqueued_at_ms, payload) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
                     ON CONFLICT(project, runtime, work_id) DO UPDATE SET root = excluded.root, \
                     origin = excluded.origin, phase = excluded.phase, \
                     enqueued_at_ms = excluded.enqueued_at_ms, payload = excluded.payload",
                    params![
                        scope,
                        record.work.runtime.as_str(),
                        record.work.id,
                        record.root.0,
                        record.origin.as_str(),
                        record.phase.as_str(),
                        record.enqueued_at_ms,
                        record.payload
                    ],
                )?;
                Ok(())
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))
    }

    pub async fn get_queued_work(
        &self,
        access: &ProjectAccess,
        work: &WorkKey,
    ) -> Result<Option<QueuedWorkRecord>, StoreError> {
        let scope = self.scope(access)?;
        let runtime = work.runtime.as_str();
        let work_id = work.id.clone();
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT project, runtime, work_id, root, origin, phase, enqueued_at_ms, \
                     payload FROM queued_work WHERE project = ?1 AND runtime = ?2 AND work_id = ?3",
                )?;
                let row = stmt
                    .query_row(params![scope, runtime, work_id], queued_work_columns)
                    .optional()?;
                match row {
                    Some(row) => Ok(Some(decode_queued_work(row)?)),
                    None => Ok(None),
                }
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))
    }

    pub async fn list_queued_work(
        &self,
        access: &ProjectAccess,
    ) -> Result<Vec<QueuedWorkRecord>, StoreError> {
        let scope = self.scope(access)?;
        let limit = self.settings.store_max_queued_work as i64;
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT project, runtime, work_id, root, origin, phase, enqueued_at_ms, \
                     payload FROM queued_work WHERE project = ?1 \
                     ORDER BY enqueued_at_ms ASC, work_id ASC LIMIT ?2",
                )?;
                let rows = stmt.query_map(params![scope, limit], queued_work_columns)?;
                let mut records = Vec::new();
                for row in rows {
                    records.push(decode_queued_work(row?)?);
                }
                Ok(records)
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))
    }

    pub async fn delete_queued_work(
        &self,
        access: &ProjectAccess,
        work: &WorkKey,
    ) -> Result<bool, StoreError> {
        let scope = self.scope(access)?;
        let runtime = work.runtime.as_str();
        let work_id = work.id.clone();
        self.conn
            .call(move |conn| {
                let affected = conn.execute(
                    "DELETE FROM queued_work WHERE project = ?1 AND runtime = ?2 AND work_id = ?3",
                    params![scope, runtime, work_id],
                )?;
                Ok(affected > 0)
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))
    }

    pub async fn put_lease(
        &self,
        access: &ProjectAccess,
        record: &LeaseRecord,
    ) -> Result<(), StoreError> {
        let scope = self.scope_for(access, &record.project)?;
        let record = record.clone();
        self.conn
            .call(move |conn| {
                conn.execute(LEASE_UPSERT_SQL, lease_params(&scope, &record))?;
                Ok(())
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))
    }

    /// Acquire `lease_key` atomically: either this owner token holds it with a
    /// definite generation afterwards, or someone else's row is reported back
    /// untouched. An expired row belonging to another owner is *not* taken —
    /// that requires [`CoordinationStore::take_over_lease`].
    pub async fn try_acquire_lease(
        &self,
        access: &ProjectAccess,
        lease_key: &str,
        holder_instance: &str,
        owner_token: &str,
        now_at_ms: i64,
        expires_at_ms: i64,
    ) -> Result<LeaseAcquisition, StoreError> {
        let scope = self.scope(access)?;
        let candidate = LeaseRecord {
            project: access.project().clone(),
            lease_key: lease_key.to_string(),
            holder_instance: holder_instance.to_string(),
            owner_token: owner_token.to_string(),
            fencing_generation: 1,
            acquired_at_ms: now_at_ms,
            expires_at_ms,
        };
        self.conn
            .call(move |conn| {
                let tx =
                    conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                let held = tx
                    .query_row(
                        &lease_select("WHERE project = ?1 AND lease_key = ?2"),
                        params![scope, candidate.lease_key],
                        lease_from_row,
                    )
                    .optional()?;
                let acquired = match held {
                    Some(held) if held.owner_token != candidate.owner_token => {
                        tx.rollback()?;
                        return Ok(LeaseAcquisition::Held(held));
                    }
                    // The same owner re-acquiring after expiry starts a new term
                    // of authority, so it is fenced like any other change.
                    Some(held) => LeaseRecord {
                        fencing_generation: held.fencing_generation
                            + i64::from(held.is_expired(candidate.acquired_at_ms)),
                        acquired_at_ms: if held.is_expired(candidate.acquired_at_ms) {
                            candidate.acquired_at_ms
                        } else {
                            held.acquired_at_ms
                        },
                        ..candidate
                    },
                    None => candidate,
                };
                tx.execute(LEASE_UPSERT_SQL, lease_params(&scope, &acquired))?;
                tx.commit()?;
                Ok(LeaseAcquisition::Acquired(acquired))
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))
    }

    /// Extend the term of a lease this owner still holds. The generation is
    /// deliberately untouched: renewal fences nobody.
    pub async fn renew_lease(
        &self,
        access: &ProjectAccess,
        authority: &LeaseAuthority,
        now_at_ms: i64,
        expires_at_ms: i64,
    ) -> Result<LeaseRecord, StoreError> {
        self.write_lease_row(
            access,
            &authority.lease_key,
            authority.fencing_generation,
            Some(&authority.owner_token),
            Some(now_at_ms),
            move |held| LeaseRecord {
                expires_at_ms,
                ..held
            },
        )
        .await
    }

    /// Take `lease_key` from the holder that `expected_generation` describes.
    /// The generation is bumped, which is what fences the previous holder out.
    pub async fn take_over_lease(
        &self,
        access: &ProjectAccess,
        lease_key: &str,
        expected_generation: i64,
        holder_instance: &str,
        owner_token: &str,
        now_at_ms: i64,
        expires_at_ms: i64,
    ) -> Result<LeaseRecord, StoreError> {
        let holder_instance = holder_instance.to_string();
        let owner_token = owner_token.to_string();
        self.write_lease_row(
            access,
            lease_key,
            expected_generation,
            None,
            None,
            move |held| LeaseRecord {
                holder_instance,
                owner_token,
                fencing_generation: held.fencing_generation + 1,
                acquired_at_ms: now_at_ms,
                expires_at_ms,
                ..held
            },
        )
        .await
    }

    /// Release a lease this owner still holds at this generation. A fenced-out
    /// holder releases nothing: the `false` says its row was already gone.
    pub async fn release_lease(
        &self,
        access: &ProjectAccess,
        authority: &LeaseAuthority,
    ) -> Result<bool, StoreError> {
        let scope = self.scope(access)?;
        let authority = authority.clone();
        self.conn
            .call(move |conn| {
                let affected = conn.execute(
                    "DELETE FROM leases WHERE project = ?1 AND lease_key = ?2 \
                     AND owner_token = ?3 AND fencing_generation = ?4",
                    params![
                        scope,
                        authority.lease_key,
                        authority.owner_token,
                        authority.fencing_generation
                    ],
                )?;
                Ok(affected > 0)
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))
    }

    /// One transaction: check the stored row against the presented generation,
    /// then rewrite it. `expected_owner` additionally demands that this owner
    /// token still holds the row, and `unexpired_at_ms` that its term is live.
    async fn write_lease_row<F>(
        &self,
        access: &ProjectAccess,
        lease_key: &str,
        expected_generation: i64,
        expected_owner: Option<&str>,
        unexpired_at_ms: Option<i64>,
        rewrite: F,
    ) -> Result<LeaseRecord, StoreError>
    where
        F: FnOnce(LeaseRecord) -> LeaseRecord + Send + 'static,
    {
        let scope = self.scope(access)?;
        let lease_key = lease_key.to_string();
        let key = lease_key.clone();
        let expected_owner = expected_owner.map(str::to_string);
        let outcome = self
            .conn
            .call(move |conn| {
                let tx =
                    conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                let held = tx
                    .query_row(
                        &lease_select("WHERE project = ?1 AND lease_key = ?2"),
                        params![scope, key],
                        lease_from_row,
                    )
                    .optional()?;
                let outcome = match held {
                    None => LeaseWrite::NotHeld,
                    Some(held) if held.fencing_generation != expected_generation => {
                        LeaseWrite::Fenced(held.fencing_generation)
                    }
                    Some(held)
                        if expected_owner.is_some_and(|expected| held.owner_token != expected) =>
                    {
                        LeaseWrite::NotHeld
                    }
                    Some(held) if unexpired_at_ms.is_some_and(|at_ms| held.is_expired(at_ms)) => {
                        LeaseWrite::Expired(held.expires_at_ms)
                    }
                    Some(held) => {
                        let written = rewrite(held);
                        tx.execute(LEASE_UPSERT_SQL, lease_params(&scope, &written))?;
                        LeaseWrite::Wrote(Box::new(written))
                    }
                };
                match &outcome {
                    LeaseWrite::Wrote(_) => tx.commit()?,
                    _ => tx.rollback()?,
                }
                Ok(outcome)
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))?;
        match outcome {
            LeaseWrite::Wrote(record) => Ok(*record),
            LeaseWrite::NotHeld => Err(StoreError::LeaseNotHeld { lease_key }),
            LeaseWrite::Expired(expires_at_ms) => Err(StoreError::LeaseExpired {
                lease_key,
                expires_at_ms,
                now_ms: unexpired_at_ms.unwrap_or_default(),
            }),
            LeaseWrite::Fenced(held) => Err(StoreError::LeaseFenced {
                lease_key,
                presented: expected_generation,
                held,
            }),
        }
    }

    pub async fn get_lease(
        &self,
        access: &ProjectAccess,
        lease_key: &str,
    ) -> Result<Option<LeaseRecord>, StoreError> {
        let scope = self.scope(access)?;
        let lease_key = lease_key.to_string();
        self.conn
            .call(move |conn| {
                let mut stmt =
                    conn.prepare(&lease_select("WHERE project = ?1 AND lease_key = ?2"))?;
                let record = stmt
                    .query_row(params![scope, lease_key], lease_from_row)
                    .optional()?;
                Ok(record)
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))
    }

    pub async fn list_leases(
        &self,
        access: &ProjectAccess,
    ) -> Result<Vec<LeaseRecord>, StoreError> {
        let scope = self.scope(access)?;
        let limit = self.settings.store_max_leases as i64;
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(&lease_select(
                    "WHERE project = ?1 ORDER BY expires_at_ms ASC, lease_key ASC LIMIT ?2",
                ))?;
                let rows = stmt.query_map(params![scope, limit], lease_from_row)?;
                let mut records = Vec::new();
                for row in rows {
                    records.push(row?);
                }
                Ok(records)
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))
    }

    pub async fn delete_lease(
        &self,
        access: &ProjectAccess,
        lease_key: &str,
    ) -> Result<bool, StoreError> {
        let scope = self.scope(access)?;
        let lease_key = lease_key.to_string();
        self.conn
            .call(move |conn| {
                let affected = conn.execute(
                    "DELETE FROM leases WHERE project = ?1 AND lease_key = ?2",
                    params![scope, lease_key],
                )?;
                Ok(affected > 0)
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))
    }

    pub async fn put_outbox_message(
        &self,
        access: &ProjectAccess,
        record: &DeliveryOutboxRecord,
    ) -> Result<(), StoreError> {
        let scope = self.scope_for(access, &record.project)?;
        let record = record.clone();
        self.conn
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO delivery_outbox (project, message_id, recipient, state, \
                     attempts, created_at_ms, updated_at_ms, payload) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
                     ON CONFLICT(project, message_id) DO UPDATE SET \
                     recipient = excluded.recipient, state = excluded.state, \
                     attempts = excluded.attempts, updated_at_ms = excluded.updated_at_ms, \
                     payload = excluded.payload",
                    params![
                        scope,
                        record.message_id,
                        record.recipient,
                        record.state.as_str(),
                        record.attempts,
                        record.created_at_ms,
                        record.updated_at_ms,
                        record.payload
                    ],
                )?;
                Ok(())
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))
    }

    pub async fn get_outbox_message(
        &self,
        access: &ProjectAccess,
        message_id: &str,
    ) -> Result<Option<DeliveryOutboxRecord>, StoreError> {
        let scope = self.scope(access)?;
        let message_id = message_id.to_string();
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT project, message_id, recipient, state, attempts, created_at_ms, \
                     updated_at_ms, payload FROM delivery_outbox \
                     WHERE project = ?1 AND message_id = ?2",
                )?;
                let row = stmt
                    .query_row(params![scope, message_id], outbox_columns)
                    .optional()?;
                match row {
                    Some(row) => Ok(Some(decode_outbox(row)?)),
                    None => Ok(None),
                }
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))
    }

    pub async fn list_outbox_messages(
        &self,
        access: &ProjectAccess,
    ) -> Result<Vec<DeliveryOutboxRecord>, StoreError> {
        let scope = self.scope(access)?;
        let limit = self.settings.store_max_outbox_messages as i64;
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT project, message_id, recipient, state, attempts, created_at_ms, \
                     updated_at_ms, payload FROM delivery_outbox WHERE project = ?1 \
                     ORDER BY created_at_ms ASC, message_id ASC LIMIT ?2",
                )?;
                let rows = stmt.query_map(params![scope, limit], outbox_columns)?;
                let mut records = Vec::new();
                for row in rows {
                    records.push(decode_outbox(row?)?);
                }
                Ok(records)
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))
    }

    pub async fn list_outbox_messages_in_states(
        &self,
        access: &ProjectAccess,
        states: &[DeliveryState],
    ) -> Result<Vec<DeliveryOutboxRecord>, StoreError> {
        let scope = self.scope(access)?;
        let limit = self.settings.store_max_outbox_messages as i64;
        if states.is_empty() {
            return Ok(Vec::new());
        }
        let wanted = sql_state_list(states);
        self.conn
            .call(move |conn| {
                let sql = format!(
                    "SELECT project, message_id, recipient, state, attempts, created_at_ms, \
                     updated_at_ms, payload FROM delivery_outbox \
                     WHERE project = ?1 AND state IN ({wanted}) \
                     ORDER BY created_at_ms ASC, message_id ASC LIMIT ?2"
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params![scope, limit], outbox_columns)?;
                let mut records = Vec::new();
                for row in rows {
                    records.push(decode_outbox(row?)?);
                }
                Ok(records)
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))
    }

    /// Delete settled outbox rows updated before `settled_before_ms`. A message
    /// whose outcome is still unknown is never deleted, so recovery can always
    /// surface it for inspection.
    pub async fn prune_settled_outbox_messages(
        &self,
        access: &ProjectAccess,
        settled_before_ms: i64,
    ) -> Result<usize, StoreError> {
        let scope = self.scope(access)?;
        let terminal = sql_terminal_delivery_states();
        self.conn
            .call(move |conn| {
                let sql = format!(
                    "DELETE FROM delivery_outbox WHERE project = ?1 \
                     AND state IN ({terminal}) AND updated_at_ms < ?2"
                );
                let removed = conn.execute(&sql, params![scope, settled_before_ms])?;
                Ok(removed)
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))
    }

    pub async fn delete_outbox_message(
        &self,
        access: &ProjectAccess,
        message_id: &str,
    ) -> Result<bool, StoreError> {
        let scope = self.scope(access)?;
        let message_id = message_id.to_string();
        self.conn
            .call(move |conn| {
                let affected = conn.execute(
                    "DELETE FROM delivery_outbox WHERE project = ?1 AND message_id = ?2",
                    params![scope, message_id],
                )?;
                Ok(affected > 0)
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))
    }

    pub async fn put_activity_observation(
        &self,
        access: &ProjectAccess,
        record: &ActivityObservationRecord,
    ) -> Result<(), StoreError> {
        let scope = self.scope_for(access, &record.project)?;
        let record = record.clone();
        self.conn
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO activity_observations (project, record_id, runtime, work_id, \
                     path, access, content_hash, observed_at_ms, file_key, file_alias, \
                     instance_id, tool_call, content_hash_before, checkout) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14) \
                     ON CONFLICT(project, record_id) DO UPDATE SET runtime = excluded.runtime, \
                     work_id = excluded.work_id, path = excluded.path, \
                     access = excluded.access, content_hash = excluded.content_hash, \
                     observed_at_ms = excluded.observed_at_ms, file_key = excluded.file_key, \
                     file_alias = excluded.file_alias, instance_id = excluded.instance_id, \
                     tool_call = excluded.tool_call, \
                     content_hash_before = excluded.content_hash_before, \
                     checkout = excluded.checkout",
                    params![
                        scope,
                        record.record_id,
                        record.work.runtime.as_str(),
                        record.work.id,
                        record.path,
                        record.access.as_str(),
                        record.content_hash,
                        record.observed_at_ms,
                        record.file_key,
                        record.file_alias,
                        record.instance_id,
                        record.tool_call,
                        record.content_hash_before,
                        record.checkout
                    ],
                )?;
                Ok(())
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))
    }

    pub async fn get_activity_observation(
        &self,
        access: &ProjectAccess,
        record_id: &str,
    ) -> Result<Option<ActivityObservationRecord>, StoreError> {
        let scope = self.scope(access)?;
        let record_id = record_id.to_string();
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {ACTIVITY_COLUMNS} FROM activity_observations \
                     WHERE project = ?1 AND record_id = ?2"
                ))?;
                let row = stmt
                    .query_row(params![scope, record_id], activity_columns)
                    .optional()?;
                match row {
                    Some(row) => Ok(Some(decode_activity(row)?)),
                    None => Ok(None),
                }
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))
    }

    pub async fn list_activity_observations(
        &self,
        access: &ProjectAccess,
    ) -> Result<Vec<ActivityObservationRecord>, StoreError> {
        let scope = self.scope(access)?;
        let limit = self.settings.store_max_activity_observations as i64;
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {ACTIVITY_COLUMNS} FROM activity_observations WHERE project = ?1 \
                     ORDER BY observed_at_ms DESC, record_id ASC LIMIT ?2"
                ))?;
                let rows = stmt.query_map(params![scope, limit], activity_columns)?;
                let mut records = Vec::new();
                for row in rows {
                    records.push(decode_activity(row?)?);
                }
                Ok(records)
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))
    }

    /// The most recent observation of one logical file, newest first with
    /// `record_id` breaking a timestamp tie so the answer is deterministic.
    /// `None` means nothing was recorded — never a guess at who wrote it.
    pub async fn last_activity_for_file(
        &self,
        access: &ProjectAccess,
        file_key: &str,
        kind: AccessKind,
        since_ms: i64,
        excluding_work_id: Option<&str>,
    ) -> Result<Option<ActivityObservationRecord>, StoreError> {
        let scope = self.scope(access)?;
        let file_key = file_key.to_string();
        let excluding_work_id = excluding_work_id.map(str::to_string);
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {ACTIVITY_COLUMNS} FROM activity_observations \
                     WHERE project = ?1 AND file_key = ?2 AND access = ?3 AND observed_at_ms >= ?4 \
                     AND (?5 IS NULL OR work_id <> ?5) \
                     ORDER BY observed_at_ms DESC, record_id DESC LIMIT 1"
                ))?;
                let row = stmt
                    .query_row(
                        params![scope, file_key, kind.as_str(), since_ms, excluding_work_id],
                        activity_columns,
                    )
                    .optional()?;
                match row {
                    Some(row) => Ok(Some(decode_activity(row)?)),
                    None => Ok(None),
                }
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))
    }

    pub async fn delete_activity_observation(
        &self,
        access: &ProjectAccess,
        record_id: &str,
    ) -> Result<bool, StoreError> {
        let scope = self.scope(access)?;
        let record_id = record_id.to_string();
        self.conn
            .call(move |conn| {
                let affected = conn.execute(
                    "DELETE FROM activity_observations WHERE project = ?1 AND record_id = ?2",
                    params![scope, record_id],
                )?;
                Ok(affected > 0)
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))
    }

    /// Deletion control: drop every observation of one logical file, or of the
    /// whole authorized project when `file_key` is `None`.
    pub async fn delete_activity_observations(
        &self,
        access: &ProjectAccess,
        file_key: Option<&str>,
    ) -> Result<usize, StoreError> {
        let scope = self.scope(access)?;
        let file_key = file_key.map(str::to_string);
        self.conn
            .call(move |conn| {
                let removed = match file_key {
                    Some(file_key) => conn.execute(
                        "DELETE FROM activity_observations WHERE project = ?1 AND file_key = ?2",
                        params![scope, file_key],
                    )?,
                    None => conn.execute(
                        "DELETE FROM activity_observations WHERE project = ?1",
                        params![scope],
                    )?,
                };
                Ok(removed)
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))
    }

    /// Keep the newest `max_records` observations of this project and delete
    /// the rest. The order is total — `observed_at_ms` then `record_id` — so
    /// the same overflow always evicts the same rows.
    pub async fn prune_activity_observations(
        &self,
        access: &ProjectAccess,
        max_records: usize,
    ) -> Result<usize, StoreError> {
        let scope = self.scope(access)?;
        let limit = max_records as i64;
        self.conn
            .call(move |conn| {
                let removed = conn.execute(
                    "DELETE FROM activity_observations WHERE project = ?1 AND record_id IN \
                     (SELECT record_id FROM activity_observations WHERE project = ?1 \
                     ORDER BY observed_at_ms DESC, record_id DESC LIMIT -1 OFFSET ?2)",
                    params![scope, limit],
                )?;
                Ok(removed)
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))
    }

    /// Record what one finished run cost. Replaying the same `record_id`
    /// overwrites it, so a run can never be counted twice.
    pub async fn put_decomposition_outcome(
        &self,
        access: &ProjectAccess,
        record: &DecompositionOutcomeRecord,
    ) -> Result<(), StoreError> {
        let scope = self.scope_for(access, &record.project)?;
        let record = record.clone();
        let paths = serde_json::to_string(&record.paths)
            .map_err(|err| StoreError::Decode(err.to_string()))?;
        self.conn
            .call(move |conn| {
                conn.execute(
                    "INSERT INTO decomposition_outcomes (project, record_id, runtime, work_id, \
                     work_class, fan_out_group, sibling_count, wall_clock_ms, tokens, turns, \
                     tool_calls, succeeded, reworked, recorded_at_ms, paths) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15) \
                     ON CONFLICT(project, record_id) DO UPDATE SET runtime = excluded.runtime, \
                     work_id = excluded.work_id, work_class = excluded.work_class, \
                     fan_out_group = excluded.fan_out_group, \
                     sibling_count = excluded.sibling_count, \
                     wall_clock_ms = excluded.wall_clock_ms, tokens = excluded.tokens, \
                     turns = excluded.turns, tool_calls = excluded.tool_calls, \
                     succeeded = excluded.succeeded, reworked = excluded.reworked, \
                     recorded_at_ms = excluded.recorded_at_ms, paths = excluded.paths",
                    params![
                        scope,
                        record.record_id,
                        record.work.runtime.as_str(),
                        record.work.id,
                        record.work_class,
                        record.fan_out_group,
                        record.sibling_count,
                        record.wall_clock_ms,
                        record.tokens,
                        record.turns,
                        record.tool_calls,
                        record.succeeded as i64,
                        record.reworked as i64,
                        record.recorded_at_ms,
                        paths
                    ],
                )?;
                Ok(())
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))
    }

    /// Recorded outcomes of this project, newest first and bounded by
    /// `store_max_decomposition_outcomes`.
    pub async fn list_decomposition_outcomes(
        &self,
        access: &ProjectAccess,
        work_class: Option<&str>,
    ) -> Result<Vec<DecompositionOutcomeRecord>, StoreError> {
        let scope = self.scope(access)?;
        let limit = self.settings.store_max_decomposition_outcomes as i64;
        let work_class = work_class.map(str::to_string);
        self.conn
            .call(move |conn| {
                let mut stmt = conn.prepare(&format!(
                    "SELECT {DECOMPOSITION_COLUMNS} FROM decomposition_outcomes \
                     WHERE project = ?1 AND (?2 IS NULL OR work_class = ?2) \
                     ORDER BY recorded_at_ms DESC, record_id DESC LIMIT ?3"
                ))?;
                let rows =
                    stmt.query_map(params![scope, work_class, limit], decomposition_columns)?;
                let mut records = Vec::new();
                for row in rows {
                    records.push(decode_decomposition(row?)?);
                }
                Ok(records)
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))
    }

    /// Keep the newest `max_records` outcomes and delete the rest. The order is
    /// total, so the same overflow always evicts the same rows.
    pub async fn prune_decomposition_outcomes(
        &self,
        access: &ProjectAccess,
        max_records: usize,
    ) -> Result<usize, StoreError> {
        let scope = self.scope(access)?;
        let limit = max_records as i64;
        self.conn
            .call(move |conn| {
                let removed = conn.execute(
                    "DELETE FROM decomposition_outcomes WHERE project = ?1 AND record_id IN \
                     (SELECT record_id FROM decomposition_outcomes WHERE project = ?1 \
                     ORDER BY recorded_at_ms DESC, record_id DESC LIMIT -1 OFFSET ?2)",
                    params![scope, limit],
                )?;
                Ok(removed)
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))
    }

    pub async fn prune(&self, access: &ProjectAccess) -> Result<PruneReport, StoreError> {
        let scope = self.scope(access)?;
        let settings = self.settings.clone();
        self.conn
            .call(move |conn| {
                let settled = format!("AND state IN ({})", sql_terminal_delivery_states());
                let specs: [(&str, &[&str], &str, i64, &str); 7] = [
                    (
                        "engine_instances",
                        &["instance_id"],
                        "heartbeat_at_ms",
                        settings.store_max_engine_instances as i64,
                        "",
                    ),
                    (
                        "run_intents",
                        &["runtime", "work_id", "attempt"],
                        "updated_at_ms",
                        settings.store_max_run_intents as i64,
                        "",
                    ),
                    (
                        "queued_work",
                        &["runtime", "work_id"],
                        "enqueued_at_ms",
                        settings.store_max_queued_work as i64,
                        "",
                    ),
                    (
                        "leases",
                        &["lease_key"],
                        "expires_at_ms",
                        settings.store_max_leases as i64,
                        "",
                    ),
                    (
                        "delivery_outbox",
                        &["message_id"],
                        "updated_at_ms",
                        settings.store_max_outbox_messages as i64,
                        settled.as_str(),
                    ),
                    (
                        "activity_observations",
                        &["record_id"],
                        "observed_at_ms",
                        settings.store_max_activity_observations as i64,
                        "",
                    ),
                    (
                        "decomposition_outcomes",
                        &["record_id"],
                        "recorded_at_ms",
                        settings.store_max_decomposition_outcomes as i64,
                        "",
                    ),
                ];
                let mut report = PruneReport::default();
                let tx =
                    conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                for (table, keys, order, limit, filter) in specs {
                    let key_list = keys.join(", ");
                    let target = if keys.len() == 1 {
                        key_list.clone()
                    } else {
                        format!("({key_list})")
                    };
                    let sql = format!(
                        "DELETE FROM {table} WHERE project = ?1 {filter} AND {target} IN \
                         (SELECT {key_list} FROM {table} WHERE project = ?1 {filter} \
                         ORDER BY {order} DESC LIMIT -1 OFFSET ?2)"
                    );
                    let removed = tx.execute(&sql, params![scope, limit])?;
                    match table {
                        "engine_instances" => report.engine_instances = removed,
                        "run_intents" => report.run_intents = removed,
                        "queued_work" => report.queued_work = removed,
                        "leases" => report.leases = removed,
                        "delivery_outbox" => report.delivery_outbox = removed,
                        "activity_observations" => report.activity_observations = removed,
                        _ => report.decomposition_outcomes = removed,
                    }
                }
                tx.commit()?;
                Ok(report)
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))
    }
}

fn sql_state_list(states: &[DeliveryState]) -> String {
    states
        .iter()
        .map(|state| format!("'{}'", state.as_str()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn sql_terminal_delivery_states() -> String {
    let terminal = DeliveryState::ALL
        .into_iter()
        .filter(DeliveryState::is_terminal)
        .collect::<Vec<_>>();
    sql_state_list(&terminal)
}

/// Two peers creating the same database attempt the journal switch in
/// lockstep, and a fixed wait keeps them in it: each fails on the other's lock,
/// both wait the same span, and neither ever wins. Decorrelating the waits is
/// what breaks the symmetry.
fn wal_switch_wait() -> Duration {
    let jitter = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| u64::from(since.subsec_nanos()))
        .unwrap_or_default()
        % WAL_SWITCH_POLL.as_millis() as u64;
    WAL_SWITCH_POLL + Duration::from_millis(jitter)
}

/// Puts the database in WAL, waiting out a peer that is still creating it.
///
/// Two things SQLite will not do for us here. It does not run the busy handler
/// for a journal-mode change, so the configured busy timeout has to be honoured
/// explicitly — without it a second engine instance fails its whole `open` on a
/// lock that clears in milliseconds and loses durability for its entire life.
/// And it takes the exclusive lock before noticing the mode is already the one
/// being asked for, so the mode is read first: an open that has nothing to
/// change must never contend with a peer, which is every open but the first.
fn enable_wal(conn: &rusqlite::Connection, busy_timeout_ms: u64) -> Result<(), rusqlite::Error> {
    let deadline = Instant::now() + Duration::from_millis(busy_timeout_ms);
    loop {
        let mode: String = conn.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
        if mode.eq_ignore_ascii_case("wal") {
            return Ok(());
        }
        let attempt = conn.query_row("PRAGMA journal_mode=WAL", [], |row| row.get::<_, String>(0));
        let code = attempt
            .as_ref()
            .err()
            .and_then(rusqlite::Error::sqlite_error_code);
        match code {
            Some(rusqlite::ErrorCode::DatabaseBusy) | Some(rusqlite::ErrorCode::DatabaseLocked)
                if Instant::now() < deadline => {}
            _ => return attempt.map(|_| ()),
        }
        std::thread::sleep(wal_switch_wait());
    }
}

fn prepare_parent_dir(path: &Path) -> Result<(), StoreError> {
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => return Ok(()),
    };
    match std::fs::create_dir_all(parent) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(err) => {
            return Err(StoreError::Io(format!(
                "create coordination store dir {parent:?}: {err}"
            )))
        }
    }
    restrict_dir_permissions(parent)
}

#[cfg(unix)]
fn restrict_dir_permissions(dir: &Path) -> Result<(), StoreError> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
        .map_err(|err| StoreError::Io(format!("secure coordination store dir {dir:?}: {err}")))
}

#[cfg(not(unix))]
fn restrict_dir_permissions(_dir: &Path) -> Result<(), StoreError> {
    Ok(())
}

type EngineInstanceColumns = (String, String, i64, String, i64, String, i64, String);

fn engine_instance_columns(row: &rusqlite::Row<'_>) -> rusqlite::Result<EngineInstanceColumns> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
    ))
}

fn decode_engine_instance(
    columns: EngineInstanceColumns,
) -> Result<EngineInstanceRecord, tokio_rusqlite::Error> {
    let (
        project,
        instance_id,
        pid,
        boot_id,
        process_started_at_ms,
        host_id,
        heartbeat_at_ms,
        presence_mark,
    ) = columns;
    let presence_mark = PresenceMark::parse(&presence_mark).ok_or_else(|| {
        decode(format!(
            "unknown presence mark `{presence_mark}` in engine_instances"
        ))
    })?;
    Ok(EngineInstanceRecord {
        project: ProjectKey::new(project),
        instance_id,
        pid,
        boot_id,
        process_started_at_ms,
        host_id,
        heartbeat_at_ms,
        presence_mark,
    })
}

const LEASE_COLUMNS: &str = "project, lease_key, holder_instance, owner_token, \
                             fencing_generation, acquired_at_ms, expires_at_ms";

const LEASE_UPSERT_SQL: &str = "INSERT INTO leases (project, lease_key, holder_instance, \
     owner_token, fencing_generation, acquired_at_ms, expires_at_ms) \
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
     ON CONFLICT(project, lease_key) DO UPDATE SET \
     holder_instance = excluded.holder_instance, owner_token = excluded.owner_token, \
     fencing_generation = excluded.fencing_generation, \
     acquired_at_ms = excluded.acquired_at_ms, expires_at_ms = excluded.expires_at_ms";

fn lease_select(tail: &str) -> String {
    format!("SELECT {LEASE_COLUMNS} FROM leases {tail}")
}

fn lease_params<'a>(
    scope: &'a dyn rusqlite::ToSql,
    record: &'a LeaseRecord,
) -> [&'a dyn rusqlite::ToSql; 7] {
    [
        scope,
        &record.lease_key,
        &record.holder_instance,
        &record.owner_token,
        &record.fencing_generation,
        &record.acquired_at_ms,
        &record.expires_at_ms,
    ]
}

/// Read the lease row a write claims authority from, in the caller's own
/// transaction. `None` means the presented authority still holds.
fn lease_refusal(
    tx: &rusqlite::Transaction<'_>,
    scope: &str,
    authority: &LeaseAuthority,
    now_at_ms: i64,
) -> Result<Option<LeaseRefusal>, tokio_rusqlite::Error> {
    let held: Option<(String, i64, i64)> = tx
        .query_row(
            "SELECT owner_token, fencing_generation, expires_at_ms FROM leases \
             WHERE project = ?1 AND lease_key = ?2",
            params![scope, authority.lease_key],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let Some((owner_token, generation, expires_at_ms)) = held else {
        return Ok(Some(LeaseRefusal::NotHeld));
    };
    Ok(if generation != authority.fencing_generation {
        Some(LeaseRefusal::Fenced(generation))
    } else if owner_token != authority.owner_token {
        Some(LeaseRefusal::NotHeld)
    } else if now_at_ms >= expires_at_ms {
        Some(LeaseRefusal::Expired(expires_at_ms))
    } else {
        None
    })
}

fn lease_refused(
    refusal: LeaseRefusal,
    lease_key: String,
    presented: i64,
    now_ms: i64,
) -> StoreError {
    match refusal {
        LeaseRefusal::NotHeld => StoreError::LeaseNotHeld { lease_key },
        LeaseRefusal::Expired(expires_at_ms) => StoreError::LeaseExpired {
            lease_key,
            expires_at_ms,
            now_ms,
        },
        LeaseRefusal::Fenced(held) => StoreError::LeaseFenced {
            lease_key,
            presented,
            held,
        },
    }
}

fn lease_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LeaseRecord> {
    Ok(LeaseRecord {
        project: ProjectKey::new(row.get::<_, String>(0)?),
        lease_key: row.get(1)?,
        holder_instance: row.get(2)?,
        owner_token: row.get(3)?,
        fencing_generation: row.get(4)?,
        acquired_at_ms: row.get(5)?,
        expires_at_ms: row.get(6)?,
    })
}

#[allow(clippy::too_many_arguments)]
fn cas_run_intent(
    tx: &rusqlite::Transaction<'_>,
    scope: &str,
    runtime: &str,
    work_id: &str,
    attempt: i64,
    expected_generation: i64,
    next_state: IntentState,
    updated_at_ms: i64,
) -> Result<CasOutcome, tokio_rusqlite::Error> {
    let current: Option<i64> = tx
        .query_row(
            "SELECT generation FROM run_intents WHERE project = ?1 AND runtime = ?2 \
             AND work_id = ?3 AND attempt = ?4",
            params![scope, runtime, work_id, attempt],
            |row| row.get(0),
        )
        .optional()?;
    let Some(generation) = current else {
        return Ok(CasOutcome::Missing);
    };
    if generation != expected_generation {
        return Ok(CasOutcome::Conflict(generation));
    }
    let next_generation = generation + 1;
    tx.execute(
        "UPDATE run_intents SET generation = ?5, state = ?6, updated_at_ms = ?7 \
         WHERE project = ?1 AND runtime = ?2 AND work_id = ?3 AND attempt = ?4",
        params![
            scope,
            runtime,
            work_id,
            attempt,
            next_generation,
            next_state.as_str(),
            updated_at_ms
        ],
    )?;
    let runtime = RuntimeKind::parse(runtime)
        .ok_or_else(|| decode(format!("unknown runtime kind `{runtime}`")))?;
    Ok(CasOutcome::Updated(Box::new(RunIntentRecord {
        project: ProjectKey::new(scope.to_string()),
        run: RunKey::new(WorkKey::new(runtime, work_id.to_string()), attempt as u64),
        generation: next_generation,
        state: next_state,
        updated_at_ms,
    })))
}

fn cas_result(
    outcome: CasOutcome,
    run_label: String,
    expected_generation: i64,
) -> Result<RunIntentRecord, StoreError> {
    match outcome {
        CasOutcome::Updated(record) => Ok(*record),
        CasOutcome::Missing => Err(StoreError::NotFound {
            kind: "run intent",
            id: run_label,
        }),
        CasOutcome::Conflict(found) => Err(StoreError::GenerationConflict {
            expected: expected_generation,
            found,
        }),
    }
}

type RunIntentColumns = (String, String, String, i64, i64, String, i64);

fn run_intent_columns(row: &rusqlite::Row<'_>) -> rusqlite::Result<RunIntentColumns> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
    ))
}

fn decode_run_intent(columns: RunIntentColumns) -> Result<RunIntentRecord, tokio_rusqlite::Error> {
    let (project, runtime, work_id, attempt, generation, state, updated_at_ms) = columns;
    let runtime = RuntimeKind::parse(&runtime)
        .ok_or_else(|| decode(format!("unknown runtime kind `{runtime}` in run_intents")))?;
    let state = IntentState::parse(&state)
        .ok_or_else(|| decode(format!("unknown intent state `{state}` in run_intents")))?;
    Ok(RunIntentRecord {
        project: ProjectKey::new(project),
        run: RunKey::new(WorkKey::new(runtime, work_id), attempt as u64),
        generation,
        state,
        updated_at_ms,
    })
}

type QueuedWorkColumns = (String, String, String, String, String, String, i64, String);

fn queued_work_columns(row: &rusqlite::Row<'_>) -> rusqlite::Result<QueuedWorkColumns> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
    ))
}

fn decode_queued_work(
    columns: QueuedWorkColumns,
) -> Result<QueuedWorkRecord, tokio_rusqlite::Error> {
    let (project, runtime, work_id, root, origin, phase, enqueued_at_ms, payload) = columns;
    let runtime = RuntimeKind::parse(&runtime)
        .ok_or_else(|| decode(format!("unknown runtime kind `{runtime}` in queued_work")))?;
    let origin = WorkOrigin::parse(&origin)
        .ok_or_else(|| decode(format!("unknown work origin `{origin}` in queued_work")))?;
    let phase = WorkPhase::parse(&phase)
        .ok_or_else(|| decode(format!("unknown work phase `{phase}` in queued_work")))?;
    Ok(QueuedWorkRecord {
        project: ProjectKey::new(project),
        work: WorkKey::new(runtime, work_id),
        root: CheckoutKey::new(root),
        origin,
        phase,
        enqueued_at_ms,
        payload,
    })
}

type OutboxColumns = (String, String, String, String, i64, i64, i64, String);

fn outbox_columns(row: &rusqlite::Row<'_>) -> rusqlite::Result<OutboxColumns> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
    ))
}

fn decode_outbox(columns: OutboxColumns) -> Result<DeliveryOutboxRecord, tokio_rusqlite::Error> {
    let (project, message_id, recipient, state, attempts, created_at_ms, updated_at_ms, payload) =
        columns;
    let state = DeliveryState::parse(&state).ok_or_else(|| {
        decode(format!(
            "unknown delivery state `{state}` in delivery_outbox"
        ))
    })?;
    Ok(DeliveryOutboxRecord {
        project: ProjectKey::new(project),
        message_id,
        recipient,
        state,
        attempts,
        created_at_ms,
        updated_at_ms,
        payload,
    })
}

struct DecompositionColumns {
    project: String,
    record_id: String,
    runtime: String,
    work_id: String,
    work_class: String,
    fan_out_group: Option<String>,
    sibling_count: i64,
    wall_clock_ms: i64,
    tokens: i64,
    turns: i64,
    tool_calls: i64,
    succeeded: i64,
    reworked: i64,
    recorded_at_ms: i64,
    paths: String,
}

fn decomposition_columns(row: &rusqlite::Row<'_>) -> rusqlite::Result<DecompositionColumns> {
    Ok(DecompositionColumns {
        project: row.get(0)?,
        record_id: row.get(1)?,
        runtime: row.get(2)?,
        work_id: row.get(3)?,
        work_class: row.get(4)?,
        fan_out_group: row.get(5)?,
        sibling_count: row.get(6)?,
        wall_clock_ms: row.get(7)?,
        tokens: row.get(8)?,
        turns: row.get(9)?,
        tool_calls: row.get(10)?,
        succeeded: row.get(11)?,
        reworked: row.get(12)?,
        recorded_at_ms: row.get(13)?,
        paths: row.get(14)?,
    })
}

fn decode_decomposition(
    columns: DecompositionColumns,
) -> Result<DecompositionOutcomeRecord, tokio_rusqlite::Error> {
    let runtime = RuntimeKind::parse(&columns.runtime).ok_or_else(|| {
        decode(format!(
            "unknown runtime kind `{}` in decomposition_outcomes",
            columns.runtime
        ))
    })?;
    let paths: Vec<String> = serde_json::from_str(&columns.paths)
        .map_err(|err| decode(format!("unreadable paths in decomposition_outcomes: {err}")))?;
    Ok(DecompositionOutcomeRecord {
        project: ProjectKey::new(columns.project),
        record_id: columns.record_id,
        work: WorkKey::new(runtime, columns.work_id),
        work_class: columns.work_class,
        fan_out_group: columns.fan_out_group,
        sibling_count: columns.sibling_count,
        wall_clock_ms: columns.wall_clock_ms,
        tokens: columns.tokens,
        turns: columns.turns,
        tool_calls: columns.tool_calls,
        succeeded: columns.succeeded != 0,
        reworked: columns.reworked != 0,
        recorded_at_ms: columns.recorded_at_ms,
        paths,
    })
}

struct ActivityColumns {
    project: String,
    record_id: String,
    runtime: String,
    work_id: String,
    path: String,
    access: String,
    content_hash: Option<String>,
    observed_at_ms: i64,
    file_key: String,
    file_alias: Option<String>,
    instance_id: String,
    tool_call: Option<String>,
    content_hash_before: Option<String>,
    checkout: Option<String>,
}

fn activity_columns(row: &rusqlite::Row<'_>) -> rusqlite::Result<ActivityColumns> {
    Ok(ActivityColumns {
        project: row.get(0)?,
        record_id: row.get(1)?,
        runtime: row.get(2)?,
        work_id: row.get(3)?,
        path: row.get(4)?,
        access: row.get(5)?,
        content_hash: row.get(6)?,
        observed_at_ms: row.get(7)?,
        file_key: row.get(8)?,
        file_alias: row.get(9)?,
        instance_id: row.get(10)?,
        tool_call: row.get(11)?,
        content_hash_before: row.get(12)?,
        checkout: row.get(13)?,
    })
}

fn decode_activity(
    columns: ActivityColumns,
) -> Result<ActivityObservationRecord, tokio_rusqlite::Error> {
    let runtime = RuntimeKind::parse(&columns.runtime).ok_or_else(|| {
        decode(format!(
            "unknown runtime kind `{}` in activity_observations",
            columns.runtime
        ))
    })?;
    let access = AccessKind::parse(&columns.access).ok_or_else(|| {
        decode(format!(
            "unknown access kind `{}` in activity_observations",
            columns.access
        ))
    })?;
    Ok(ActivityObservationRecord {
        project: ProjectKey::new(columns.project),
        record_id: columns.record_id,
        work: WorkKey::new(runtime, columns.work_id),
        path: columns.path,
        access,
        content_hash: columns.content_hash,
        observed_at_ms: columns.observed_at_ms,
        file_key: columns.file_key,
        file_alias: columns.file_alias,
        instance_id: columns.instance_id,
        tool_call: columns.tool_call,
        content_hash_before: columns.content_hash_before,
        checkout: columns.checkout,
    })
}
