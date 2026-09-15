pub mod filesystem;
pub mod records;
pub mod schema;

use crate::contracts::{WorkOrigin, WorkPhase};
use crate::identity::{CheckoutKey, ProjectKey, RunKey, RuntimeKind, WorkKey};
use crate::settings::{CoordinationSettings, SettingsError};
use filesystem::FilesystemKind;
use records::{
    AccessKind, ActivityObservationRecord, DeliveryOutboxRecord, DeliveryState,
    EngineInstanceRecord, IntentState, LeaseRecord, PresenceMark, PruneReport, QueuedWorkRecord,
    RunIntentRecord,
};
use rusqlite::{params, OptionalExtension};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio_rusqlite::Connection;

const MEMORY_PATH: &str = ":memory:";

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
                let _: String = conn.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))?;
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
                let current: Option<i64> = tx
                    .query_row(
                        "SELECT generation FROM run_intents WHERE project = ?1 AND runtime = ?2 \
                         AND work_id = ?3 AND attempt = ?4",
                        params![scope, runtime, work_id, attempt],
                        |row| row.get(0),
                    )
                    .optional()?;
                let generation = match current {
                    Some(generation) => generation,
                    None => {
                        tx.rollback()?;
                        return Ok(CasOutcome::Missing);
                    }
                };
                if generation != expected_generation {
                    tx.rollback()?;
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
                tx.commit()?;
                let runtime = RuntimeKind::parse(runtime)
                    .ok_or_else(|| decode(format!("unknown runtime kind `{runtime}`")))?;
                Ok(CasOutcome::Updated(Box::new(RunIntentRecord {
                    project: ProjectKey::new(scope),
                    run: RunKey::new(WorkKey::new(runtime, work_id), attempt as u64),
                    generation: next_generation,
                    state: next_state,
                    updated_at_ms,
                })))
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))?;
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
                conn.execute(
                    "INSERT INTO leases (project, lease_key, owner_token, fencing_generation, \
                     acquired_at_ms, expires_at_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
                     ON CONFLICT(project, lease_key) DO UPDATE SET \
                     owner_token = excluded.owner_token, \
                     fencing_generation = excluded.fencing_generation, \
                     acquired_at_ms = excluded.acquired_at_ms, \
                     expires_at_ms = excluded.expires_at_ms",
                    params![
                        scope,
                        record.lease_key,
                        record.owner_token,
                        record.fencing_generation,
                        record.acquired_at_ms,
                        record.expires_at_ms
                    ],
                )?;
                Ok(())
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))
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
                let mut stmt = conn.prepare(
                    "SELECT project, lease_key, owner_token, fencing_generation, acquired_at_ms, \
                     expires_at_ms FROM leases WHERE project = ?1 AND lease_key = ?2",
                )?;
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
                let mut stmt = conn.prepare(
                    "SELECT project, lease_key, owner_token, fencing_generation, acquired_at_ms, \
                     expires_at_ms FROM leases WHERE project = ?1 \
                     ORDER BY expires_at_ms ASC, lease_key ASC LIMIT ?2",
                )?;
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
                     path, access, content_hash, observed_at_ms) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
                     ON CONFLICT(project, record_id) DO UPDATE SET runtime = excluded.runtime, \
                     work_id = excluded.work_id, path = excluded.path, \
                     access = excluded.access, content_hash = excluded.content_hash, \
                     observed_at_ms = excluded.observed_at_ms",
                    params![
                        scope,
                        record.record_id,
                        record.work.runtime.as_str(),
                        record.work.id,
                        record.path,
                        record.access.as_str(),
                        record.content_hash,
                        record.observed_at_ms
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
                let mut stmt = conn.prepare(
                    "SELECT project, record_id, runtime, work_id, path, access, content_hash, \
                     observed_at_ms FROM activity_observations \
                     WHERE project = ?1 AND record_id = ?2",
                )?;
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
                let mut stmt = conn.prepare(
                    "SELECT project, record_id, runtime, work_id, path, access, content_hash, \
                     observed_at_ms FROM activity_observations WHERE project = ?1 \
                     ORDER BY observed_at_ms DESC, record_id ASC LIMIT ?2",
                )?;
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

    pub async fn prune(&self, access: &ProjectAccess) -> Result<PruneReport, StoreError> {
        let scope = self.scope(access)?;
        let settings = self.settings.clone();
        self.conn
            .call(move |conn| {
                let specs: [(&str, &[&str], &str, i64); 6] = [
                    (
                        "engine_instances",
                        &["instance_id"],
                        "heartbeat_at_ms",
                        settings.store_max_engine_instances as i64,
                    ),
                    (
                        "run_intents",
                        &["runtime", "work_id", "attempt"],
                        "updated_at_ms",
                        settings.store_max_run_intents as i64,
                    ),
                    (
                        "queued_work",
                        &["runtime", "work_id"],
                        "enqueued_at_ms",
                        settings.store_max_queued_work as i64,
                    ),
                    (
                        "leases",
                        &["lease_key"],
                        "expires_at_ms",
                        settings.store_max_leases as i64,
                    ),
                    (
                        "delivery_outbox",
                        &["message_id"],
                        "updated_at_ms",
                        settings.store_max_outbox_messages as i64,
                    ),
                    (
                        "activity_observations",
                        &["record_id"],
                        "observed_at_ms",
                        settings.store_max_activity_observations as i64,
                    ),
                ];
                let mut report = PruneReport::default();
                let tx =
                    conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                for (table, keys, order, limit) in specs {
                    let key_list = keys.join(", ");
                    let target = if keys.len() == 1 {
                        key_list.clone()
                    } else {
                        format!("({key_list})")
                    };
                    let sql = format!(
                        "DELETE FROM {table} WHERE project = ?1 AND {target} IN \
                         (SELECT {key_list} FROM {table} WHERE project = ?1 \
                         ORDER BY {order} DESC LIMIT -1 OFFSET ?2)"
                    );
                    let removed = tx.execute(&sql, params![scope, limit])?;
                    match table {
                        "engine_instances" => report.engine_instances = removed,
                        "run_intents" => report.run_intents = removed,
                        "queued_work" => report.queued_work = removed,
                        "leases" => report.leases = removed,
                        "delivery_outbox" => report.delivery_outbox = removed,
                        _ => report.activity_observations = removed,
                    }
                }
                tx.commit()?;
                Ok(report)
            })
            .await
            .map_err(|err| from_tokio(&self.path, err))
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

fn lease_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LeaseRecord> {
    Ok(LeaseRecord {
        project: ProjectKey::new(row.get::<_, String>(0)?),
        lease_key: row.get(1)?,
        owner_token: row.get(2)?,
        fencing_generation: row.get(3)?,
        acquired_at_ms: row.get(4)?,
        expires_at_ms: row.get(5)?,
    })
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

type ActivityColumns = (
    String,
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    i64,
);

fn activity_columns(row: &rusqlite::Row<'_>) -> rusqlite::Result<ActivityColumns> {
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

fn decode_activity(
    columns: ActivityColumns,
) -> Result<ActivityObservationRecord, tokio_rusqlite::Error> {
    let (project, record_id, runtime, work_id, path, access, content_hash, observed_at_ms) =
        columns;
    let runtime = RuntimeKind::parse(&runtime).ok_or_else(|| {
        decode(format!(
            "unknown runtime kind `{runtime}` in activity_observations"
        ))
    })?;
    let access = AccessKind::parse(&access).ok_or_else(|| {
        decode(format!(
            "unknown access kind `{access}` in activity_observations"
        ))
    })?;
    Ok(ActivityObservationRecord {
        project: ProjectKey::new(project),
        record_id,
        work: WorkKey::new(runtime, work_id),
        path,
        access,
        content_hash,
        observed_at_ms,
    })
}
