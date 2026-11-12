use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use refact_buddy_core::operator_types::{
    ActionExecution, ActionExecutionState, ArtifactRevision, OperatorContract, ProjectIdentity,
    ReadState, RunOutcome, WorkItem, WorkSnooze, OPERATOR_SCHEMA_VERSION_V1,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{de::DeserializeOwned, Serialize};

const SCHEMA_VERSION: i64 = 7;
const MAX_RECORD_BYTES: usize = 1_048_576;
const MAX_PAGE_SIZE: usize = 200;

#[derive(Debug)]
pub enum OperatorStoreError {
    Io(std::io::Error),
    Sql(rusqlite::Error),
    Json(serde_json::Error),
    InvalidInput(&'static str),
    RecordTooLarge,
    SchemaTooNew(i64),
    CorruptSchema(&'static str),
    ProjectMismatch,
    AlreadyOwned(Option<OperatorLockOwner>),
    Conflict(&'static str),
    StaleRevision { current: u64 },
}

impl std::fmt::Display for OperatorStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => e.fmt(f),
            Self::Sql(e) => e.fmt(f),
            Self::Json(e) => e.fmt(f),
            Self::InvalidInput(s) | Self::CorruptSchema(s) | Self::Conflict(s) => f.write_str(s),
            Self::RecordTooLarge => f.write_str("operator record exceeds size limit"),
            Self::SchemaTooNew(v) => write!(f, "operator store schema {v} is newer than supported"),
            Self::ProjectMismatch => f.write_str("operator store belongs to a different project"),
            Self::AlreadyOwned(_) => {
                f.write_str("operator store is already owned by another writer")
            }
            Self::StaleRevision {
                current,
            } => {
                write!(f, "stale revision; current revision is {current}")
            }
        }
    }
}
impl std::error::Error for OperatorStoreError {}
impl From<std::io::Error> for OperatorStoreError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<rusqlite::Error> for OperatorStoreError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sql(e)
    }
}
impl From<serde_json::Error> for OperatorStoreError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperatorLockOwner {
    pub pid: u32,
    pub hostname: String,
    pub started_at_ms: u64,
    pub version: String,
    pub command: String,
}

impl OperatorLockOwner {
    fn for_current_process() -> Self {
        Self {
            pid: std::process::id(),
            hostname: hostname::get()
                .ok()
                .and_then(|value| value.into_string().ok())
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "unknown-host".to_string()),
            started_at_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_millis() as u64)
                .unwrap_or(0),
            version: env!("CARGO_PKG_VERSION").to_string(),
            command: std::env::args().collect::<Vec<_>>().join(" "),
        }
    }

    pub fn describe(&self) -> String {
        let started_at = chrono::DateTime::from_timestamp_millis(self.started_at_ms as i64)
            .map(|time| time.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_else(|| "an unknown time".to_string());
        format!(
            "pid {} (refact-lsp {}) on {}, started {}, cmd: {}",
            self.pid, self.version, self.hostname, started_at, self.command
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Stored<T> {
    pub value: T,
    pub revision: u64,
    pub change_seq: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page<T> {
    pub entries: Vec<T>,
    pub next_after: Option<u64>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecutionClaim {
    Claimed(Stored<ActionExecution>),
    Existing(Stored<ActionExecution>),
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ActiveRun {
    start: super::outcome_runtime::WorkflowOutcomeStart,
    checkpoint: super::outcome_runtime::WorkflowOutcomeCheckpoint,
}

const MAX_ACTIVE_RUNS: usize = 200;

pub struct OperatorStore {
    connection: Connection,
    path: PathBuf,
    identity: ProjectIdentity,
    _writer_lock: WriterLock,
}

struct WriterLock(fs::File);

impl Drop for WriterLock {
    fn drop(&mut self) {
        if let Err(error) = self.0.unlock() {
            tracing::warn!(error = %error, "operator writer lock could not be released");
        }
    }
}

// Capability minted only after the store rechecks the exact retained native proof.
pub(crate) struct SupervisionCompletion {
    mandate_id: String,
    outcome: RunOutcome,
}

impl OperatorStore {
    fn read_existing_connection(
        root: &Path,
        identity: &ProjectIdentity,
    ) -> Result<Connection, OperatorStoreError> {
        let root = dunce::canonicalize(root)?;
        if identity.canonical_root != root.to_string_lossy() {
            return Err(OperatorStoreError::ProjectMismatch);
        }
        for path in [root.join(".refact"), root.join(".refact/buddy")] {
            let m = fs::symlink_metadata(&path)?;
            if !m.is_dir() || m.file_type().is_symlink() {
                return Err(OperatorStoreError::InvalidInput(
                    "unsafe operator directory",
                ));
            }
        }
        let path = root.join(".refact/buddy/operator.sqlite");
        reject_symlink_or_permissive_file(&path, "operator database")?;
        let connection =
            Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let version: i64 = connection.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version != SCHEMA_VERSION {
            return Err(OperatorStoreError::CorruptSchema(
                "policy preview requires current existing schema",
            ));
        }
        verify_schema_matches_version(&connection, version)?;
        let binding: String = connection.query_row(
            "SELECT value FROM meta WHERE key='project_identity'",
            [],
            |r| r.get(0),
        )?;
        if decode::<ProjectIdentity>(&binding)? != *identity {
            return Err(OperatorStoreError::ProjectMismatch);
        }
        Ok(connection)
    }

    pub(crate) fn retained_supervision_budget(
        root: &Path,
        identity: &ProjectIdentity,
        chat_id: &str,
    ) -> Result<Option<refact_chat_api::GoalBudget>, OperatorStoreError> {
        if !root.join(".refact/buddy/operator.sqlite").try_exists()? {
            return Ok(None);
        }
        let connection = Self::read_existing_connection(root, identity)?;
        let payload: Option<String> = connection.query_row(
            "SELECT d.payload FROM operator_documents d WHERE d.namespace='supervision_links' AND NOT EXISTS(SELECT 1 FROM operator_documents newer WHERE newer.namespace=d.namespace AND newer.id=d.id AND newer.revision>d.revision) AND (json_extract(d.payload,'$.chat_id')=?1 OR json_extract(d.payload,'$.child_chat_id')=?1)",
            [chat_id], |row| row.get(0),
        ).optional()?;
        let Some(payload) = payload else {
            return Ok(None);
        };
        let link: super::supervision::SupervisionLink = decode(&payload)?;
        if link.project != *identity {
            return Err(OperatorStoreError::ProjectMismatch);
        }
        let payload: String = connection.query_row(
            "SELECT payload FROM operator_documents WHERE namespace='supervision_mandates' AND id=?1 ORDER BY revision DESC LIMIT 1",
            [&link.mandate_id], |row| row.get(0),
        )?;
        let record: super::supervision_authority::MandateRecord = decode(&payload)?;
        if record.project != *identity || record.mandate.mandate_id != link.mandate_id {
            return Err(OperatorStoreError::ProjectMismatch);
        }
        super::supervision::native_budget(&record)
            .map(Some)
            .map_err(|_| OperatorStoreError::CorruptSchema("invalid retained supervision budget"))
    }

    pub(crate) fn read_document_policy_snapshot(
        root: &Path,
        identity: &ProjectIdentity,
    ) -> Result<(super::operator_observability::PolicySnapshot, usize), OperatorStoreError> {
        let connection = Self::read_existing_connection(root, identity)?;
        validate_document_accounting(&connection)?;
        let epoch: Option<String> = connection
            .query_row(
                "SELECT value FROM meta WHERE key='document_policy_epoch'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        let payload: Option<String> = connection
            .query_row(
                "SELECT value FROM meta WHERE key='document_growth_policy'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        let policy: Option<super::operator_observability::DocumentGrowthPolicy> =
            payload.map(|s| decode(&s)).transpose()?.flatten();
        if let Some(p) = &policy {
            if !(1..=100_000).contains(&p.max_rows_per_namespace) {
                return Err(OperatorStoreError::CorruptSchema("invalid policy"));
            }
            document_bound(&connection, p)?;
        }
        Ok((
            super::operator_observability::PolicySnapshot {
                epoch: epoch
                    .map(|s| {
                        s.parse()
                            .map_err(|_| OperatorStoreError::CorruptSchema("invalid epoch"))
                    })
                    .transpose()?
                    .unwrap_or(0),
                policy,
            },
            document_charge(&connection)?,
        ))
    }

    pub fn policy_snapshot(
        &self,
    ) -> Result<super::operator_observability::PolicySnapshot, OperatorStoreError> {
        let epoch: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM meta WHERE key='document_policy_epoch'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        let epoch = epoch
            .map(|s| {
                s.parse::<u64>()
                    .map_err(|_| OperatorStoreError::CorruptSchema("invalid document policy epoch"))
            })
            .transpose()?
            .unwrap_or(0);
        Ok(super::operator_observability::PolicySnapshot {
            epoch,
            policy: self.document_growth_policy()?,
        })
    }

    /// Owner-only CAS; callers must pass through the registered exact-action service.
    pub(crate) fn compare_set_document_policy(
        &mut self,
        expected: &str,
        policy: Option<super::operator_observability::DocumentGrowthPolicy>,
    ) -> Result<super::operator_observability::PolicySnapshot, OperatorStoreError> {
        let old = self.policy_snapshot()?;
        if old.version() != expected {
            return Err(OperatorStoreError::Conflict(
                "policy changed; prepare and grant again",
            ));
        }
        let epoch = old
            .epoch
            .checked_add(1)
            .ok_or(OperatorStoreError::Conflict("policy epoch exhausted"))?;
        self.set_document_growth_policy_at_epoch(policy, epoch)?;
        self.policy_snapshot()
    }

    pub(crate) fn document_policy_charge(&self) -> Result<usize, OperatorStoreError> {
        document_charge(&self.connection)
    }

    /// Authenticated native policy control only. Audit and policy commit together;
    /// abandoned previews allocate nothing, and a raise is charged to the NEW cap.
    pub(crate) fn apply_document_policy_with_audit(
        &mut self,
        expected: &str,
        policy: Option<super::operator_observability::DocumentGrowthPolicy>,
        audit: &super::resources::retention::PolicyControlAudit,
    ) -> Result<super::operator_observability::PolicySnapshot, OperatorStoreError> {
        use super::resources::retention::PolicyControlRecord;
        if audit.project != self.identity
            || audit.spec_hash.is_empty()
            || audit.idempotency_key.is_empty()
        {
            return Err(OperatorStoreError::ProjectMismatch);
        }
        if policy
            .as_ref()
            .is_some_and(|p| !(1..=100_000).contains(&p.max_rows_per_namespace))
        {
            return Err(OperatorStoreError::InvalidInput(
                "invalid document capacity",
            ));
        }
        let epoch: u64 = expected
            .strip_prefix("document-policy:")
            .and_then(|s| s.parse().ok())
            .ok_or(OperatorStoreError::InvalidInput("invalid policy epoch"))?;
        let next = epoch
            .checked_add(1)
            .ok_or(OperatorStoreError::Conflict("policy epoch exhausted"))?;
        let id = format!("document-policy:{next}");
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current_epoch: Option<String> = tx
            .query_row(
                "SELECT value FROM meta WHERE key='document_policy_epoch'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        let current_policy: Option<String> = tx
            .query_row(
                "SELECT value FROM meta WHERE key='document_growth_policy'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        let current = super::operator_observability::PolicySnapshot {
            epoch: current_epoch
                .map(|s| {
                    s.parse::<u64>()
                        .map_err(|_| OperatorStoreError::CorruptSchema("invalid policy epoch"))
                })
                .transpose()?
                .unwrap_or(0),
            policy: current_policy.map(|s| decode(&s)).transpose()?.flatten(),
        };
        if current
            .policy
            .as_ref()
            .is_some_and(|p| !(1..=100_000).contains(&p.max_rows_per_namespace))
        {
            return Err(OperatorStoreError::CorruptSchema("invalid document policy"));
        }
        let reused: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM operator_documents WHERE namespace='policy_control_audit' AND id<>?1 AND json_extract(payload,'$.audit.idempotency_key')=?2)", params![id,audit.idempotency_key], |r| r.get(0))?;
        if reused {
            return Err(OperatorStoreError::Conflict(
                "policy idempotency key already consumed",
            ));
        }
        let saved: Option<String> = tx.query_row("SELECT payload FROM operator_documents WHERE namespace='policy_control_audit' AND id=?1 ORDER BY revision DESC LIMIT 1", [&id], |r| r.get(0)).optional()?;
        if let Some(saved) = saved {
            let saved: PolicyControlRecord = decode(&saved)?;
            if saved.audit == *audit && saved.after.policy == policy && current == saved.after {
                return Ok(saved.after);
            }
            return Err(OperatorStoreError::Conflict(
                "policy audit identity already consumed",
            ));
        }
        if current.version() != expected || document_charge(&tx)? != audit.expected_charge {
            return Err(OperatorStoreError::Conflict(
                "policy or document charge changed; preview and confirm again",
            ));
        }
        let after = super::operator_observability::PolicySnapshot {
            epoch: next,
            policy: policy.clone(),
        };
        let record = PolicyControlRecord {
            schema_version: 1,
            project: self.identity.clone(),
            audit: audit.clone(),
            before: current,
            after: after.clone(),
        };
        let payload = encode(&record)?;
        Self::set_document_growth_policy_in_tx(&tx, policy.clone(), next)?;
        admit_document_charge(&tx, policy.as_ref(), 1)?;
        let seq = next_seq(&tx)?;
        tx.execute("INSERT INTO operator_documents(namespace,id,revision,payload,change_seq) VALUES('policy_control_audit',?1,1,?2,?3)", params![id,payload,seq])?;
        tx.commit()?;
        Ok(after)
    }

    pub(crate) fn historical_document_policy(
        &self,
        epoch: u64,
    ) -> Result<Option<super::operator_observability::DocumentGrowthPolicy>, OperatorStoreError>
    {
        let payload: String = self.connection.query_row(
            "SELECT value FROM meta WHERE key=?1",
            [format!("document_policy_history:{epoch}")],
            |r| r.get(0),
        )?;
        decode(&payload)
    }

    pub(crate) fn put_editorial_document(
        &mut self,
        value: &super::editorial::EditorialState,
        revision: Option<u64>,
    ) -> Result<Stored<super::editorial::EditorialState>, OperatorStoreError> {
        self.put_document_inner(
            "editorial",
            "attention-v1",
            value,
            revision,
            None,
            false,
            true,
            false,
        )
    }

    pub(crate) fn put_recovery_grant(
        &mut self,
        id: &str,
        value: &super::resource_execution::ResourceDocument<
            super::resource_execution::ExactActionGrant,
        >,
    ) -> Result<(), OperatorStoreError> {
        let original_id = value.payload.recovery_execution_id.as_deref().ok_or(
            OperatorStoreError::InvalidInput("recovery original missing"),
        )?;
        let original = self
            .get_execution(original_id)?
            .ok_or(OperatorStoreError::Conflict("recovery original missing"))?
            .value;
        if original.spec_hash != value.payload.spec_hash
            || original.project != value.project
            || original.recovery.as_ref().is_none_or(|r| !r.available)
        {
            return Err(OperatorStoreError::Conflict(
                "recovery grant binding changed",
            ));
        }
        let policy = self.document_growth_policy()?;
        let tx = self.connection.transaction()?;
        let existing: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM terminal_reservations WHERE producer='recovery_grant' AND producer_id=?1)", [original_id], |r| r.get(0))?;
        if !existing {
            let changed = tx.execute(
                "UPDATE meta SET value='2' WHERE key=?1 AND value='4'",
                [format!("document_recovery_credit:{original_id}")],
            )?;
            if changed != 1 && policy.is_some() {
                return Err(OperatorStoreError::Conflict(
                    "recovery grant reserve exhausted",
                ));
            }
        }
        reserve_terminal_documents_in_tx(
            &tx,
            policy.as_ref(),
            "recovery_grant",
            original_id,
            &[("action_grant", id.to_owned(), 1)],
            Some("recovery_grant"),
        )?;
        tx.commit()?;
        self.put_reserved_document(
            "recovery_grant",
            original_id,
            "action_grant",
            id,
            value,
            None,
        )
        .map(|_| ())
    }

    pub(crate) fn put_native_ordinary_document<T: Serialize + DeserializeOwned + Clone>(
        &mut self,
        producer: &str,
        producer_id: &str,
        namespace: &str,
        id: &str,
        value: &T,
        revision: Option<u64>,
    ) -> Result<Stored<T>, OperatorStoreError> {
        let owned: bool = self.connection.query_row("SELECT EXISTS(SELECT 1 FROM terminal_reservations WHERE producer=?1 AND producer_id=?2 AND namespace=?3 AND document_id=?4)", params![producer,producer_id,namespace,id], |r| r.get(0))?;
        if !owned {
            return Err(OperatorStoreError::Conflict(
                "native ordinary write reservation missing",
            ));
        }
        self.put_document_inner(namespace, id, value, revision, None, false, false, true)
    }

    pub(crate) fn reserve_native_documents(
        &mut self,
        producer: &str,
        id: &str,
        slots: &[(&str, String, usize)],
    ) -> Result<(), OperatorStoreError> {
        self.reserve_terminal_documents_inner(producer, id, slots, None)
    }

    /// Only a native producer holding the exact reservation can consume its credit.
    /// Unlike the final terminal write, this does not release sibling slots.
    pub(crate) fn put_reserved_document<T: Serialize + DeserializeOwned + Clone>(
        &mut self,
        producer: &str,
        producer_id: &str,
        namespace: &str,
        id: &str,
        value: &T,
        expected_revision: Option<u64>,
    ) -> Result<Stored<T>, OperatorStoreError> {
        self.put_document_inner(
            namespace,
            id,
            value,
            expected_revision,
            Some((producer, producer_id)),
            false,
            false,
            false,
        )
    }

    pub(crate) fn reserve_terminal_documents(
        &mut self,
        producer: &str,
        id: &str,
        slots: &[(&str, String, usize)],
    ) -> Result<(), OperatorStoreError> {
        self.reserve_terminal_documents_inner(producer, id, slots, None)
    }

    pub(crate) fn reserve_grandfathered_terminal_documents(
        &mut self,
        producer: &str,
        id: &str,
        slots: &[(&str, String, usize)],
    ) -> Result<(), OperatorStoreError> {
        let cutoff: u64 = self.connection.query_row(
            "SELECT CAST(value AS INTEGER) FROM meta WHERE key=CASE WHEN EXISTS(SELECT 1 FROM meta WHERE key='document_global_cutoff') THEN 'document_global_cutoff' ELSE 'terminal_reservation_legacy_cutoff' END",
            [], |row| row.get(0),
        )?;
        let legacy: bool = match producer {
            "fixed_evaluation" => self.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM operator_documents WHERE namespace='improvement_fixed_runs' AND id=?1 AND change_seq<=?2)",
                params![id, cutoff], |row| row.get(0),
            )?,
            "resource_apply" => self.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM executions WHERE execution_id=?1 AND change_seq<=?2)",
                params![id, cutoff], |row| row.get(0),
            )?,
            "native_installation" | "artifact_stage" | "artifact_cleanup" | "host_release" | "supervision" => self.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM operator_documents WHERE id=?1 AND change_seq<=?2 AND namespace=?3)",
                params![id,cutoff,match producer { "native_installation" => "improvement_installation", "artifact_stage" => "improvement_artifacts", "artifact_cleanup" => "improvement_artifact_cleanup", "host_release" => "improvement_host_release", _ => "supervision_mandates" }], |r| r.get(0),
            )?,
            _ => false,
        };
        let existing: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM terminal_reservations WHERE producer=?1 AND producer_id=?2)",
            params![producer, id], |row| row.get(0),
        )?;
        if !legacy && !existing {
            return Err(OperatorStoreError::Conflict(
                "no pre-migration producer evidence",
            ));
        }
        self.reserve_terminal_documents_inner(producer, id, slots, Some("grandfathered_legacy"))
    }

    pub(crate) fn reserve_policy_terminal_documents(
        &mut self,
        spec: &refact_buddy_core::operator_types::ActionSpec,
        id: &str,
        slots: &[(&str, String, usize)],
    ) -> Result<(), OperatorStoreError> {
        if !super::resources::retention::is_policy_action(spec) || spec.project != self.identity {
            return Err(OperatorStoreError::ProjectMismatch);
        }
        let execution = self
            .get_execution(id)?
            .ok_or(OperatorStoreError::Conflict(
                "policy execution claim missing",
            ))?
            .value;
        if execution.status != ActionExecutionState::Running
            || execution.spec_hash
                != super::action_execution::action_spec_hash(spec, &spec.parameter_ref)
            || execution.action_id != spec.action_id
        {
            return Err(OperatorStoreError::Conflict(
                "policy execution binding changed",
            ));
        }
        self.reserve_terminal_documents_inner("resource_apply", id, slots, Some("policy_control"))
    }

    fn reserve_terminal_documents_inner(
        &mut self,
        producer: &str,
        id: &str,
        slots: &[(&str, String, usize)],
        exemption: Option<&str>,
    ) -> Result<(), OperatorStoreError> {
        let policy = self.document_growth_policy()?;
        let tx = self.connection.transaction()?;
        reserve_terminal_documents_in_tx(&tx, policy.as_ref(), producer, id, slots, exemption)?;
        tx.commit()?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn set_document_growth_policy(
        &mut self,
        policy: Option<super::operator_observability::DocumentGrowthPolicy>,
    ) -> Result<(), OperatorStoreError> {
        let version = self.policy_snapshot()?.version();
        self.compare_set_document_policy(&version, policy)
            .map(|_| ())
    }

    fn set_document_growth_policy_at_epoch(
        &mut self,
        policy: Option<super::operator_observability::DocumentGrowthPolicy>,
        epoch: u64,
    ) -> Result<(), OperatorStoreError> {
        if policy
            .as_ref()
            .is_some_and(|p| !(1..=100_000).contains(&p.max_rows_per_namespace))
        {
            return Err(OperatorStoreError::InvalidInput(
                "document capacity must be 1..=100000",
            ));
        }
        let tx = self.connection.transaction()?;
        Self::set_document_growth_policy_in_tx(&tx, policy, epoch)?;
        tx.commit()?;
        Ok(())
    }

    fn set_document_growth_policy_in_tx(
        tx: &Transaction<'_>,
        policy: Option<super::operator_observability::DocumentGrowthPolicy>,
        epoch: u64,
    ) -> Result<(), OperatorStoreError> {
        if policy.is_some() {
            initialize_document_accounting(tx)?;
            // History and the fixed epoch/current-policy keys are charged under the
            // explicitly proposed budget, so a raise works even at the old ceiling.
        }
        if policy
            .as_ref()
            .is_some_and(|p| p.replace_editorial_aggregate)
        {
            // Freeze everything predating authorization, including on re-enablement.
            let revision: u64 = tx.query_row("SELECT COALESCE(max(revision),0) FROM operator_documents WHERE namespace='editorial' AND id='attention-v1'", [], |r| r.get(0))?;
            tx.execute("INSERT INTO meta(key,value) VALUES('editorial_frozen_revision',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [revision.to_string()])?;
        }
        tx.execute("INSERT OR IGNORE INTO meta(key,value) VALUES('document_policy_history:0',COALESCE((SELECT value FROM meta WHERE key='document_growth_policy'),'null'))", [])?;
        tx.execute(
            "INSERT INTO meta(key,value) VALUES(?1,?2)",
            params![format!("document_policy_history:{epoch}"), encode(&policy)?],
        )?;
        tx.execute("INSERT INTO meta(key,value) VALUES('document_policy_epoch',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [epoch.to_string()])?;
        tx.execute("INSERT INTO meta(key,value) VALUES('document_growth_policy',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [encode(&policy)?])?;
        admit_document_charge(tx, policy.as_ref(), 0)?;
        Ok(())
    }

    fn document_growth_policy(
        &self,
    ) -> Result<Option<super::operator_observability::DocumentGrowthPolicy>, OperatorStoreError>
    {
        let payload: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM meta WHERE key='document_growth_policy'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        let policy: Option<super::operator_observability::DocumentGrowthPolicy> =
            payload.map(|p| decode(&p)).transpose()?.flatten();
        if policy
            .as_ref()
            .is_some_and(|p| !(1..=100_000).contains(&p.max_rows_per_namespace))
        {
            return Err(OperatorStoreError::CorruptSchema(
                "invalid document capacity policy",
            ));
        }
        if let Some(active) = &policy {
            document_bound(&self.connection, active)?;
        }
        Ok(policy)
    }

    /// Editorial owner calls this only for new selections, never publish/claim/feedback.
    pub(crate) fn admit_editorial_selection(&self) -> Result<(), OperatorStoreError> {
        if let Some(policy) = self.document_growth_policy()? {
            let full: bool = self.connection.query_row("SELECT EXISTS(SELECT 1 FROM operator_documents WHERE namespace='editorial' LIMIT 1 OFFSET ?1)", [policy.max_rows_per_namespace - 1], |r| r.get(0))?;
            if full {
                return Err(OperatorStoreError::Conflict("editorial selection history capacity reached; delivery resolution remains available"));
            }
        }
        Ok(())
    }

    /// Historical evidentiary versions remain addressable even after admission closes.
    pub fn get_document_revision<T: DeserializeOwned>(
        &self,
        namespace: &str,
        id: &str,
        revision: u64,
    ) -> Result<Option<Stored<T>>, OperatorStoreError> {
        document_key(namespace, id)?;
        let row: Option<(String, u64)> = self.connection.query_row("SELECT payload,change_seq FROM operator_documents WHERE namespace=?1 AND id=?2 AND revision=?3", params![namespace,id,revision], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
        row.map(|(payload, change_seq)| {
            self.validate_document(&payload)?;
            Ok(Stored {
                value: decode(&payload)?,
                revision,
                change_seq,
            })
        })
        .transpose()
    }

    /// Explicit native-owner policy installation in the one project authority.
    /// No default policy is silently activated and there is no HTTP setter.
    pub fn set_capacity_policy(
        &mut self,
        policy: Option<super::operator_observability::CapacityPolicy>,
    ) -> Result<(), OperatorStoreError> {
        if policy.as_ref().is_some_and(|p| {
            p.max_rows_per_table == 0 || p.max_rows_per_table > 100_000 || p.max_database_bytes == 0
        }) {
            return Err(OperatorStoreError::InvalidInput(
                "capacity row threshold must be 1..=100000 and bytes positive",
            ));
        }
        let growth_policy = self.document_growth_policy()?;
        let tx = self.connection.transaction()?;
        tx.execute("INSERT INTO meta(key,value) VALUES('capacity_policy',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [encode(&policy)?])?;
        admit_document_charge(&tx, growth_policy.as_ref(), 0)?;
        tx.commit()?;
        Ok(())
    }

    fn capacity_policy(
        &self,
    ) -> Result<Option<super::operator_observability::CapacityPolicy>, OperatorStoreError> {
        let payload: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM meta WHERE key='capacity_policy'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        payload.map(|p| decode(&p)).transpose().map(Option::flatten)
    }

    fn admit_new_work(&self) -> Result<(), OperatorStoreError> {
        let Some(policy) = self.capacity_policy()? else {
            return Ok(());
        };
        if policy.max_rows_per_table == 0
            || policy.max_rows_per_table > 100_000
            || policy.max_database_bytes == 0
        {
            return Err(OperatorStoreError::CorruptSchema("invalid capacity policy"));
        }
        for table in [
            "run_outcomes",
            "artifacts",
            "operator_documents",
            "executions",
            "work_items",
            "changes",
        ] {
            let full: bool = self.connection.query_row(
                &format!("SELECT EXISTS(SELECT 1 FROM {table} LIMIT 1 OFFSET ?1)"),
                [policy.max_rows_per_table - 1],
                |r| r.get(0),
            )?;
            if full {
                return Err(OperatorStoreError::Conflict(
                    "new work capacity reached; terminal writes remain available",
                ));
            }
        }
        let pages: u64 = self
            .connection
            .query_row("PRAGMA page_count", [], |r| r.get(0))?;
        let size: u64 = self
            .connection
            .query_row("PRAGMA page_size", [], |r| r.get(0))?;
        if pages.saturating_mul(size) >= policy.max_database_bytes {
            return Err(OperatorStoreError::Conflict(
                "new work byte threshold reached; terminal writes remain available",
            ));
        }
        Ok(())
    }

    /// Opt-in bounded snapshot: fixed table names and counts only, never payloads or paths.
    pub fn diagnostic_summary(
        &self,
    ) -> Result<super::operator_observability::StoreSummary, OperatorStoreError> {
        self.diagnostic_summary_inner(false)
            .map(|(summary, _)| summary)
    }

    pub fn diagnostic_summary_with_growth_warnings(
        &self,
    ) -> Result<(super::operator_observability::StoreSummary, Vec<String>), OperatorStoreError>
    {
        self.diagnostic_summary_inner(true)
    }

    fn diagnostic_summary_inner(
        &self,
        evaluate_growth: bool,
    ) -> Result<(super::operator_observability::StoreSummary, Vec<String>), OperatorStoreError>
    {
        use super::operator_observability::*;
        let mut tables = Vec::new();
        let mut protected = 0;
        for table in [
            "run_outcomes",
            "artifacts",
            "work_items",
            "executions",
            "active_runs",
            "operator_documents",
            "read_states",
            "work_snoozes",
            "changes",
        ] {
            let (rows, bytes): (u64, u64) = if table == "changes" {
                let mut statement = self
                    .connection
                    .prepare("SELECT seq FROM changes LIMIT ?1")?;
                let sample = statement
                    .query_map([MAX_DIAGNOSTIC_ROWS_PER_TABLE], |row| row.get::<_, u64>(0))?
                    .collect::<Result<Vec<_>, _>>()?;
                (sample.len() as u64, (sample.len() as u64).saturating_mul(8))
            } else {
                let sql = format!("SELECT count(*),COALESCE(sum(bytes),0) FROM (SELECT length(CAST(payload AS BLOB)) AS bytes FROM {table} LIMIT ?1)");
                self.connection
                    .query_row(&sql, [MAX_DIAGNOSTIC_ROWS_PER_TABLE], |r| {
                        Ok((r.get(0)?, r.get(1)?))
                    })?
            };
            let truncated: bool = self.connection.query_row(
                &format!("SELECT EXISTS(SELECT 1 FROM {table} LIMIT 1 OFFSET ?1)"),
                [MAX_DIAGNOSTIC_ROWS_PER_TABLE],
                |r| r.get(0),
            )?;
            if table != "changes" {
                protected += rows;
            }
            tables.push(TableSummary {
                table,
                sampled_rows: rows,
                payload_bytes_lower_bound: bytes,
                truncated,
                enforced_row_capacity: (table == "active_runs").then_some(MAX_ACTIVE_RUNS),
            });
        }
        let (active, unknown) = self.connection.query_row("SELECT COALESCE(sum(json_extract(payload,'$.status') IN ('claimed','running','reconciling')),0),COALESCE(sum(json_extract(payload,'$.status')='outcome_unknown'),0) FROM (SELECT payload FROM executions ORDER BY execution_id LIMIT ?1)", [MAX_DIAGNOSTIC_ROWS_PER_TABLE], |r| Ok((r.get(0)?,r.get(1)?)))?;
        let growth_thresholds = GrowthThresholds::from_env();
        let growth = self.store_growth_metrics()?;
        let warnings = if evaluate_growth {
            evaluate_process_growth_warnings(&growth, growth_thresholds)
        } else {
            Vec::new()
        };
        let pragma = |name: &str| -> Result<u64, OperatorStoreError> {
            Ok(self
                .connection
                .query_row(&format!("PRAGMA {name}"), [], |r| r.get(0))?)
        };
        let summary = StoreSummary {
            interrupted_workflow_outcomes_lower_bound: self.connection.query_row("SELECT count(*) FROM (SELECT payload FROM run_outcomes LIMIT ?1) WHERE json_extract(payload,'$.failure_class')='engine_interrupted'", [MAX_DIAGNOSTIC_ROWS_PER_TABLE], |r| r.get(0))?,
            new_work_admission_policy: self.capacity_policy()?,
            document_growth: DocumentGrowthSummary {
                snapshot: self.policy_snapshot()?,
                terminal_reservations: self.connection.query_row("SELECT count(*),COALESCE(sum(remaining),0),COALESCE(sum(coverage IN ('grandfathered_inactive','grandfathered_legacy')),0),COALESCE(sum(coverage='policy_control'),0) FROM terminal_reservations", [], |r| Ok(TerminalReservationSummary { retained_entries:r.get(0)?,outstanding_slots:r.get(1)?,grandfathered_entries:r.get(2)?,policy_control_entries:r.get(3)?,retained_ledger_is_charged:true,covered_producers:&["resource_apply", "fixed_evaluation", "native_installation", "artifact_stage", "artifact_cleanup", "host_release", "supervision", "supervision_completion", "recovery_grant"],unclassified_writes_exempt:false,hard_database_quota:false }))?,
                editorial_mutable_slots: usize::from(self.document_growth_policy()?.is_some_and(|p| p.replace_editorial_aggregate)),
                editorial_selection_admission: self.document_growth_policy()?.is_some(),
                sequence_allocator_growth_bounded: false,
                policy: self.document_growth_policy()?,
                admission_namespaces: DOCUMENT_ADMISSION_NAMESPACES,
                recovery_exemptions: DOCUMENT_RECOVERY_EXEMPTIONS,
                charged_document_units: document_charge(&self.connection)?,
                reserved_namespaces: DOCUMENT_RESERVED_NAMESPACES,
                global_document_bound: self.document_growth_policy()?.as_ref().map(|p| document_bound(&self.connection,p)).transpose()?,
                unclassified_namespaces_may_grow: self.document_growth_policy()?.is_none(),
                editorial_frozen_through_revision: self.connection.query_row("SELECT CAST(value AS INTEGER) FROM meta WHERE key='editorial_frozen_revision'", [], |r| r.get(0)).optional()?,
            },
            generic_document_growth_is_native_owner_governed: true,
            schema_version: 1,
            active_runs: tables.iter().find(|t| t.table == "active_runs").map_or(0, |t| t.sampled_rows),
            tables,
            record_byte_limit: MAX_RECORD_BYTES,
            page_limit: MAX_PAGE_SIZE,
            checkpoint_evidence_limit: super::outcome_runtime::MAX_OUTCOME_EVIDENCE,
            scan_rows_per_table: MAX_DIAGNOSTIC_ROWS_PER_TABLE,
            database_allocated_bytes: pragma("page_count")?.saturating_mul(pragma("page_size")?),
            database_reusable_bytes: pragma("freelist_count")?.saturating_mul(pragma("page_size")?),
            effective_database_byte_capacity: pragma("max_page_count")?.saturating_mul(pragma("page_size")?),
            database_capacity_excludes_wal_and_backups: true,
            active_executions_lower_bound: active,
            unknown_executions_lower_bound: unknown,
            protected_payload_rows_lower_bound: protected,
            evidence_deletion_supported: false,
            growth_thresholds,
            growth,
        };
        Ok((summary, warnings))
    }

    fn store_growth_metrics(
        &self,
    ) -> Result<super::operator_observability::StoreGrowthMetrics, OperatorStoreError> {
        use super::operator_observability::*;
        let mut files = Vec::new();
        let mut total_disk_bytes = 0u64;
        for (label, path) in [
            ("db", self.path.clone()),
            ("wal", sidecar_path(&self.path, "-wal")),
            ("shm", sidecar_path(&self.path, "-shm")),
        ] {
            let (size_bytes, modified_at_ms) = match fs::metadata(path) {
                Ok(metadata) if metadata.is_file() => {
                    let modified_at_ms = metadata
                        .modified()
                        .ok()
                        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                        .map(|duration| duration.as_millis());
                    (metadata.len(), modified_at_ms)
                }
                Ok(_) => (0, None),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => (0, None),
                Err(error) => return Err(error.into()),
            };
            total_disk_bytes = total_disk_bytes.saturating_add(size_bytes);
            files.push(StoreFileMetric {
                file: label,
                size_bytes,
                modified_at_ms,
            });
        }
        let mut table_row_counts = Vec::new();
        for table in [
            "run_outcomes",
            "artifacts",
            "work_items",
            "executions",
            "active_runs",
            "operator_documents",
            "read_states",
            "work_snoozes",
            "changes",
        ] {
            let exact_rows = if table == "changes" {
                None
            } else {
                Some(self.connection.query_row(
                    &format!("SELECT count(*) FROM {table}"),
                    [],
                    |row| row.get(0),
                )?)
            };
            table_row_counts.push(TableRowCount {
                table,
                exact_rows,
            });
        }
        let (min_seq, max_seq): (Option<u64>, Option<u64>) = self.connection.query_row(
            "SELECT (SELECT min(seq) FROM changes), (SELECT max(seq) FROM changes)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        let retained_seq_span = match (min_seq, max_seq) {
            (Some(min), Some(max)) => max.saturating_sub(min).saturating_add(1),
            _ => 0,
        };
        let mut stmt = self.connection.prepare("SELECT artifact_id,count(*) AS revisions FROM artifacts GROUP BY artifact_id ORDER BY revisions DESC,artifact_id LIMIT 5")?;
        let top_artifacts_by_revision_count = stmt
            .query_map([], |row| {
                Ok(ArtifactRevisionCount {
                    artifact_id: row.get(0)?,
                    revisions: row.get(1)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let first_recorded_outcome_started_at = self.connection.query_row("SELECT json_extract(payload,'$.started_at') FROM run_outcomes ORDER BY rowid ASC LIMIT 1", [], |row| row.get(0)).optional()?;
        let last_recorded_outcome_started_at = self.connection.query_row("SELECT json_extract(payload,'$.started_at') FROM run_outcomes ORDER BY rowid DESC LIMIT 1", [], |row| row.get(0)).optional()?;
        Ok(StoreGrowthMetrics {
            total_disk_bytes,
            files,
            table_row_counts,
            min_seq,
            max_seq,
            retained_seq_span,
            retained_seq_span_exact_under_prefix_compaction: true,
            first_recorded_outcome_started_at,
            last_recorded_outcome_started_at,
            top_artifacts_by_revision_count,
        })
    }

    /// Safe proposal only: all evidence remains protected until an owning retention
    /// policy defines pins, decisions, and recent-revision/receipt windows.
    pub fn retention_preview(
        &self,
    ) -> Result<super::operator_observability::RetentionPreview, OperatorStoreError> {
        let prepared = self.prepare_sequence_compaction()?;
        Ok(super::operator_observability::RetentionPreview {
            policy: "preserve_all_evidence_compact_sequence_only_v1",
            evidence_deletion_supported: false,
            protected_payload_rows_lower_bound: self
                .diagnostic_summary()?
                .protected_payload_rows_lower_bound,
            redundant_sequence_rows_in_batch: prepared.rows,
            sequence_high_watermark: prepared.watermark,
            max_batch_rows: super::operator_observability::MAX_COMPACTION_ROWS,
            requires_native_policy_for_evidence_deletion: true,
        })
    }

    /// Explicit maintenance preparation, not called by startup, writes, or timers.
    pub fn prepare_sequence_compaction(
        &self,
    ) -> Result<super::operator_observability::PreparedSequenceCompaction, OperatorStoreError> {
        let watermark: u64 =
            self.connection
                .query_row("SELECT COALESCE(max(seq),0) FROM changes", [], |r| r.get(0))?;
        let (rows, cutoff) = self.connection.query_row(
            "SELECT count(*),COALESCE(max(seq),0) FROM (SELECT seq FROM changes WHERE seq<?1 ORDER BY seq LIMIT ?2)",
            params![watermark, super::operator_observability::MAX_COMPACTION_ROWS], |r| Ok((r.get(0)?,r.get(1)?)))?;
        Ok(super::operator_observability::PreparedSequenceCompaction {
            project: self.identity.clone(),
            watermark,
            cutoff,
            rows,
        })
    }

    /// Apply an explicitly prepared bounded batch. Only redundant allocator rows
    /// are removed; all payloads and the max sequence survive. No VACUUM or file
    /// deletion: reusable pages are reported honestly, not promised as disk savings.
    pub fn apply_sequence_compaction(
        &mut self,
        prepared: super::operator_observability::PreparedSequenceCompaction,
    ) -> Result<usize, OperatorStoreError> {
        if prepared.project != self.identity {
            return Err(OperatorStoreError::ProjectMismatch);
        }
        let tx = self.connection.transaction()?;
        let watermark: u64 =
            tx.query_row("SELECT COALESCE(max(seq),0) FROM changes", [], |r| r.get(0))?;
        if watermark != prepared.watermark {
            return Err(OperatorStoreError::Conflict(
                "sequence changed; prepare cleanup again",
            ));
        }
        let rows = tx.execute(
            "DELETE FROM changes WHERE seq<=?1 AND seq<?2",
            params![prepared.cutoff, watermark],
        )?;
        if rows > prepared.rows || rows > super::operator_observability::MAX_COMPACTION_ROWS {
            return Err(OperatorStoreError::Conflict("cleanup batch changed"));
        }
        tx.commit()?;
        Ok(rows)
    }

    pub fn open(
        project_root: &Path,
        identity: &ProjectIdentity,
    ) -> Result<Self, OperatorStoreError> {
        validate_identity(identity)?;
        let canonical_root = dunce::canonicalize(project_root)?;
        if identity.canonical_root != canonical_root.to_string_lossy() {
            return Err(OperatorStoreError::ProjectMismatch);
        }
        let refact_dir = private_child_directory(&canonical_root, ".refact")?;
        let directory = private_child_directory(&refact_dir, "buddy")?;
        let path = directory.join("operator.sqlite");
        let lock_path = directory.join("operator.lock");
        reject_symlink_or_permissive_file(&lock_path, "operator lock")?;
        let mut lock_options = fs::OpenOptions::new();
        lock_options.create(true).read(true).write(true);
        #[cfg(unix)]
        lock_options.mode(0o600);
        let mut writer_lock = lock_options.open(lock_path)?;
        restrict_file(&writer_lock)?;
        match writer_lock.try_lock() {
            Ok(()) => {}
            Err(fs::TryLockError::WouldBlock) => {
                return Err(OperatorStoreError::AlreadyOwned(read_lock_owner(
                    &mut writer_lock,
                )));
            }
            Err(fs::TryLockError::Error(error)) => return Err(OperatorStoreError::Io(error)),
        }
        write_lock_owner(&mut writer_lock, &OperatorLockOwner::for_current_process())?;
        let writer_lock = WriterLock(writer_lock);
        reject_symlink_or_permissive_file(&path, "operator database")?;
        for suffix in ["-wal", "-shm", "-journal"] {
            reject_symlink_or_permissive_file(
                &sidecar_path(&path, suffix),
                "operator database sidecar",
            )?;
        }
        let mut db_options = fs::OpenOptions::new();
        db_options.create_new(true).read(true).write(true);
        #[cfg(unix)]
        db_options.mode(0o600);
        match db_options.open(&path) {
            Ok(file) => {
                restrict_file(&file)?;
                drop(file);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        let opened = (|| {
            let connection = Connection::open(&path)?;
            let version: i64 = connection.query_row("PRAGMA user_version", [], |r| r.get(0))?;
            if version > SCHEMA_VERSION {
                return Err(OperatorStoreError::SchemaTooNew(version));
            }
            verify_schema_matches_version(&connection, version)?;
            if version == SCHEMA_VERSION {
                validate_document_accounting(&connection)?;
            }
            connection.busy_timeout(std::time::Duration::from_secs(2))?;
            connection.pragma_update(None, "journal_mode", "WAL")?;
            connection.pragma_update(None, "synchronous", "FULL")?;
            connection.pragma_update(None, "foreign_keys", "ON")?;
            migrate(&connection, identity)?;
            connection.execute_batch("CREATE INDEX IF NOT EXISTS executions_by_spec ON executions(spec_hash, change_seq)")?;
            connection.execute_batch("CREATE INDEX IF NOT EXISTS outcomes_by_work ON run_outcomes(json_extract(payload,'$.work_id'), change_seq)")?;
            connection.execute_batch("CREATE INDEX IF NOT EXISTS work_items_by_material_change ON work_items(julianday(json_extract(payload,'$.last_material_change_at')) DESC,id DESC)")?;
            restrict_store_files(&path)?;
            let mut store = Self {
                connection,
                path,
                identity: identity.clone(),
                _writer_lock: writer_lock,
            };
            store.reconcile_interrupted_executions()?;
            store.reconcile_interrupted_workflows()?;
            Ok(store)
        })();
        opened
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn identity(&self) -> &ProjectIdentity {
        &self.identity
    }

    pub fn get_document<T: DeserializeOwned>(
        &self,
        namespace: &str,
        id: &str,
    ) -> Result<Option<Stored<T>>, OperatorStoreError> {
        document_key(namespace, id)?;
        let row: Option<(String, u64, u64)> = self.connection.query_row(
            "SELECT payload,revision,change_seq FROM operator_documents WHERE namespace=?1 AND id=?2 ORDER BY revision DESC LIMIT 1",
            params![namespace, id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).optional()?;
        row.map(|(payload, revision, change_seq)| {
            self.validate_document(&payload)?;
            Ok(Stored {
                value: decode(&payload)?,
                revision,
                change_seq,
            })
        })
        .transpose()
    }

    pub fn put_document<T: Serialize + DeserializeOwned + Clone>(
        &mut self,
        namespace: &str,
        id: &str,
        value: &T,
        expected_revision: Option<u64>,
    ) -> Result<Stored<T>, OperatorStoreError> {
        self.put_document_inner(
            namespace,
            id,
            value,
            expected_revision,
            None,
            false,
            false,
            false,
        )
    }

    pub(crate) fn put_terminal_document<T: Serialize + DeserializeOwned + Clone>(
        &mut self,
        producer: &str,
        producer_id: &str,
        namespace: &str,
        id: &str,
        value: &T,
        expected_revision: Option<u64>,
    ) -> Result<Stored<T>, OperatorStoreError> {
        self.put_document_inner(
            namespace,
            id,
            value,
            expected_revision,
            Some((producer, producer_id)),
            true,
            false,
            false,
        )
    }

    fn put_document_inner<T: Serialize + DeserializeOwned + Clone>(
        &mut self,
        namespace: &str,
        id: &str,
        value: &T,
        expected_revision: Option<u64>,
        terminal: Option<(&str, &str)>,
        finish: bool,
        editorial: bool,
        native_ordinary: bool,
    ) -> Result<Stored<T>, OperatorStoreError> {
        document_key(namespace, id)?;
        let payload = encode(value)?;
        self.validate_document(&payload)?;
        let growth_policy = self.document_growth_policy()?;
        let tx = self.connection.transaction()?;
        if let Some((producer, producer_id)) = terminal {
            let owned: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM terminal_reservations WHERE producer=?1 AND producer_id=?2 AND namespace=?3 AND document_id=?4)",
                params![producer, producer_id, namespace, id], |row| row.get(0),
            )?;
            if !owned {
                return Err(OperatorStoreError::Conflict(
                    "terminal document has no owned reservation",
                ));
            }
        }
        if namespace == "editorial" && growth_policy.is_some() && !editorial {
            return Err(OperatorStoreError::Conflict(
                "editorial policy writes require native owner",
            ));
        }
        if terminal.is_none() && !native_ordinary {
            let owned: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM terminal_reservations WHERE namespace=?1 AND document_id=?2)", params![namespace,id], |r| r.get(0))?;
            if owned {
                return Err(OperatorStoreError::Conflict(
                    "reserved identity requires native producer write",
                ));
            }
        }
        let current: Option<(String, u64, u64)> = tx.query_row(
            "SELECT payload,revision,change_seq FROM operator_documents WHERE namespace=?1 AND id=?2 ORDER BY revision DESC LIMIT 1",
            params![namespace, id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).optional()?;
        if let Some((existing, revision, change_seq)) = &current {
            if expected_revision.is_none() && *existing == payload {
                if finish {
                    finish_reservation_in_tx(&tx, terminal)?;
                }
                tx.commit()?;
                return Ok(Stored {
                    value: value.clone(),
                    revision: *revision,
                    change_seq: *change_seq,
                });
            }
            if expected_revision != Some(*revision) {
                return Err(OperatorStoreError::StaleRevision {
                    current: *revision,
                });
            }
        } else if expected_revision.is_some() {
            return Err(OperatorStoreError::StaleRevision {
                current: 0,
            });
        }
        // A matching CAS retry with no material change allocates neither history nor sequence.
        if let Some((existing, revision, change_seq)) = &current {
            if *existing == payload {
                if finish {
                    finish_reservation_in_tx(&tx, terminal)?;
                }
                tx.commit()?;
                return Ok(Stored {
                    value: value.clone(),
                    revision: *revision,
                    change_seq: *change_seq,
                });
            }
        }
        // Generic documents never consume credit merely by knowing a reserved ID.
        if let Some((producer, producer_id)) = terminal {
            let consumed = tx.execute("UPDATE terminal_reservations SET remaining=remaining-1,consumed=consumed+1 WHERE producer=?1 AND producer_id=?2 AND namespace=?3 AND document_id=?4 AND remaining>0 AND state='reserved'", params![producer,producer_id,namespace,id])?;
            if consumed != 1 {
                return Err(OperatorStoreError::Conflict(
                    "terminal reservation exhausted",
                ));
            }
        }
        let mut replace_aggregate_revision = None;
        if growth_policy
            .as_ref()
            .is_some_and(|p| p.replace_editorial_aggregate)
            && namespace == "editorial"
            && id == "attention-v1"
        {
            if current.is_none() {
                // The first receipt-bearing state is material too, not a mutable aggregate.
                let new: super::editorial::EditorialState = decode(&payload)?;
                if !new.receipts.is_empty() {
                    tx.execute(
                        "UPDATE meta SET value='1' WHERE key='editorial_frozen_revision'",
                        [],
                    )?;
                }
            }
            if let Some((existing, revision, _)) = &current {
                // Typed strict decoding: no unknown state or receipt fields can be erased.
                let old: super::editorial::EditorialState = decode(existing)?;
                let new: super::editorial::EditorialState = decode(&payload)?;
                let same_receipts =
                    serde_json::to_value(&old.receipts)? == serde_json::to_value(&new.receipts)?;
                let frozen: u64 = tx.query_row(
                    "SELECT CAST(value AS INTEGER) FROM meta WHERE key='editorial_frozen_revision'",
                    [],
                    |r| r.get(0),
                )?;
                if same_receipts && *revision > frozen {
                    replace_aggregate_revision = Some(*revision);
                } else if !same_receipts {
                    // Receipt transitions freeze the previous slot. No delivery/feedback
                    // lineage is discarded, even when the admission ceiling is saturated.
                    tx.execute(
                        "UPDATE meta SET value=?1 WHERE key='editorial_frozen_revision'",
                        [revision
                            .checked_add(1)
                            .ok_or(OperatorStoreError::Conflict("document revision exhausted"))?
                            .to_string()],
                    )?;
                }
            }
        }
        if editorial && growth_policy.is_some() && replace_aggregate_revision.is_none() {
            let old = current
                .as_ref()
                .map(|(p, _, _)| decode::<super::editorial::EditorialState>(p))
                .transpose()?;
            let new: super::editorial::EditorialState = decode(&payload)?;
            let before = old.as_ref().map(editorial_obligations).unwrap_or(0);
            let after = editorial_obligations(&new);
            admit_document_charge(
                &tx,
                growth_policy.as_ref(),
                (1 + after).saturating_sub(before),
            )?;
        }
        if terminal.is_none() && !editorial && replace_aggregate_revision.is_none() {
            admit_document_charge(&tx, growth_policy.as_ref(), 1)?;
            if let Some(policy) = &growth_policy {
                let full: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM operator_documents WHERE namespace=?1 LIMIT 1 OFFSET ?2)", params![namespace, policy.max_rows_per_namespace - 1], |r| r.get(0))?;
                if full {
                    return Err(OperatorStoreError::Conflict(
                        "document namespace admission capacity reached",
                    ));
                }
            }
        }
        let revision = current.map_or(Ok(1), |(_, revision, _)| {
            revision
                .checked_add(1)
                .ok_or(OperatorStoreError::Conflict("document revision exhausted"))
        })?;
        let change_seq = next_seq(&tx)? as u64;
        if let Some(previous_revision) = replace_aggregate_revision {
            tx.execute("UPDATE operator_documents SET revision=?3,payload=?4,change_seq=?5 WHERE namespace=?1 AND id=?2 AND revision=?6", params![namespace,id,revision,payload,change_seq,previous_revision])?;
        } else {
            tx.execute(
                "INSERT INTO operator_documents(namespace,id,revision,payload,change_seq) VALUES(?1,?2,?3,?4,?5)",
                params![namespace, id, revision, payload, change_seq],
            )?;
        }
        if finish {
            finish_reservation_in_tx(&tx, terminal)?;
        }
        tx.commit()?;
        Ok(Stored {
            value: value.clone(),
            revision,
            change_seq,
        })
    }

    pub fn list_documents<T: DeserializeOwned>(
        &self,
        namespace: &str,
        after: Option<u64>,
        limit: usize,
    ) -> Result<Page<Stored<T>>, OperatorStoreError> {
        document_key(namespace, "list")?;
        bounded(limit)?;
        let mut statement = self.connection.prepare(
            "SELECT d.payload,d.revision,d.change_seq FROM operator_documents d WHERE d.namespace=?1 AND d.change_seq>?2 AND NOT EXISTS(SELECT 1 FROM operator_documents newer WHERE newer.namespace=d.namespace AND newer.id=d.id AND newer.revision>d.revision) ORDER BY d.change_seq LIMIT ?3",
        )?;
        let rows =
            statement.query_map(params![namespace, after.unwrap_or(0), limit + 1], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, u64>(1)?,
                    row.get::<_, u64>(2)?,
                ))
            })?;
        let mut entries = Vec::new();
        for row in rows {
            let (payload, revision, change_seq) = row?;
            self.validate_document(&payload)?;
            entries.push(Stored {
                value: decode(&payload)?,
                revision,
                change_seq,
            });
        }
        let next_after = if entries.len() > limit {
            entries.pop();
            entries.last().map(|entry| entry.change_seq)
        } else {
            None
        };
        Ok(Page {
            entries,
            next_after,
        })
    }

    /// Latest retained receipt identities by source creation time, not import/change
    /// sequence. Select only the current revision; undo/reimport must not duplicate
    /// a receipt or make an old receipt look newly created. Public cursor paging
    /// remains change-sequence based in list_documents.
    pub(crate) fn latest_receipt_documents<T: DeserializeOwned>(
        &self,
        limit: usize,
    ) -> Result<Vec<Stored<T>>, OperatorStoreError> {
        bounded(limit)?;
        let mut statement = self.connection.prepare(
            "SELECT d.payload,d.revision,d.change_seq FROM operator_documents d \
             WHERE d.namespace='legacy_receipt' AND NOT EXISTS(\
             SELECT 1 FROM operator_documents newer WHERE newer.namespace=d.namespace \
             AND newer.id=d.id AND newer.revision>d.revision) \
             ORDER BY julianday(json_extract(d.payload,'$.value.created_at')) DESC, \
             json_extract(d.payload,'$.value.id') DESC LIMIT ?1",
        )?;
        let rows = statement.query_map(params![limit], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u64>(1)?,
                row.get::<_, u64>(2)?,
            ))
        })?;
        let mut entries = Vec::new();
        for row in rows {
            let (payload, revision, change_seq) = row?;
            self.validate_document(&payload)?;
            entries.push(Stored {
                value: decode(&payload)?,
                revision,
                change_seq,
            });
        }
        Ok(entries)
    }

    fn validate_document(&self, payload: &str) -> Result<(), OperatorStoreError> {
        let document: serde_json::Value = decode(payload)?;
        if document
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            != Some(1)
        {
            return Err(OperatorStoreError::InvalidInput(
                "unsupported document schema version",
            ));
        }
        let project: ProjectIdentity =
            serde_json::from_value(document.get("project").cloned().ok_or(
                OperatorStoreError::InvalidInput("document has no project identity"),
            )?)?;
        if project != self.identity {
            return Err(OperatorStoreError::ProjectMismatch);
        }
        Ok(())
    }

    pub fn get_execution_by_idempotency_key(
        &self,
        key: &str,
    ) -> Result<Option<Stored<ActionExecution>>, OperatorStoreError> {
        let id: Option<String> = self
            .connection
            .query_row(
                "SELECT execution_id FROM executions WHERE idempotency_key=?1",
                [key],
                |row| row.get(0),
            )
            .optional()?;
        id.map(|id| self.get_execution(&id))
            .transpose()
            .map(Option::flatten)
    }

    pub fn backup(&self) -> Result<PathBuf, OperatorStoreError> {
        let directory = private_child_directory(
            self.path.parent().ok_or(OperatorStoreError::InvalidInput(
                "operator database has no parent",
            ))?,
            "backups",
        )?;
        let destination = directory.join(format!("operator-{}.sqlite", uuid::Uuid::new_v4()));
        let mut options = fs::OpenOptions::new();
        options.create_new(true).read(true).write(true);
        #[cfg(unix)]
        options.mode(0o600);
        let file = options.open(&destination)?;
        restrict_file(&file)?;
        drop(file);
        let result = (|| {
            self.connection
                .execute("VACUUM INTO ?1", [destination.to_string_lossy().as_ref()])?;
            let backup = Connection::open_with_flags(
                &destination,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )?;
            let integrity: String =
                backup.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
            if integrity != "ok" {
                return Err(OperatorStoreError::CorruptSchema(
                    "backup integrity check failed",
                ));
            }
            let binding: String = backup.query_row(
                "SELECT value FROM meta WHERE key='project_identity'",
                [],
                |row| row.get(0),
            )?;
            if binding != encode(&self.identity)? {
                return Err(OperatorStoreError::ProjectMismatch);
            }
            drop(backup);
            fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&destination)?
                .sync_all()?;
            #[cfg(unix)]
            fs::File::open(&directory)?.sync_all()?;
            Ok(destination.clone())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&destination);
        }
        result
    }

    fn reconcile_interrupted_executions(&mut self) -> Result<(), OperatorStoreError> {
        let mut after = 0u64;
        loop {
            let batch: Vec<(String, u64, u64)> = {
                let mut statement = self.connection.prepare(
                    "SELECT payload,revision,change_seq FROM executions WHERE change_seq>?1 AND json_extract(payload,'$.status') IN ('claimed','running','reconciling') ORDER BY change_seq LIMIT 100",
                )?;
                let rows = statement
                    .query_map([after], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
                rows.collect::<Result<_, _>>()?
            };
            if batch.is_empty() {
                break;
            }
            for (payload, revision, seq) in batch {
                after = seq;
                let mut execution: ActionExecution = decode(&payload)?;
                if !matches!(
                    execution.status,
                    ActionExecutionState::Claimed
                        | ActionExecutionState::Running
                        | ActionExecutionState::Reconciling
                ) {
                    continue;
                }
                execution.status = ActionExecutionState::OutcomeUnknown;
                execution.verification_evidence.push("The engine restarted before a durable terminal result. Effects were not replayed; reconcile before retrying.".into());
                self.replace_execution(&execution, revision)?;
            }
        }
        Ok(())
    }

    pub fn begin_workflow(
        &mut self,
        start: &super::outcome_runtime::WorkflowOutcomeStart,
    ) -> Result<(), OperatorStoreError> {
        if self.active_run(&start.run_id)?.is_none()
            && self.get_run_outcome(&start.run_id)?.is_none()
        {
            self.admit_new_work()?;
        }
        let active = ActiveRun {
            start: start.clone(),
            checkpoint: super::outcome_runtime::WorkflowOutcomeCheckpoint::from_evidence([]),
        };
        self.validate_active(&active)?;
        let payload = encode(&active)?;
        let tx = self.connection.transaction()?;
        if stored_immutable_in_tx::<RunOutcome>(
            &tx,
            "run_outcomes",
            "run_id=?1",
            [&start.run_id],
            0,
        )?
        .is_some()
        {
            return Err(OperatorStoreError::Conflict("run is already terminal"));
        }
        let existing: Option<String> = tx
            .query_row(
                "SELECT payload FROM active_runs WHERE run_id=?1",
                [&start.run_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(existing) = existing {
            if decode::<ActiveRun>(&existing)?.start != *start {
                return Err(OperatorStoreError::Conflict(
                    "active run identity cannot change",
                ));
            }
            return Ok(());
        }
        let count: usize =
            tx.query_row("SELECT count(*) FROM active_runs", [], |row| row.get(0))?;
        if count >= MAX_ACTIVE_RUNS {
            return Err(OperatorStoreError::Conflict("active run capacity reached"));
        }
        tx.execute(
            "INSERT INTO active_runs(run_id,payload) VALUES(?1,?2)",
            params![start.run_id, payload],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn checkpoint_workflow(
        &mut self,
        start: &super::outcome_runtime::WorkflowOutcomeStart,
        checkpoint: super::outcome_runtime::WorkflowOutcomeCheckpoint,
    ) -> Result<(), OperatorStoreError> {
        let mut active = self
            .active_run(&start.run_id)?
            .ok_or(OperatorStoreError::Conflict(
                "run has not begun or is already terminal",
            ))?;
        if active.start != *start {
            return Err(OperatorStoreError::Conflict(
                "active run identity cannot change",
            ));
        }
        active.checkpoint.evidence = super::outcome_runtime::merge_checkpoint_evidence(
            active.checkpoint.evidence,
            checkpoint.evidence,
        )?;
        self.validate_active(&active)?;
        let payload = encode(&active)?;
        let tx = self.connection.transaction()?;
        tx.execute(
            "UPDATE active_runs SET payload=?1 WHERE run_id=?2",
            params![payload, start.run_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    fn active_run(&self, run_id: &str) -> Result<Option<ActiveRun>, OperatorStoreError> {
        let payload: Option<String> = self
            .connection
            .query_row(
                "SELECT payload FROM active_runs WHERE run_id=?1",
                [run_id],
                |row| row.get(0),
            )
            .optional()?;
        payload
            .map(|payload| {
                let active: ActiveRun = decode(&payload)?;
                self.validate_active(&active)?;
                Ok(active)
            })
            .transpose()
    }

    fn validate_active(&self, active: &ActiveRun) -> Result<(), OperatorStoreError> {
        let outcome = super::outcome_runtime::build_workflow_outcome(
            active.start.clone(),
            active.checkpoint.clone(),
            super::outcome_runtime::WorkflowTerminal::Inconclusive,
            active.start.started_at.clone(),
        );
        self.validate_for_project(&outcome, &outcome.project)?;
        if active.checkpoint.evidence.len() > super::outcome_runtime::MAX_OUTCOME_EVIDENCE {
            return Err(OperatorStoreError::InvalidInput(
                "too much active run evidence",
            ));
        }
        super::work_items::SubjectKey::new(
            &active.start.project,
            "workflow",
            &active.start.workflow,
            &active.start.coverage,
            &active.start.input_fingerprint,
        )
        .map_err(|_| OperatorStoreError::InvalidInput("invalid active run subject"))?;
        encode(active)?;
        Ok(())
    }

    fn reconcile_interrupted_workflows(&mut self) -> Result<(), OperatorStoreError> {
        let ids: Vec<String> = {
            let mut statement = self
                .connection
                .prepare("SELECT run_id FROM active_runs ORDER BY run_id LIMIT ?1")?;
            let rows = statement.query_map([MAX_ACTIVE_RUNS + 1], |row| row.get(0))?;
            rows.collect::<Result<_, _>>()?
        };
        if ids.len() > MAX_ACTIVE_RUNS {
            return Err(OperatorStoreError::CorruptSchema(
                "active run capacity exceeded",
            ));
        }
        for id in ids {
            let active = self
                .active_run(&id)?
                .ok_or(OperatorStoreError::CorruptSchema("active run disappeared"))?;
            if self.get_run_outcome(&id)?.is_some() {
                return Err(OperatorStoreError::CorruptSchema(
                    "run is both active and terminal",
                ));
            }
            let terminal = if active.checkpoint.evidence.is_empty() {
                super::outcome_runtime::WorkflowTerminal::Blocked {
                    failure_class: Some("engine_interrupted".into()),
                }
            } else {
                super::outcome_runtime::WorkflowTerminal::Partial {
                    failure_class: Some("engine_interrupted".into()),
                }
            };
            let subject = super::work_items::SubjectKey::new(
                &active.start.project,
                "workflow",
                &active.start.workflow,
                &active.start.coverage,
                &active.start.input_fingerprint,
            )
            .map_err(|_| OperatorStoreError::InvalidInput("invalid active run subject"))?;
            let title = super::workflow_display::workflow_interrupted_title(&active.start.workflow);
            let mut outcome = super::outcome_runtime::build_workflow_outcome(
                active.start,
                active.checkpoint,
                terminal,
                chrono::Utc::now().to_rfc3339(),
            );
            outcome.work_id = subject.work_item_id();
            super::outcome_runtime::publish_report_in_store(self, outcome, subject, title,
                "The engine restarted before this run durably published a terminal result. Only checkpointed evidence is retained. Completion and any effects after the last checkpoint are unknown. No effects were replayed; inspect the native sources before retrying.".into())
                .map_err(|_| OperatorStoreError::Conflict("interrupted run publication failed; active run retained"))?;
        }
        Ok(())
    }

    pub(crate) fn retain_run_checkpoint(
        &self,
        outcome: &mut RunOutcome,
    ) -> Result<(), OperatorStoreError> {
        if let Some(active) = self.active_run(&outcome.run_id)? {
            let start = active.start;
            let subject = super::work_items::SubjectKey::new(
                &start.project,
                "workflow",
                &start.workflow,
                &start.coverage,
                &start.input_fingerprint,
            )
            .map_err(|_| OperatorStoreError::InvalidInput("invalid active run subject"))?;
            if outcome.project != start.project
                || outcome.worker != start.worker
                || outcome.parent_run_id != start.parent_run_id
                || outcome.origin != start.origin
                || outcome.workflow.as_deref() != Some(start.workflow.as_str())
                || outcome.started_at != start.started_at
                || outcome.coverage != start.coverage
                || outcome.input_fingerprint != start.input_fingerprint
                || (outcome.work_id != start.work_id && outcome.work_id != subject.work_item_id())
            {
                return Err(OperatorStoreError::Conflict(
                    "terminal run identity differs from begun run",
                ));
            }
            outcome.evidence = super::outcome_runtime::merge_checkpoint_evidence(
                active.checkpoint.evidence,
                outcome.evidence.clone(),
            )?;
        }
        Ok(())
    }

    pub fn append_run_outcome(
        &mut self,
        outcome: &RunOutcome,
    ) -> Result<Stored<RunOutcome>, OperatorStoreError> {
        self.validate_for_project(outcome, &outcome.project)?;
        if self.active_run(&outcome.run_id)?.is_some() {
            return Err(OperatorStoreError::Conflict(
                "begun runs require atomic report publication",
            ));
        }
        if self.get_run_outcome(&outcome.run_id)?.is_none() {
            self.admit_new_work()?;
        }
        self.append_immutable("run_outcomes", "run_id", &outcome.run_id, outcome)
    }
    pub fn get_run_outcome(
        &self,
        run_id: &str,
    ) -> Result<Option<Stored<RunOutcome>>, OperatorStoreError> {
        self.get_immutable("run_outcomes", "run_id", run_id)
    }
    pub fn list_work_outcomes(
        &self,
        work_id: &str,
        after: Option<u64>,
        limit: usize,
    ) -> Result<Page<Stored<RunOutcome>>, OperatorStoreError> {
        bounded(limit)?;
        if self.get_work_item(work_id)?.is_none() {
            return Err(OperatorStoreError::InvalidInput("work item not found"));
        }
        let mut statement = self.connection.prepare(
            "SELECT payload,0,change_seq FROM run_outcomes WHERE json_extract(payload,'$.work_id')=?1 AND change_seq>?2 ORDER BY change_seq LIMIT ?3",
        )?;
        let rows = statement.query_map(
            params![work_id, after.unwrap_or(0), (limit + 1) as i64],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        page(rows, limit)
    }

    pub fn artifact_for_run(
        &self,
        artifact_id: &str,
        run: &Stored<RunOutcome>,
    ) -> Result<Option<Stored<ArtifactRevision>>, OperatorStoreError> {
        self.validate_for_project(&run.value, &run.value.project)?;
        let revision: Option<u32> = self.connection.query_row(
            "SELECT revision FROM artifacts a WHERE artifact_id=?1 AND (
                EXISTS(SELECT 1 FROM json_each(a.payload,'$.source_run_ids') WHERE value=?2) OR change_seq<=?3)
             ORDER BY EXISTS(SELECT 1 FROM json_each(a.payload,'$.source_run_ids') WHERE value=?2) DESC, revision DESC LIMIT 1",
            params![artifact_id, run.value.run_id, run.change_seq], |row| row.get(0),
        ).optional()?;
        revision
            .map(|revision| self.get_artifact_revision(artifact_id, Some(revision)))
            .transpose()
            .map(Option::flatten)
    }

    /// Three relational records, charged before any supervised worker effect.
    /// Spent units remain charged because these records are outside documents.
    pub(crate) fn reserve_supervision_completion(
        &mut self,
        id: &str,
    ) -> Result<(), OperatorStoreError> {
        if self.require_supervision_completion(id).is_ok() {
            return Ok(());
        }
        self.admit_new_work()?;
        self.reserve_native_documents(
            "supervision_completion",
            id,
            &[("supervision_report", id.to_owned(), 3)],
        )
    }

    pub(crate) fn require_supervision_completion(
        &self,
        id: &str,
    ) -> Result<(), OperatorStoreError> {
        let reserved: bool = self.connection.query_row("SELECT EXISTS(SELECT 1 FROM terminal_reservations WHERE producer='supervision_completion' AND producer_id=?1 AND namespace='supervision_report' AND document_id=?1 AND budget=3 AND remaining=3 AND consumed=0 AND state='reserved')", [id], |row| row.get(0))?;
        if !reserved {
            return Err(OperatorStoreError::Conflict(
                "pre-work supervision completion admission missing",
            ));
        }
        Ok(())
    }

    pub(crate) fn publish_supervision_report(
        &mut self,
        link: &super::supervision::SupervisionLink,
    ) -> Result<String, OperatorStoreError> {
        let saved = self
            .get_document::<super::supervision::SupervisionLink>(
                super::supervision::LINKS,
                &link.mandate_id,
            )?
            .ok_or(OperatorStoreError::Conflict("supervision link missing"))?;
        let mandate = self
            .get_document::<super::supervision_authority::MandateRecord>(
                super::supervision_authority::MANDATES,
                &link.mandate_id,
            )?
            .ok_or(OperatorStoreError::Conflict("supervision mandate missing"))?;
        if encode(&saved.value)? != encode(link)?
            || link.schema_version != 1
            || link.project != mandate.value.project
            || !(link.owner_epoch == mandate.value.mandate.owner_epoch
                || (mandate.value.mandate.revoked_at.is_some()
                    && link.owner_epoch.checked_add(1) == Some(mandate.value.mandate.owner_epoch)))
            || mandate.value.mandate.mandate_id != link.mandate_id
            || !link.dispatch_attempted
            || link.worker_agent_id.is_none()
        {
            return Err(OperatorStoreError::Conflict(
                "supervision completion identity changed",
            ));
        }
        let engineering = super::supervision_authority::is_engineering(&mandate.value);
        if engineering {
            let proof = link
                .engineering_proof
                .as_ref()
                .ok_or(OperatorStoreError::Conflict(
                    "engineering delivery requires revision-bound native source proof",
                ))?;
            let targets = super::supervision_authority::worker_target_files(&mandate.value)
                .map_err(|_| OperatorStoreError::Conflict("engineering target grant invalid"))?;
            if proof.project != mandate.value.project
                || proof.worker != mandate.value.worker
                || proof.source.source_workspace_root != mandate.value.project.canonical_root
                || proof.source.target_files != targets
                || proof.owner_epoch != link.owner_epoch
            {
                return Err(OperatorStoreError::Conflict(
                    "engineering proof scope changed",
                ));
            }
            super::supervision_proof::validate_retained(
                proof,
                &mandate.value,
                link.child_chat_id
                    .as_deref()
                    .ok_or(OperatorStoreError::Conflict("native child unavailable"))?,
                link.delivery_proof
                    .as_ref()
                    .ok_or(OperatorStoreError::Conflict(
                        "native goal proof unavailable",
                    ))?,
            )
            .map_err(|_| OperatorStoreError::Conflict("engineering proof invalid"))?;
        } else if link.engineering_proof.is_some() {
            return Err(OperatorStoreError::Conflict(
                "read-only mandate has engineering proof",
            ));
        }
        // Use the admitted worker identity, not a restarted runtime epoch.
        let (mut start, evidence, report) =
            super::supervision::delivery_report(link, mandate.value.worker)
                .map_err(|_| OperatorStoreError::Conflict("supervision proof invalid"))?;
        let subject = super::work_items::SubjectKey::new(
            &start.project,
            "workflow",
            &start.workflow,
            &start.coverage,
            &start.input_fingerprint,
        )
        .map_err(|_| OperatorStoreError::Conflict("supervision subject invalid"))?;
        start.work_id = subject.work_item_id();
        let run_id = start.run_id.clone();
        let ended_at = start.started_at.clone();
        let mut evidence = vec![evidence];
        if let Some(proof) = &link.engineering_proof {
            evidence.push(refact_buddy_core::operator_types::EvidenceRef {
                evidence_id: format!("{run_id}:source"),
                source: refact_buddy_core::operator_types::NativeSourceIdentity {
                    source_kind: refact_buddy_core::operator_types::NativeSourceKind::Check,
                    source_id: format!("{}:{}", proof.child_chat_id, proof.verifier_attempt_at_ms),
                    source_version: proof.source.content_digest.clone(),
                },
                locator: format!(
                    "worktree://{}@{}",
                    proof.source.worktree_id, proof.source.current_head
                ),
                observed_at: proof.observed_after_at.clone(),
                digest: Some(format!("sha256:{}", proof.source.content_digest)),
            });
        }
        let outcome = super::outcome_runtime::build_workflow_outcome(
            start,
            super::outcome_runtime::WorkflowOutcomeCheckpoint::from_evidence(evidence),
            super::outcome_runtime::WorkflowTerminal::Completed,
            ended_at,
        );
        let token = SupervisionCompletion {
            mandate_id: link.mandate_id.clone(),
            outcome: outcome.clone(),
        };
        super::outcome_runtime::publish_report_in_store_owned(
            self,
            outcome,
            subject,
            if engineering {
                "Verified isolated engineering result"
            } else {
                "Verified bounded analysis result"
            }
            .into(),
            super::jobs::autonomous_chats::redact_and_cap_text(&report, 8_000),
            Some(&token),
        )
        .map_err(|_| OperatorStoreError::Conflict("supervision report publication failed"))?;
        Ok(run_id)
    }

    pub fn append_outcome_work_item_artifact(
        &mut self,
        outcome: &RunOutcome,
        item: &WorkItem,
        expected_work_revision: Option<u64>,
        artifact: Option<&ArtifactRevision>,
    ) -> Result<
        (
            Stored<RunOutcome>,
            Stored<WorkItem>,
            Option<Stored<ArtifactRevision>>,
        ),
        OperatorStoreError,
    > {
        self.append_outcome_work_item_artifact_owned(
            outcome,
            item,
            expected_work_revision,
            artifact,
            None,
        )
    }

    pub(crate) fn append_outcome_work_item_artifact_owned(
        &mut self,
        outcome: &RunOutcome,
        item: &WorkItem,
        expected_work_revision: Option<u64>,
        artifact: Option<&ArtifactRevision>,
        completion: Option<&SupervisionCompletion>,
    ) -> Result<
        (
            Stored<RunOutcome>,
            Stored<WorkItem>,
            Option<Stored<ArtifactRevision>>,
        ),
        OperatorStoreError,
    > {
        self.validate_for_project(outcome, &outcome.project)?;
        self.validate_for_project(item, &item.project)?;
        if outcome.work_id != item.id {
            return Err(OperatorStoreError::Conflict(
                "outcome must be linked to the supplied work item",
            ));
        }
        if let Some(artifact) = artifact {
            self.validate_for_project(artifact, &artifact.project)?;
            if artifact.revision == 0 {
                return Err(OperatorStoreError::InvalidInput(
                    "artifact revision must be positive",
                ));
            }
            if item.current_artifact_id.as_deref() != Some(&artifact.artifact_id)
                || item.current_artifact_revision != Some(artifact.revision)
            {
                return Err(OperatorStoreError::Conflict(
                    "work item current artifact does not match artifact revision",
                ));
            }
            if !artifact
                .source_run_ids
                .iter()
                .any(|run_id| run_id == &outcome.run_id)
            {
                return Err(OperatorStoreError::Conflict(
                    "artifact must include the outcome run as a source",
                ));
            }
        }

        let mut retained = outcome.clone();
        self.retain_run_checkpoint(&mut retained)?;
        if retained != *outcome {
            return Err(OperatorStoreError::Conflict(
                "terminal outcome must retain checkpoint evidence",
            ));
        }
        if completion.is_none()
            && self.active_run(&outcome.run_id)?.is_none()
            && self.get_run_outcome(&outcome.run_id)?.is_none()
        {
            self.admit_new_work()?;
        }
        let outcome_payload = encode(outcome)?;
        let artifact_payload = artifact.map(encode).transpose()?;
        let tx = self.connection.transaction()?;

        if let Some(existing) = stored_immutable_in_tx::<RunOutcome>(
            &tx,
            "run_outcomes",
            "run_id = ?1",
            params![outcome.run_id],
            0,
        )? {
            if encode(&existing.value)? != outcome_payload {
                return Err(OperatorStoreError::Conflict(
                    "immutable record id already exists with different payload",
                ));
            }
            let work = stored_mutable_in_tx::<WorkItem>(&tx, "work_items", &item.id)?.ok_or(
                OperatorStoreError::Conflict("existing outcome has no linked work item"),
            )?;
            let mut expected_item = item.clone();
            if item.subject.resource_kind == "workflow"
                && work.value.subject == item.subject
                && work.value.state == refact_buddy_core::operator_types::WorkItemState::Cancelled
            {
                expected_item.state = refact_buddy_core::operator_types::WorkItemState::Cancelled;
            }
            if work.value != expected_item {
                return Err(OperatorStoreError::Conflict(
                    "existing outcome was completed against a different work item revision",
                ));
            }
            let artifact = match artifact {
                Some(artifact) => {
                    let stored = stored_artifact_in_tx(&tx, artifact)?;
                    if stored.value != *artifact {
                        return Err(OperatorStoreError::Conflict(
                            "immutable record id already exists with different payload",
                        ));
                    }
                    Some(stored)
                }
                None => None,
            };
            return Ok((existing, work, artifact));
        }

        if let Some(completion) = completion {
            if completion.outcome != *outcome {
                return Err(OperatorStoreError::Conflict(
                    "supervision completion payload changed",
                ));
            }
            let changed = tx.execute("UPDATE terminal_reservations SET remaining=0,consumed=3,state='finished' WHERE producer='supervision_completion' AND producer_id=?1 AND namespace='supervision_report' AND document_id=?1 AND budget=3 AND remaining=3 AND consumed=0 AND state='reserved'", [&completion.mandate_id])?;
            if changed != 1 {
                return Err(OperatorStoreError::Conflict(
                    "supervision completion admission missing or spent",
                ));
            }
        }

        let mut published_item = item.clone();
        retire_workflow_siblings_in_tx(&tx, outcome, &mut published_item)?;
        let item_payload = encode(&published_item)?;

        let work =
            upsert_work_item_in_tx(&tx, &published_item, &item_payload, expected_work_revision)?;
        let outcome = insert_outcome_in_tx(&tx, outcome, &outcome_payload)?;
        let artifact = match (artifact, artifact_payload.as_deref()) {
            (Some(artifact), Some(payload)) => Some(insert_artifact_in_tx(&tx, artifact, payload)?),
            (None, None) => None,
            _ => unreachable!("artifact payload is encoded with artifact"),
        };
        tx.execute(
            "DELETE FROM active_runs WHERE run_id=?1",
            [&outcome.value.run_id],
        )?;
        tx.commit()?;
        Ok((outcome, work, artifact))
    }
    pub fn append_artifact_revision(
        &mut self,
        artifact: &ArtifactRevision,
    ) -> Result<Stored<ArtifactRevision>, OperatorStoreError> {
        self.validate_for_project(artifact, &artifact.project)?;
        if artifact.revision == 0 {
            return Err(OperatorStoreError::InvalidInput(
                "artifact revision must be positive",
            ));
        }
        let payload = encode(artifact)?;
        let tx = self.connection.transaction()?;
        let seq = next_seq(&tx)?;
        let affected = tx.execute("INSERT OR IGNORE INTO artifacts(artifact_id, revision, payload, change_seq) VALUES (?1, ?2, ?3, ?4)", params![artifact.artifact_id, artifact.revision, payload, seq])?;
        if affected == 0 {
            return immutable_existing(
                &tx,
                "artifacts",
                "artifact_id = ?1 AND revision = ?2",
                params![artifact.artifact_id, artifact.revision],
                artifact,
                artifact.revision,
            );
        }
        tx.commit()?;
        Ok(Stored {
            value: artifact.clone(),
            revision: artifact.revision as u64,
            change_seq: seq as u64,
        })
    }
    pub fn list_artifact_revisions(
        &self,
        artifact_id: &str,
        after: Option<u64>,
        limit: usize,
    ) -> Result<Page<Stored<ArtifactRevision>>, OperatorStoreError> {
        bounded(limit)?;
        let mut stmt = self.connection.prepare("SELECT payload, revision, change_seq FROM artifacts WHERE artifact_id=?1 AND change_seq>?2 ORDER BY change_seq LIMIT ?3")?;
        let rows = stmt.query_map(
            params![artifact_id, after.unwrap_or(0), (limit + 1) as i64],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            },
        )?;
        page(rows, limit)
    }
    pub fn get_artifact_revision(
        &self,
        id: &str,
        revision: Option<u32>,
    ) -> Result<Option<Stored<ArtifactRevision>>, OperatorStoreError> {
        let row: Option<(String, i64, i64)> = match revision {
            Some(revision) => self.connection.query_row(
                "SELECT payload, revision, change_seq FROM artifacts WHERE artifact_id=?1 AND revision=?2",
                params![id, revision],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            ).optional()?,
            None => self.connection.query_row(
                "SELECT payload, revision, change_seq FROM artifacts WHERE artifact_id=?1 ORDER BY revision DESC LIMIT 1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            ).optional()?,
        };
        row.map(|(payload, revision, change_seq)| {
            Ok(Stored {
                value: decode(&payload)?,
                revision: revision as u64,
                change_seq: change_seq as u64,
            })
        })
        .transpose()
    }

    pub fn upsert_work_item(
        &mut self,
        item: &WorkItem,
        expected_revision: Option<u64>,
    ) -> Result<Stored<WorkItem>, OperatorStoreError> {
        self.validate_for_project(item, &item.project)?;
        let payload = encode(item)?;
        let tx = self.connection.transaction()?;
        let current: Option<i64> = tx
            .query_row(
                "SELECT revision FROM work_items WHERE id=?1",
                [&item.id],
                |r| r.get(0),
            )
            .optional()?;
        if current.is_some() && expected_revision.is_none() {
            return Err(OperatorStoreError::Conflict(
                "updating a work item requires an expected revision",
            ));
        }
        if let Some(expected) = expected_revision {
            if current.map(|v| v as u64) != Some(expected) {
                return Err(OperatorStoreError::StaleRevision {
                    current: current.unwrap_or(0) as u64,
                });
            }
        }
        if current.is_none() && expected_revision.is_some() {
            return Err(OperatorStoreError::StaleRevision {
                current: 0,
            });
        }
        let revision = current.unwrap_or(0) + 1;
        let seq = next_seq(&tx)?;
        tx.execute("INSERT INTO work_items(id,payload,revision,change_seq) VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload,revision=excluded.revision,change_seq=excluded.change_seq", params![item.id, payload, revision, seq])?;
        tx.commit()?;
        Ok(Stored {
            value: item.clone(),
            revision: revision as u64,
            change_seq: seq as u64,
        })
    }
    pub fn get_work_item(&self, id: &str) -> Result<Option<Stored<WorkItem>>, OperatorStoreError> {
        self.get_mutable("work_items", id)
    }

    pub fn trajectory_is_referenced(&self, id: &str) -> Result<bool, OperatorStoreError> {
        crate::chat::trajectories::validate_trajectory_id(id)
            .map_err(|_| OperatorStoreError::InvalidInput("invalid trajectory identity"))?;
        let mut statement = self.connection.prepare(
            "SELECT payload FROM work_items UNION ALL SELECT payload FROM run_outcomes
             UNION ALL SELECT payload FROM artifacts UNION ALL SELECT payload FROM executions
             UNION ALL SELECT payload FROM active_runs UNION ALL SELECT payload FROM operator_documents
             UNION ALL SELECT payload FROM read_states UNION ALL SELECT payload FROM work_snoozes",
        )?;
        let mut rows = statement.query([])?;
        let mut scanned = 0usize;
        while let Some(row) = rows.next()? {
            scanned += 1;
            if scanned > 100_000 {
                return Err(OperatorStoreError::Conflict(
                    "reference scan capacity exceeded; archive refused",
                ));
            }
            let payload: String = row.get(0)?;
            let value: serde_json::Value = decode(&payload)?;
            if contains_trajectory_reference(&value, id) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(crate) fn worktree_is_referenced(
        &self,
        id: &str,
        root: &str,
        branch: Option<&str>,
    ) -> Result<bool, OperatorStoreError> {
        if id.is_empty() || root.is_empty() || branch == Some("") {
            return Err(OperatorStoreError::InvalidInput(
                "invalid worktree identity",
            ));
        }
        let housekeeping_spec = |hash: &str| -> Result<bool, OperatorStoreError> {
            let Some(document) = self.get_document::<super::resource_execution::ResourceDocument<
                super::action_execution::PreparedAction,
            >>("prepared_action", hash)?
            else {
                return Ok(false);
            };
            let prepared = document.value.payload;
            Ok(prepared.spec_hash == hash
                && super::action_execution::action_spec_hash(
                    &prepared.spec,
                    &prepared.bound_parameter_ref,
                ) == hash
                && prepared.spec.project == self.identity
                && prepared.spec.adapter == "worktree_housekeeping"
                && prepared.spec.kind == "archive"
                && prepared.spec.resource.resource_kind == "registered_worktree"
                && prepared.spec.resource.resource_id == id)
        };
        let mut statement = self.connection.prepare(
            "SELECT 'work',id,payload FROM work_items
             UNION ALL SELECT 'outcome',run_id,payload FROM run_outcomes
             UNION ALL SELECT 'artifact',artifact_id,payload FROM artifacts
             UNION ALL SELECT 'execution',execution_id,payload FROM executions
             UNION ALL SELECT 'active',run_id,payload FROM active_runs
             UNION ALL SELECT namespace,id,payload FROM operator_documents
             UNION ALL SELECT 'read',work_id,payload FROM read_states
             UNION ALL SELECT 'snooze',work_id,payload FROM work_snoozes",
        )?;
        let mut rows = statement.query([])?;
        let mut scanned = 0usize;
        while let Some(row) = rows.next()? {
            scanned += 1;
            if scanned > 100_000 {
                return Err(OperatorStoreError::Conflict(
                    "reference scan capacity exceeded; archive refused",
                ));
            }
            let namespace: String = row.get(0)?;
            let key: String = row.get(1)?;
            let payload: String = row.get(2)?;
            let mut value: serde_json::Value = decode(&payload)?;
            if matches!(namespace.as_str(), "resource_draft" | "prepared_draft") {
                if let Ok(document) = serde_json::from_value::<
                    super::resource_execution::ResourceDocument<
                        super::resources::worktree_housekeeping::WorktreeHousekeepingDraft,
                    >,
                >(value.clone())
                {
                    if document.schema_version == 1
                        && document.project == self.identity
                        && document.payload.project == self.identity
                        && document.payload.worktree_id == id
                    {
                        value["payload"]["worktree_id"] = serde_json::Value::Null;
                    }
                }
            } else if namespace == "prepared_action" && housekeeping_spec(&key)? {
                value["payload"]["spec"]["resource"]["resource_id"] = serde_json::Value::Null;
            } else if matches!(namespace.as_str(), "execution" | "resource_receipt") {
                let execution = if namespace == "execution" {
                    Some(decode::<ActionExecution>(&payload)?)
                } else {
                    self.get_execution(&key)?.map(|record| record.value)
                };
                if let Some(execution) = execution {
                    if housekeeping_spec(&execution.spec_hash)? {
                        if namespace == "resource_receipt" {
                            if value
                                .pointer("/payload/resource_id")
                                .and_then(|v| v.as_str())
                                == Some(id)
                            {
                                value["payload"]["resource_id"] = serde_json::Value::Null;
                            }
                        } else if let Some(effects) =
                            value.get_mut("effects").and_then(|v| v.as_array_mut())
                        {
                            for effect in effects {
                                if effect
                                    .pointer("/resource/resource_id")
                                    .and_then(|v| v.as_str())
                                    == Some(id)
                                {
                                    effect["resource"]["resource_id"] = serde_json::Value::Null;
                                }
                            }
                        }
                    }
                }
            }
            if contains_worktree_reference(&value, id, root, branch) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub fn get_read_state(&self, id: &str) -> Result<Option<ReadState>, OperatorStoreError> {
        let payload: Option<String> = self
            .connection
            .query_row(
                "SELECT payload FROM read_states WHERE work_id=?1",
                [id],
                |row| row.get(0),
            )
            .optional()?;
        payload
            .map(|payload| {
                let state: ReadState = decode(&payload)?;
                self.validate_for_project(&state, &state.project)?;
                Ok(state)
            })
            .transpose()
    }

    pub fn mark_work_read(
        &mut self,
        work_id: &str,
        artifact_id: &str,
        artifact_revision: u32,
    ) -> Result<ReadState, OperatorStoreError> {
        let tx = self.connection.transaction()?;
        let work = stored_mutable_in_tx::<WorkItem>(&tx, "work_items", work_id)?
            .ok_or(OperatorStoreError::Conflict("work item no longer exists"))?;
        if work.value.current_artifact_id.as_deref() != Some(artifact_id)
            || work.value.current_artifact_revision != Some(artifact_revision)
        {
            return Err(OperatorStoreError::Conflict(
                "report changed; reload before marking read",
            ));
        }
        let artifact: Option<String> = tx
            .query_row(
                "SELECT payload FROM artifacts WHERE artifact_id=?1 AND revision=?2",
                params![artifact_id, artifact_revision],
                |row| row.get(0),
            )
            .optional()?;
        let artifact: ArtifactRevision =
            decode(&artifact.ok_or(OperatorStoreError::Conflict("saved report is unavailable"))?)?;
        if artifact.project != self.identity || work.value.project != self.identity {
            return Err(OperatorStoreError::ProjectMismatch);
        }
        let previous: Option<String> = tx
            .query_row(
                "SELECT payload FROM read_states WHERE work_id=?1",
                [work_id],
                |row| row.get(0),
            )
            .optional()?;
        let previous: Option<ReadState> = previous.map(|payload| decode(&payload)).transpose()?;
        if let Some(previous) = &previous {
            if previous.project != self.identity {
                return Err(OperatorStoreError::ProjectMismatch);
            }
            if previous.artifact_id == artifact_id
                && previous.last_read_revision >= artifact_revision
            {
                return Ok(previous.clone());
            }
        }
        let state = ReadState {
            schema_version: OPERATOR_SCHEMA_VERSION_V1,
            project: self.identity.clone(),
            work_id: work_id.to_owned(),
            artifact_id: artifact_id.to_owned(),
            last_read_revision: artifact_revision,
            read_at: chrono::Utc::now().to_rfc3339(),
            snoozed_until: previous
                .as_ref()
                .and_then(|previous| previous.snoozed_until.clone()),
            muted: previous.as_ref().is_some_and(|previous| previous.muted),
        };
        tx.execute(
            "INSERT INTO read_states(work_id,payload) VALUES(?1,?2) ON CONFLICT(work_id) DO UPDATE SET payload=excluded.payload",
            params![work_id, encode(&state)?],
        )?;
        tx.commit()?;
        Ok(state)
    }

    pub(crate) fn retire_superseded_backlog(&mut self) -> Result<usize, OperatorStoreError> {
        use refact_buddy_core::operator_types::{RunStatus, WorkItemState};
        use std::collections::BTreeMap;
        #[cfg(test)]
        if !super::supersede::open_pass_enabled() {
            return Ok(0);
        }

        let rows = {
            let mut statement = self.connection.prepare(
                "SELECT payload,revision,change_seq FROM work_items WHERE json_extract(payload,'$.project.project_id')=?1 AND json_extract(payload,'$.project.canonical_root')=?2 AND json_extract(payload,'$.subject.resource_kind')='workflow' AND json_extract(payload,'$.state') IN ('needs_attention','watching','blocked') ORDER BY id",
            )?;
            let rows = statement.query_map(
                params![self.identity.project_id, self.identity.canonical_root],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, u64>(1)?,
                        row.get::<_, u64>(2)?,
                    ))
                },
            )?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        let mut groups: BTreeMap<
            (String, String),
            Vec<(Stored<WorkItem>, super::supersede::Candidate)>,
        > = BTreeMap::new();
        for (payload, revision, change_seq) in rows {
            let work: WorkItem = decode(&payload)?;
            if work.project != self.identity
                || work.subject.resource_kind != "workflow"
                || !matches!(
                    work.state,
                    WorkItemState::NeedsAttention
                        | WorkItemState::Watching
                        | WorkItemState::Blocked
                )
            {
                continue;
            }
            let workflow = work.subject.resource_id.clone();
            let latest: Option<String> = self
                .connection
                .query_row(
                    "SELECT payload FROM run_outcomes WHERE json_extract(payload,'$.work_id')=?1 AND json_extract(payload,'$.workflow')=?2 ORDER BY change_seq DESC LIMIT 1",
                    params![work.id, workflow],
                    |row| row.get(0),
                )
                .optional()?;
            let run = match latest {
                Some(payload) => match decode::<RunOutcome>(&payload) {
                    Ok(run) => Some(run),
                    Err(_) => None,
                },
                None => None,
            };
            let (status, coverage, outcome_time) = match run {
                Some(run) => (
                    run.status,
                    run.coverage,
                    chrono::DateTime::parse_from_rfc3339(&run.started_at).ok(),
                ),
                None => (
                    match work.state {
                        WorkItemState::Blocked => RunStatus::Blocked,
                        WorkItemState::NeedsAttention | WorkItemState::Watching => {
                            RunStatus::Inconclusive
                        }
                        _ => continue,
                    },
                    work.scope.clone().unwrap_or_default(),
                    None,
                ),
            };
            let observed_at = outcome_time.or_else(|| {
                chrono::DateTime::parse_from_rfc3339(&work.last_material_change_at).ok()
            });
            let Some(observed_at) = observed_at else {
                continue;
            };
            groups
                .entry((workflow, super::supersede::coverage_class(&coverage)))
                .or_default()
                .push((
                    Stored {
                        value: work.clone(),
                        revision,
                        change_seq,
                    },
                    super::supersede::Candidate {
                        id: work.id,
                        status,
                        observed_at,
                        eligible: true,
                    },
                ));
        }

        let mut retired = 0;
        #[cfg(test)]
        let mut processed_groups = 0;
        for (_, group) in groups {
            let candidates: Vec<_> = group
                .iter()
                .map(|(_, candidate)| candidate.clone())
                .collect();
            let retire_ids = super::supersede::retire_ids(&candidates);
            let tx = self.connection.transaction()?;
            let mut group_retired = 0;
            for (stored, _) in &group {
                if !retire_ids.contains(&stored.value.id) {
                    continue;
                }
                let Some(current) =
                    stored_mutable_in_tx::<WorkItem>(&tx, "work_items", &stored.value.id)?
                else {
                    continue;
                };
                if !matches!(
                    current.value.state,
                    WorkItemState::NeedsAttention
                        | WorkItemState::Watching
                        | WorkItemState::Blocked
                ) {
                    continue;
                }
                let mut retired_item = current.value;
                retired_item.state = WorkItemState::Cancelled;
                upsert_work_item_in_tx(
                    &tx,
                    &retired_item,
                    &encode(&retired_item)?,
                    Some(current.revision),
                )?;
                group_retired += 1;
            }
            tx.commit()?;
            retired += group_retired;
            #[cfg(test)]
            {
                processed_groups += 1;
                if super::supersede::should_interrupt_after_group(processed_groups) {
                    return Err(OperatorStoreError::Conflict(
                        "interrupted superseded backlog pass",
                    ));
                }
            }
        }
        Ok(retired)
    }

    pub fn list_work_items(
        &self,
        after_seq: Option<u64>,
        limit: usize,
    ) -> Result<Page<Stored<WorkItem>>, OperatorStoreError> {
        bounded(limit)?;
        let mut stmt=self.connection.prepare("SELECT payload,revision,change_seq FROM work_items WHERE change_seq>?1 ORDER BY change_seq,id LIMIT ?2")?;
        let result = page(
            stmt.query_map(params![after_seq.unwrap_or(0), (limit + 1) as i64], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?,
            limit,
        );
        result
    }

    pub fn get_work_snooze(&self, work_id: &str) -> Result<Option<WorkSnooze>, OperatorStoreError> {
        let work = self
            .get_work_item(work_id)?
            .ok_or(OperatorStoreError::Conflict("work item no longer exists"))?;
        let payload: Option<String> = self
            .connection
            .query_row(
                "SELECT payload FROM work_snoozes WHERE work_id=?1",
                [work_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(payload) = payload else {
            return Ok(None);
        };
        let snooze: WorkSnooze = decode(&payload)?;
        self.validate_for_project(&snooze, &snooze.project)?;
        let until = chrono::DateTime::parse_from_rfc3339(&snooze.until)
            .map_err(|_| OperatorStoreError::CorruptSchema("invalid work snooze timestamp"))?;
        Ok((snooze.work_id == work_id
            && work.value.current_artifact_id.as_deref() == Some(snooze.artifact_id.as_str())
            && work.value.current_artifact_revision == Some(snooze.artifact_revision)
            && until > chrono::Utc::now())
        .then_some(snooze))
    }

    pub fn snooze_work(
        &mut self,
        work_id: &str,
        artifact_id: &str,
        artifact_revision: u32,
        minutes: Option<u32>,
    ) -> Result<Option<WorkSnooze>, OperatorStoreError> {
        if minutes.is_some_and(|minutes| !(1..=10080).contains(&minutes)) {
            return Err(OperatorStoreError::InvalidInput(
                "snooze must be between one minute and seven days",
            ));
        }
        let tx = self.connection.transaction()?;
        let work = stored_mutable_in_tx::<WorkItem>(&tx, "work_items", work_id)?
            .ok_or(OperatorStoreError::Conflict("work item no longer exists"))?;
        if work.value.project != self.identity {
            return Err(OperatorStoreError::ProjectMismatch);
        }
        if work.value.current_artifact_id.as_deref() != Some(artifact_id)
            || work.value.current_artifact_revision != Some(artifact_revision)
        {
            return Err(OperatorStoreError::Conflict(
                "report changed; reload before snoozing",
            ));
        }
        let state = minutes.map(|minutes| WorkSnooze {
            schema_version: OPERATOR_SCHEMA_VERSION_V1,
            project: self.identity.clone(),
            work_id: work_id.to_owned(),
            artifact_id: artifact_id.to_owned(),
            artifact_revision,
            until: (chrono::Utc::now() + chrono::Duration::minutes(i64::from(minutes)))
                .to_rfc3339(),
        });
        if let Some(state) = &state {
            tx.execute("INSERT INTO work_snoozes(work_id,payload) VALUES(?1,?2) ON CONFLICT(work_id) DO UPDATE SET payload=excluded.payload",
                params![work_id, encode(state)?])?;
        } else {
            tx.execute("DELETE FROM work_snoozes WHERE work_id=?1", [work_id])?;
        }
        tx.commit()?;
        Ok(state)
    }

    pub fn search_work_items(
        &self,
        after: Option<u64>,
        limit: usize,
        view: Option<&str>,
        search: Option<&str>,
    ) -> Result<Page<Stored<WorkItem>>, OperatorStoreError> {
        self.search_work_items_with_attention(after, limit, view, search, None)
    }

    pub fn search_work_items_with_attention(
        &self,
        after: Option<u64>,
        limit: usize,
        view: Option<&str>,
        search: Option<&str>,
        attention: Option<&str>,
    ) -> Result<Page<Stored<WorkItem>>, OperatorStoreError> {
        bounded(limit)?;
        if attention
            .is_some_and(|value| !["all", "unread", "snoozed", "unsnoozed"].contains(&value))
        {
            return Err(OperatorStoreError::InvalidInput("unknown attention filter"));
        }
        if view.is_some_and(|view| {
            !["needs_you", "working_on", "delivered", "watching"].contains(&view)
        }) {
            return Err(OperatorStoreError::InvalidInput("unknown work view"));
        }
        let search = search.unwrap_or("").trim().to_ascii_lowercase();
        if search.len() > 512 {
            return Err(OperatorStoreError::InvalidInput("work search is too long"));
        }
        let cursor_key = after
            .map(|rowid| {
                let rowid = i64::try_from(rowid)
                    .map_err(|_| OperatorStoreError::InvalidInput("invalid work cursor"))?;
                self.connection
                    .query_row(
                        "SELECT julianday(json_extract(payload,'$.last_material_change_at')),id FROM work_items WHERE rowid=?1",
                        [rowid],
                        |row| Ok((row.get::<_, f64>(0)?, row.get::<_, String>(1)?)),
                    )
                    .optional()?
                    .ok_or(OperatorStoreError::InvalidInput("unknown work cursor"))
            })
            .transpose()?;
        let mut statement = self.connection.prepare(
            "SELECT w.payload,w.revision,w.change_seq,w.rowid FROM work_items w
             LEFT JOIN read_states r ON r.work_id=w.id
             LEFT JOIN work_snoozes s ON s.work_id=w.id
             WHERE (?1 IS NULL OR julianday(json_extract(w.payload,'$.last_material_change_at'))<?2 OR\n                (julianday(json_extract(w.payload,'$.last_material_change_at'))=?2 AND w.id<?3)) AND (\n                ?4 IS NULL OR
                (?4='needs_you' AND json_extract(w.payload,'$.state') IN ('needs_attention','blocked')) OR
                (?4='working_on' AND json_extract(w.payload,'$.state')='working') OR
                (?4='delivered' AND json_extract(w.payload,'$.state') IN ('delivered','cancelled')) OR
                (?4='watching' AND json_extract(w.payload,'$.state')='watching')
            ) AND (?5='' OR instr(lower(json_extract(w.payload,'$.title') || ' ' ||
                json_extract(w.payload,'$.subject.resource_id') || ' ' || json_extract(w.payload,'$.kind')),?5)>0)
            AND (?7='all' OR (?7='unread' AND
                json_extract(w.payload,'$.current_artifact_id') IS NOT NULL AND
                (r.payload IS NULL OR
                 json_extract(r.payload,'$.artifact_id') != json_extract(w.payload,'$.current_artifact_id') OR
                 json_extract(r.payload,'$.last_read_revision') < json_extract(w.payload,'$.current_artifact_revision'))) OR
                (?7='snoozed' AND COALESCE(
                    json_extract(s.payload,'$.artifact_id')=json_extract(w.payload,'$.current_artifact_id') AND
                    json_extract(s.payload,'$.artifact_revision')=json_extract(w.payload,'$.current_artifact_revision') AND
                    julianday(json_extract(s.payload,'$.until'))>julianday(?6),0)) OR
                (?7='unsnoozed' AND NOT COALESCE(
                    json_extract(s.payload,'$.artifact_id')=json_extract(w.payload,'$.current_artifact_id') AND
                    json_extract(s.payload,'$.artifact_revision')=json_extract(w.payload,'$.current_artifact_revision') AND
                    julianday(json_extract(s.payload,'$.until'))>julianday(?6),0)))
            ORDER BY julianday(json_extract(w.payload,'$.last_material_change_at')) DESC,w.id DESC LIMIT ?8",
        )?;
        let rows = statement.query_map(
            params![
                after
                    .map(i64::try_from)
                    .transpose()
                    .map_err(|_| OperatorStoreError::InvalidInput("invalid work cursor"))?,
                cursor_key.as_ref().map(|(date, _)| *date),
                cursor_key.as_ref().map(|(_, id)| id.as_str()),
                view,
                search,
                chrono::Utc::now().to_rfc3339(),
                attention.unwrap_or("all"),
                (limit + 1) as i64
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, u64>(1)?,
                    row.get::<_, u64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )?;
        let mut entries = Vec::new();
        for row in rows {
            let (payload, revision, change_seq, rowid) = row?;
            entries.push((
                Stored {
                    value: decode(&payload)?,
                    revision,
                    change_seq,
                },
                rowid as u64,
            ));
        }
        let next_after = if entries.len() > limit {
            entries.pop();
            entries.last().map(|(_, rowid)| *rowid)
        } else {
            None
        };
        Ok(Page {
            entries: entries.into_iter().map(|(entry, _)| entry).collect(),
            next_after,
        })
    }

    pub fn dismiss_work_item_with_execution(
        &mut self,
        execution: &ActionExecution,
        item: &WorkItem,
        expected_work_revision: u64,
    ) -> Result<(Stored<ActionExecution>, Stored<WorkItem>), OperatorStoreError> {
        self.validate_for_project(execution, &execution.project)?;
        self.validate_for_project(item, &item.project)?;
        if execution.project != self.identity
            || item.project != self.identity
            || execution.status != ActionExecutionState::Completed
            || item.state != refact_buddy_core::operator_types::WorkItemState::Cancelled
        {
            return Err(OperatorStoreError::Conflict("invalid work dismissal"));
        }
        let payload = encode(execution)?;
        let item_payload = encode(item)?;
        let tx = self.connection.transaction()?;
        let existing: Option<(String, u64, u64)> = tx
            .query_row(
                "SELECT payload,revision,change_seq FROM executions WHERE idempotency_key=?1",
                [&execution.idempotency_key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        if let Some((payload, revision, change_seq)) = existing {
            let existing: ActionExecution = decode(&payload)?;
            if existing.spec_hash == execution.spec_hash
                && existing.action_id == execution.action_id
                && existing.requester == execution.requester
                && existing.project == execution.project
            {
                let work = stored_mutable_in_tx::<WorkItem>(&tx, "work_items", &item.id)?
                    .ok_or(OperatorStoreError::Conflict("dismissed work item missing"))?;
                return Ok((
                    Stored {
                        value: existing,
                        revision,
                        change_seq,
                    },
                    work,
                ));
            }
            return Err(OperatorStoreError::Conflict(
                "idempotency key was used with different immutable execution identity",
            ));
        }
        let current = stored_mutable_in_tx::<WorkItem>(&tx, "work_items", &item.id)?
            .ok_or(OperatorStoreError::Conflict("work item no longer exists"))?;
        if current.revision != expected_work_revision {
            return Err(OperatorStoreError::StaleRevision {
                current: current.revision,
            });
        }
        let execution_seq = next_seq(&tx)?;
        tx.execute(
            "INSERT INTO executions(execution_id,idempotency_key,spec_hash,payload,revision,change_seq) VALUES(?1,?2,?3,?4,1,?5)",
            params![execution.execution_id, execution.idempotency_key, execution.spec_hash, payload, execution_seq],
        )?;
        let work = upsert_work_item_in_tx(&tx, item, &item_payload, Some(expected_work_revision))?;
        tx.commit()?;
        Ok((
            Stored {
                value: execution.clone(),
                revision: 1,
                change_seq: execution_seq as u64,
            },
            work,
        ))
    }

    /// Inserts the intent before any effect. A matching key/hash returns the original claim;
    /// a reused key with a different hash is rejected. It does not provide exactly-once effects.
    pub fn claim_execution(
        &mut self,
        execution: &ActionExecution,
    ) -> Result<ExecutionClaim, OperatorStoreError> {
        self.claim_execution_inner(execution, false, None)
    }

    /// Only authorized policy-control/recovery owners may bypass new-work admission.
    pub(crate) fn claim_completion_execution(
        &mut self,
        execution: &ActionExecution,
    ) -> Result<ExecutionClaim, OperatorStoreError> {
        self.claim_execution_inner(execution, true, None)
    }

    pub(crate) fn claim_resource_execution(
        &mut self,
        execution: &ActionExecution,
        prepared: &super::action_execution::PreparedAction,
    ) -> Result<ExecutionClaim, OperatorStoreError> {
        if prepared.spec.project != self.identity
            || prepared.spec_hash != execution.spec_hash
            || prepared.spec.action_id != execution.action_id
            || super::action_execution::action_spec_hash(
                &prepared.spec,
                &prepared.bound_parameter_ref,
            ) != prepared.spec_hash
        {
            return Err(OperatorStoreError::Conflict(
                "resource execution binding changed",
            ));
        }
        let policy_control = super::resources::retention::is_policy_action(&prepared.spec);
        self.claim_execution_inner(
            execution,
            policy_control,
            Some(policy_control.then_some("policy_control")),
        )
    }

    /// The migration cutoff identifies old completed receipts; active old executions
    /// retain their migration marker even after a state update advances change_seq.
    pub(crate) fn is_legacy_resource_execution(
        &self,
        id: &str,
    ) -> Result<bool, OperatorStoreError> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM executions e WHERE e.execution_id=?1 AND (e.change_seq <= (SELECT CAST(value AS INTEGER) FROM meta WHERE key='terminal_reservation_legacy_cutoff') OR EXISTS(SELECT 1 FROM terminal_reservations r WHERE r.producer='resource_apply' AND r.producer_id=e.execution_id AND r.coverage='grandfathered_legacy')))",
            [id], |row| row.get(0),
        )?)
    }

    /// Recovery is completion work, but still owns a charged receipt slot keyed by
    /// its actual execution ID. The original must have native reservation evidence
    /// (or pre-migration identity), not merely the same specification hash.
    pub(crate) fn claim_resource_recovery_execution(
        &mut self,
        execution: &ActionExecution,
        original_id: &str,
    ) -> Result<ExecutionClaim, OperatorStoreError> {
        let original = self
            .get_execution(original_id)?
            .ok_or(OperatorStoreError::Conflict("recovery original missing"))?
            .value;
        let recovery = original
            .recovery
            .as_ref()
            .ok_or(OperatorStoreError::Conflict(
                "recovery original unavailable",
            ))?;
        let legacy = self.is_legacy_resource_execution(original_id)?;
        let reserved: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM terminal_reservations WHERE producer='resource_apply' AND producer_id=?1 AND namespace='resource_receipt' AND document_id=?1)",
            [original_id], |row| row.get(0),
        )?;
        if (!legacy && !reserved)
            || !matches!(
                original.status,
                ActionExecutionState::Completed | ActionExecutionState::Partial
            )
            || !recovery.available
            || recovery.action_id != original.action_id
            || (recovery.recovery_id != original.execution_id
                && !(legacy && recovery.recovery_id == original.spec_hash))
            || execution.project != original.project
            || execution.execution_id == original.execution_id
            || execution.status != ActionExecutionState::Claimed
            || execution.action_id != format!("recovery:{}:{}", original_id, recovery.recovery_id)
        {
            return Err(OperatorStoreError::Conflict(
                "recovery execution binding changed",
            ));
        }
        if let Some(existing) = self.get_execution_by_idempotency_key(&execution.idempotency_key)? {
            if existing.value.action_id == execution.action_id
                && existing.value.spec_hash == execution.spec_hash
                && existing.value.requester == execution.requester
            {
                return Ok(ExecutionClaim::Existing(existing));
            }
            return Err(OperatorStoreError::Conflict("recovery identity changed"));
        }
        let credit_key = format!("document_recovery_credit:{original_id}");
        let credit: Option<String> = self
            .connection
            .query_row("SELECT value FROM meta WHERE key=?1", [&credit_key], |r| {
                r.get(0)
            })
            .optional()?;
        if self.document_growth_policy()?.is_some() && !matches!(credit.as_deref(), Some("2" | "4"))
        {
            return Err(OperatorStoreError::Conflict(
                "one-shot recovery reserve exhausted",
            ));
        }
        // Claim and credit consumption must roll back together on an identity collision.
        self.claim_execution_inner(
            execution,
            true,
            Some(Some(&format!("recovery_completion:{original_id}"))),
        )
    }

    fn claim_execution_inner(
        &mut self,
        execution: &ActionExecution,
        completion: bool,
        resource: Option<Option<&str>>,
    ) -> Result<ExecutionClaim, OperatorStoreError> {
        if self
            .get_execution_by_idempotency_key(&execution.idempotency_key)?
            .is_none()
            && !completion
        {
            self.admit_new_work()?;
        }
        self.validate_for_project(execution, &execution.project)?;
        let payload = encode(execution)?;
        let policy = self.document_growth_policy()?;
        let tx = self.connection.transaction()?;
        let existing_claim: Option<(String, i64, i64, String)> = tx.query_row("SELECT payload,revision,change_seq,spec_hash FROM executions WHERE idempotency_key=?1", [&execution.idempotency_key], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).optional()?;
        if let Some((existing, revision, seq, hash)) = existing_claim {
            let existing_value: ActionExecution = decode(&existing)?;
            if hash != execution.spec_hash
                || existing_value.action_id != execution.action_id
                || existing_value.requester != execution.requester
                || existing_value.project != execution.project
            {
                return Err(OperatorStoreError::Conflict(
                    "idempotency key was used with different immutable execution identity",
                ));
            }
            return Ok(ExecutionClaim::Existing(Stored {
                value: existing_value,
                revision: revision as u64,
                change_seq: seq as u64,
            }));
        }
        if let Some(exemption) = resource {
            if let Some(original) = exemption.and_then(|s| s.strip_prefix("recovery_completion:")) {
                let changed = tx.execute(
                    "UPDATE meta SET value='0' WHERE key=?1 AND value IN ('2','4')",
                    [format!("document_recovery_credit:{original}")],
                )?;
                if changed != 1 && policy.is_some() {
                    return Err(OperatorStoreError::Conflict(
                        "one-shot recovery reserve exhausted",
                    ));
                }
            } else {
                admit_document_charge(&tx, policy.as_ref(), 5)?;
                tx.execute(
                    "INSERT INTO meta(key,value) VALUES(?1,'4')",
                    [format!(
                        "document_recovery_credit:{}",
                        execution.execution_id
                    )],
                )?;
            }
            reserve_terminal_documents_in_tx(
                &tx,
                policy.as_ref(),
                "resource_apply",
                &execution.execution_id,
                &[("resource_receipt", execution.execution_id.clone(), 1)],
                exemption,
            )?;
        }
        let seq = next_seq(&tx)?;
        tx.execute("INSERT INTO executions(execution_id,idempotency_key,spec_hash,payload,revision,change_seq) VALUES(?1,?2,?3,?4,1,?5)", params![execution.execution_id,execution.idempotency_key,execution.spec_hash,payload,seq])?;
        tx.commit()?;
        Ok(ExecutionClaim::Claimed(Stored {
            value: execution.clone(),
            revision: 1,
            change_seq: seq as u64,
        }))
    }
    pub fn update_execution_state(
        &mut self,
        id: &str,
        expected_revision: u64,
        state: ActionExecutionState,
    ) -> Result<Stored<ActionExecution>, OperatorStoreError> {
        let tx = self.connection.transaction()?;
        let (payload, current): (String, i64) = tx
            .query_row(
                "SELECT payload,revision FROM executions WHERE execution_id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or(OperatorStoreError::StaleRevision {
                current: 0,
            })?;
        if current as u64 != expected_revision {
            return Err(OperatorStoreError::StaleRevision {
                current: current as u64,
            });
        }
        let mut value: ActionExecution = decode(&payload)?;
        value.status = state;
        let encoded = encode(&value)?;
        let seq = next_seq(&tx)?;
        let count=tx.execute("UPDATE executions SET payload=?1,revision=?2,change_seq=?3 WHERE execution_id=?4 AND revision=?5",params![encoded,current+1,seq,id,current])?;
        if count != 1 {
            return Err(OperatorStoreError::StaleRevision {
                current: current as u64,
            });
        }
        tx.commit()?;
        Ok(Stored {
            value,
            revision: (current + 1) as u64,
            change_seq: seq as u64,
        })
    }

    pub fn replace_execution(
        &mut self,
        execution: &ActionExecution,
        expected_revision: u64,
    ) -> Result<Stored<ActionExecution>, OperatorStoreError> {
        self.validate_for_project(execution, &execution.project)?;
        let payload = encode(execution)?;
        let tx = self.connection.transaction()?;
        let (previous, revision): (String, u64) = tx
            .query_row(
                "SELECT payload, revision FROM executions WHERE execution_id=?1",
                [&execution.execution_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .ok_or(OperatorStoreError::StaleRevision {
                current: 0,
            })?;
        if revision != expected_revision {
            return Err(OperatorStoreError::StaleRevision {
                current: revision,
            });
        }
        let previous: ActionExecution = decode(&previous)?;
        if previous.action_id != execution.action_id
            || previous.spec_hash != execution.spec_hash
            || previous.requester != execution.requester
            || previous.project != execution.project
            || previous.idempotency_key != execution.idempotency_key
            || previous.decision != execution.decision
            || previous.worker != execution.worker
        {
            return Err(OperatorStoreError::Conflict(
                "execution identity cannot change",
            ));
        }
        let change_seq = next_seq(&tx)?;
        let next_revision = revision
            .checked_add(1)
            .ok_or(OperatorStoreError::InvalidInput(
                "execution revision overflow",
            ))?;
        tx.execute(
            "UPDATE executions SET payload=?1,revision=?2,change_seq=?3 WHERE execution_id=?4 AND revision=?5",
            params![payload, next_revision, change_seq, execution.execution_id, expected_revision],
        )?;
        if matches!(
            execution.status,
            ActionExecutionState::Completed
                | ActionExecutionState::Failed
                | ActionExecutionState::Cancelled
        ) {
            tx.execute("UPDATE terminal_reservations SET remaining=0,state='finished' WHERE producer='resource_apply' AND producer_id=?1", [&execution.execution_id])?;
        }
        tx.commit()?;
        Ok(Stored {
            value: execution.clone(),
            revision: next_revision,
            change_seq: change_seq as u64,
        })
    }

    pub fn get_execution(
        &self,
        execution_id: &str,
    ) -> Result<Option<Stored<ActionExecution>>, OperatorStoreError> {
        let row: Option<(String, u64, u64)> = self
            .connection
            .query_row(
                "SELECT payload, revision, change_seq FROM executions WHERE execution_id=?1",
                [execution_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        row.map(|(payload, revision, change_seq)| {
            let value: ActionExecution = decode(&payload)?;
            self.validate_for_project(&value, &value.project)?;
            Ok(Stored {
                value,
                revision,
                change_seq,
            })
        })
        .transpose()
    }

    pub fn latest_execution_for_spec(
        &self,
        spec_hash: &str,
    ) -> Result<Option<Stored<ActionExecution>>, OperatorStoreError> {
        let id: Option<String> = self.connection.query_row(
            "SELECT execution_id FROM executions WHERE spec_hash=?1 ORDER BY change_seq DESC LIMIT 1",
            [spec_hash], |row| row.get(0),
        ).optional()?;
        id.map(|id| self.get_execution(&id))
            .transpose()
            .map(Option::flatten)
    }

    fn validate_for_project<T: OperatorContract>(
        &self,
        value: &T,
        project: &ProjectIdentity,
    ) -> Result<(), OperatorStoreError> {
        value
            .validate()
            .map_err(|_| OperatorStoreError::InvalidInput("invalid operator contract"))?;
        if project != &self.identity {
            return Err(OperatorStoreError::ProjectMismatch);
        }
        Ok(())
    }

    fn append_immutable<T: Serialize + DeserializeOwned + Clone>(
        &mut self,
        table: &str,
        id_col: &str,
        id: &str,
        value: &T,
    ) -> Result<Stored<T>, OperatorStoreError> {
        let payload = encode(value)?;
        let tx = self.connection.transaction()?;
        let seq = next_seq(&tx)?;
        let sql =
            format!("INSERT OR IGNORE INTO {table}({id_col},payload,change_seq) VALUES(?1,?2,?3)");
        if tx.execute(&sql, params![id, payload, seq])? == 0 {
            return immutable_existing(
                &tx,
                table,
                &format!("{id_col} = ?1"),
                params![id],
                value,
                0,
            );
        }
        tx.commit()?;
        Ok(Stored {
            value: value.clone(),
            revision: 0,
            change_seq: seq as u64,
        })
    }
    fn get_immutable<T: DeserializeOwned>(
        &self,
        table: &str,
        id_col: &str,
        id: &str,
    ) -> Result<Option<Stored<T>>, OperatorStoreError> {
        let sql = format!("SELECT payload,change_seq FROM {table} WHERE {id_col}=?1");
        self.connection
            .query_row(&sql, [id], |r| {
                Ok(Stored {
                    value: decode::<T>(&r.get::<_, String>(0)?)
                        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
                    revision: 0,
                    change_seq: r.get::<_, i64>(1)? as u64,
                })
            })
            .optional()
            .map_err(Into::into)
    }
    fn get_mutable<T: DeserializeOwned>(
        &self,
        table: &str,
        id: &str,
    ) -> Result<Option<Stored<T>>, OperatorStoreError> {
        let sql = format!("SELECT payload,revision,change_seq FROM {table} WHERE id=?1");
        self.connection
            .query_row(&sql, [id], |r| {
                Ok(Stored {
                    value: decode::<T>(&r.get::<_, String>(0)?)
                        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
                    revision: r.get::<_, i64>(1)? as u64,
                    change_seq: r.get::<_, i64>(2)? as u64,
                })
            })
            .optional()
            .map_err(Into::into)
    }
}
fn migrate(connection: &Connection, identity: &ProjectIdentity) -> Result<(), OperatorStoreError> {
    let version: i64 = connection.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version < 0 {
        return Err(OperatorStoreError::CorruptSchema(
            "negative SQLite user_version",
        ));
    }
    verify_schema_matches_version(connection, version)?;
    let encoded = encode(identity)?;
    if version == 0 {
        let tx = connection.unchecked_transaction()?;
        tx.execute_batch("CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT NOT NULL); CREATE TABLE run_outcomes(run_id TEXT PRIMARY KEY,payload TEXT NOT NULL,change_seq INTEGER NOT NULL); CREATE TABLE artifacts(artifact_id TEXT NOT NULL,revision INTEGER NOT NULL,payload TEXT NOT NULL,change_seq INTEGER NOT NULL,PRIMARY KEY(artifact_id,revision)); CREATE TABLE work_items(id TEXT PRIMARY KEY,payload TEXT NOT NULL,revision INTEGER NOT NULL,change_seq INTEGER NOT NULL); CREATE TABLE executions(execution_id TEXT PRIMARY KEY,idempotency_key TEXT UNIQUE NOT NULL,spec_hash TEXT NOT NULL,payload TEXT NOT NULL,revision INTEGER NOT NULL,change_seq INTEGER NOT NULL); CREATE TABLE changes(seq INTEGER PRIMARY KEY);")?;
        tx.execute(
            "INSERT INTO meta(key,value) VALUES('project_identity',?1)",
            [&encoded],
        )?;
        tx.pragma_update(None, "user_version", 1)?;
        tx.commit()?;
    }
    let existing: Option<String> = connection
        .query_row(
            "SELECT value FROM meta WHERE key='project_identity'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    match existing {
        Some(value) if value == encoded => {
            if version < 2 {
                let tx = connection.unchecked_transaction()?;
                tx.execute_batch("CREATE TABLE read_states(work_id TEXT PRIMARY KEY REFERENCES work_items(id),payload TEXT NOT NULL);")?;
                tx.pragma_update(None, "user_version", 2)?;
                tx.commit()?;
            }
            if version < 3 {
                let tx = connection.unchecked_transaction()?;
                tx.execute_batch("CREATE TABLE work_snoozes(work_id TEXT PRIMARY KEY REFERENCES work_items(id),payload TEXT NOT NULL);")?;
                tx.pragma_update(None, "user_version", 3)?;
                tx.commit()?;
            }
            if version < 4 {
                let tx = connection.unchecked_transaction()?;
                tx.execute_batch(
                    "CREATE TABLE active_runs(run_id TEXT PRIMARY KEY,payload TEXT NOT NULL);",
                )?;
                tx.pragma_update(None, "user_version", 4)?;
                tx.commit()?;
            }
            if version < 5 {
                let tx = connection.unchecked_transaction()?;
                tx.execute_batch("CREATE TABLE operator_documents(namespace TEXT NOT NULL,id TEXT NOT NULL,revision INTEGER NOT NULL,payload TEXT NOT NULL,change_seq INTEGER NOT NULL,PRIMARY KEY(namespace,id,revision)); CREATE INDEX documents_by_change ON operator_documents(namespace,change_seq);")?;
                tx.pragma_update(None, "user_version", 5)?;
                tx.commit()?;
            }
            if version < 6 {
                let tx = connection.unchecked_transaction()?;
                tx.execute_batch("CREATE TABLE terminal_reservations(producer TEXT NOT NULL,producer_id TEXT NOT NULL,namespace TEXT NOT NULL,document_id TEXT NOT NULL,budget INTEGER NOT NULL CHECK(budget>0),remaining INTEGER NOT NULL CHECK(remaining>=0),consumed INTEGER NOT NULL CHECK(consumed>=0),coverage TEXT NOT NULL,state TEXT NOT NULL CHECK(state IN ('reserved','finished')),PRIMARY KEY(producer,producer_id,namespace,document_id),UNIQUE(namespace,document_id),CHECK(remaining+consumed<=budget));")?;
                tx.execute("INSERT INTO meta(key,value) VALUES('terminal_reservation_legacy_cutoff',(SELECT COALESCE(max(seq),0) FROM changes)) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [])?;
                tx.execute("INSERT INTO terminal_reservations(producer,producer_id,namespace,document_id,budget,remaining,consumed,coverage,state) SELECT 'resource_apply',execution_id,'resource_receipt',execution_id,1,1,0,'grandfathered_legacy','reserved' FROM executions WHERE json_extract(payload,'$.status') IN ('claimed','running','reconciling','outcome_unknown')", [])?;
                tx.pragma_update(None, "user_version", 6)?;
                tx.commit()?;
            }
            if version < 7 {
                let tx = connection.unchecked_transaction()?;
                let payload: Option<String> = tx
                    .query_row(
                        "SELECT value FROM meta WHERE key='document_growth_policy'",
                        [],
                        |r| r.get(0),
                    )
                    .optional()?;
                let policy: Option<super::operator_observability::DocumentGrowthPolicy> =
                    payload.map(|p| decode(&p)).transpose()?.flatten();
                if let Some(policy) = &policy {
                    if !(1..=100_000).contains(&policy.max_rows_per_namespace) {
                        return Err(OperatorStoreError::CorruptSchema("invalid document policy"));
                    }
                    initialize_document_accounting(&tx)?;
                    document_bound(&tx, policy)?;
                }
                validate_document_accounting(&tx)?;
                tx.pragma_update(None, "user_version", 7)?;
                tx.commit()?;
            }
            validate_document_accounting(connection)?;
            Ok(())
        }
        Some(_) => Err(OperatorStoreError::ProjectMismatch),
        None => Err(OperatorStoreError::CorruptSchema(
            "operator store has no project binding",
        )),
    }
}
const V1_TABLES: [&str; 6] = [
    "meta",
    "run_outcomes",
    "artifacts",
    "work_items",
    "executions",
    "changes",
];
const V2_TABLES: [&str; 1] = ["read_states"];
const V3_TABLES: [&str; 1] = ["work_snoozes"];
const V4_TABLES: [&str; 1] = ["active_runs"];
const V5_TABLES: [&str; 1] = ["operator_documents"];
const V6_TABLES: [&str; 1] = ["terminal_reservations"];

fn document_key(namespace: &str, id: &str) -> Result<(), OperatorStoreError> {
    if namespace.is_empty()
        || namespace.len() > 80
        || !namespace
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
        || id.trim().is_empty()
        || id.len() > 512
        || id.chars().any(char::is_control)
    {
        return Err(OperatorStoreError::InvalidInput(
            "invalid document identity",
        ));
    }
    Ok(())
}

fn user_tables(connection: &Connection) -> Result<Vec<String>, OperatorStoreError> {
    let mut statement = connection.prepare(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
    )?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn verify_schema_matches_version(
    connection: &Connection,
    version: i64,
) -> Result<(), OperatorStoreError> {
    let present = user_tables(connection)?;
    let has = |name: &str| present.iter().any(|table| table == name);
    if version == 0 {
        return if present.is_empty() {
            Ok(())
        } else {
            Err(OperatorStoreError::CorruptSchema(
                "operator store has tables but no recorded schema version",
            ))
        };
    }
    let required = V1_TABLES
        .iter()
        .chain(V2_TABLES.iter().filter(|_| version >= 2))
        .chain(V3_TABLES.iter().filter(|_| version >= 3))
        .chain(V4_TABLES.iter().filter(|_| version >= 4))
        .chain(V5_TABLES.iter().filter(|_| version >= 5))
        .chain(V6_TABLES.iter().filter(|_| version >= 6));
    for table in required {
        if !has(*table) {
            return Err(OperatorStoreError::CorruptSchema(
                "operator store is missing a table required by its schema version",
            ));
        }
    }
    let unexpected = V2_TABLES
        .iter()
        .filter(|_| version < 2)
        .chain(V3_TABLES.iter().filter(|_| version < 3))
        .chain(V4_TABLES.iter().filter(|_| version < 4))
        .chain(V5_TABLES.iter().filter(|_| version < 5))
        .chain(V6_TABLES.iter().filter(|_| version < 6));
    for table in unexpected {
        if has(*table) {
            return Err(OperatorStoreError::CorruptSchema(
                "operator store has a table from a newer schema version",
            ));
        }
    }
    Ok(())
}

// Initialize exactly once, either migrating an enabled v6 policy or first enabling
// an inactive store. Neither policy epochs nor reopening replenish this allowance.
fn initialize_document_accounting(tx: &Transaction<'_>) -> Result<(), OperatorStoreError> {
    let initialized: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM meta WHERE key='document_global_baseline')",
        [],
        |r| r.get(0),
    )?;
    if initialized {
        return validate_document_accounting(tx);
    }
    let partial: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM meta WHERE key IN ('document_global_cutoff','document_recovery_credit:legacy'))", [], |r| r.get(0))?;
    if partial {
        return Err(OperatorStoreError::CorruptSchema(
            "incomplete global document baseline",
        ));
    }
    // One finite envelope per retained legacy document identity. The typed API
    // limits each envelope (including ledger rows) to 324 units.
    let identities: usize = tx.query_row(
        "SELECT count(*) FROM (SELECT DISTINCT namespace,id FROM operator_documents)",
        [],
        |r| r.get(0),
    )?;
    let executions: usize = tx.query_row("SELECT count(*) FROM executions", [], |r| r.get(0))?;
    let credit = identities
        .checked_mul(324)
        .and_then(|n| executions.checked_mul(4).and_then(|e| n.checked_add(e)))
        .ok_or(OperatorStoreError::CorruptSchema(
            "legacy document credit overflow",
        ))?;
    tx.execute(
        "INSERT INTO meta(key,value) VALUES('document_recovery_credit:legacy',?1)",
        [credit.to_string()],
    )?;
    tx.execute("INSERT INTO meta(key,value) VALUES('document_global_cutoff',(SELECT COALESCE(max(seq),0) FROM changes))", [])?;
    tx.execute("INSERT OR IGNORE INTO meta(key,value) SELECT 'document_recovery_credit:'||execution_id,'4' FROM executions WHERE json_extract(payload,'$.action_id') NOT LIKE 'recovery:%' AND (json_extract(payload,'$.recovery.available')=1 OR EXISTS(SELECT 1 FROM terminal_reservations r WHERE r.producer='resource_apply' AND r.producer_id=execution_id))", [])?;
    let baseline = document_charge(tx)?
        .checked_add(1)
        .ok_or(OperatorStoreError::CorruptSchema(
            "document baseline overflow",
        ))?;
    tx.execute(
        "INSERT INTO meta(key,value) VALUES('document_global_baseline',?1)",
        [baseline.to_string()],
    )?;
    validate_document_accounting(tx)
}

fn validate_document_accounting(connection: &Connection) -> Result<(), OperatorStoreError> {
    let mut statement = connection.prepare("SELECT key,value FROM meta WHERE key IN ('document_global_baseline','document_global_cutoff','document_policy_epoch') OR key LIKE 'document_recovery_credit:%'")?;
    let mut rows = statement.query([])?;
    let mut baseline = false;
    let mut cutoff = false;
    let mut legacy = false;
    while let Some(row) = rows.next()? {
        let key: String = row.get(0)?;
        let value: String = row.get(1)?;
        let number = value
            .parse::<usize>()
            .map_err(|_| OperatorStoreError::CorruptSchema("invalid document accounting value"))?;
        if number.to_string() != value || number > i64::MAX as usize {
            return Err(OperatorStoreError::CorruptSchema(
                "noncanonical document accounting value",
            ));
        }
        baseline |= key == "document_global_baseline";
        cutoff |= key == "document_global_cutoff";
        legacy |= key == "document_recovery_credit:legacy";
    }
    let policy: Option<String> = connection
        .query_row(
            "SELECT value FROM meta WHERE key='document_growth_policy'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    let policy: Option<super::operator_observability::DocumentGrowthPolicy> =
        policy.map(|p| decode(&p)).transpose()?.flatten();
    if baseline != cutoff || baseline != legacy || (policy.is_some() && !baseline) {
        return Err(OperatorStoreError::CorruptSchema(
            "incomplete global document accounting",
        ));
    }
    if let Some(policy) = policy {
        if !(1..=100_000).contains(&policy.max_rows_per_namespace) {
            return Err(OperatorStoreError::CorruptSchema("invalid document policy"));
        }
        document_bound(connection, &policy)?;
    }
    Ok(())
}

// All namespaces share this budget. Retained metadata and spent reservation ledger
// entries remain charged; spending a reservation exchanges future rows for actual rows.
fn document_charge(connection: &Connection) -> Result<usize, OperatorStoreError> {
    let base: usize = connection.query_row(
        "SELECT (SELECT count(*) FROM operator_documents)+(SELECT count(*) FROM meta)+(SELECT count(*)+COALESCE(sum(remaining),0)+COALESCE(sum(CASE WHEN producer='supervision_completion' THEN consumed ELSE 0 END),0) FROM terminal_reservations)+(SELECT COALESCE(sum(CAST(value AS INTEGER)),0) FROM meta WHERE key LIKE 'document_recovery_credit:%')",
        [], |row| row.get(0),
    )?;
    let payload: Option<String> = connection.query_row("SELECT payload FROM operator_documents WHERE namespace='editorial' AND id='attention-v1' ORDER BY revision DESC LIMIT 1", [], |r| r.get(0)).optional()?;
    let obligations = payload
        .map(|p| decode::<super::editorial::EditorialState>(&p).map(|s| editorial_obligations(&s)))
        .transpose()?
        .unwrap_or(0);
    base.checked_add(obligations)
        .ok_or(OperatorStoreError::CorruptSchema(
            "document charge overflow",
        ))
}

fn editorial_obligations(state: &super::editorial::EditorialState) -> usize {
    state
        .receipts
        .iter()
        .map(|r| {
            usize::from(r.speech_id.is_none())
                + usize::from(r.claimed_surface.is_none())
                + usize::from(r.feedback.is_none())
        })
        .sum()
}

fn document_bound(
    connection: &Connection,
    policy: &super::operator_observability::DocumentGrowthPolicy,
) -> Result<usize, OperatorStoreError> {
    let baseline: String = connection.query_row(
        "SELECT value FROM meta WHERE key='document_global_baseline'",
        [],
        |row| row.get(0),
    )?;
    let baseline: usize = baseline
        .parse()
        .map_err(|_| OperatorStoreError::CorruptSchema("invalid global document baseline"))?;
    baseline
        .checked_add(
            policy
                .max_rows_per_namespace
                .checked_mul(super::operator_observability::GLOBAL_DOCUMENT_BUDGET_MULTIPLIER)
                .ok_or(OperatorStoreError::CorruptSchema(
                    "global document budget overflow",
                ))?,
        )
        .ok_or(OperatorStoreError::CorruptSchema(
            "global document budget overflow",
        ))
}

fn admit_document_charge(
    connection: &Connection,
    policy: Option<&super::operator_observability::DocumentGrowthPolicy>,
    additional: usize,
) -> Result<(), OperatorStoreError> {
    if let Some(policy) = policy {
        if document_charge(connection)?
            .checked_add(additional)
            .ok_or(OperatorStoreError::Conflict("document accounting overflow"))?
            > document_bound(connection, policy)?
        {
            return Err(OperatorStoreError::Conflict("global document capacity reached; reserved completion retained; explicitly raise policy capacity"));
        }
    }
    Ok(())
}

fn reserve_terminal_documents_in_tx(
    tx: &Transaction<'_>,
    policy: Option<&super::operator_observability::DocumentGrowthPolicy>,
    producer: &str,
    id: &str,
    slots: &[(&str, String, usize)],
    exemption: Option<&str>,
) -> Result<(), OperatorStoreError> {
    document_key(producer, id)?;
    if slots.is_empty() || slots.len() > 3 {
        return Err(OperatorStoreError::InvalidInput(
            "invalid terminal reservation",
        ));
    }
    let mut identities = std::collections::BTreeSet::new();
    let mut budgets = std::collections::BTreeMap::<&str, usize>::new();
    for (namespace, document_id, budget) in slots {
        document_key(namespace, document_id)?;
        let valid = match producer {
            "resource_apply" => *namespace == "resource_receipt" && document_id == id,
            "recovery_grant" => *namespace == "action_grant",
            "native_installation" => {
                (*namespace == "improvement_installation" && document_id == id)
                    || *namespace == "improvement_native_canary"
            }
            "host_release" => *namespace == "improvement_host_release" && document_id == id,
            "artifact_stage" => *namespace == "improvement_artifacts" && document_id == id,
            "artifact_cleanup" => *namespace == "improvement_artifact_cleanup" && document_id == id,
            "supervision" => {
                matches!(*namespace, "supervision_mandates" | "supervision_links")
                    && document_id == id
            }
            // A completion atomically retains the outcome, work item, and artifact
            // records it will publish. Its sole ledger slot is deliberately typed:
            // the report identity is the mandate identity and it has exactly three
            // charged units, one for each relational record.
            "supervision_completion" => {
                *namespace == "supervision_report" && document_id == id && *budget == 3
            }
            "editorial" => *namespace == "editorial" && document_id == "attention-v1",
            "fixed_evaluation" => match *namespace {
                "improvement_fixed_runs" | "improvement_comparisons" => document_id == id,
                "improvement_proofs" => document_id == &format!("fixed:{id}"),
                _ => false,
            },
            _ => false,
        };
        if !valid
            || !(1..=if producer == "supervision" { 320 } else { 16 }).contains(budget)
            || !identities.insert((*namespace, document_id.as_str()))
        {
            return Err(OperatorStoreError::InvalidInput(
                "invalid terminal slot coverage",
            ));
        }
        *budgets.entry(*namespace).or_default() += *budget;
    }
    let existing: usize = tx.query_row(
        "SELECT count(*) FROM terminal_reservations WHERE producer=?1 AND producer_id=?2",
        params![producer, id],
        |row| row.get(0),
    )?;
    if existing != 0 {
        for (namespace, document_id, budget) in slots {
            let saved: Option<usize> = tx.query_row(
                "SELECT budget FROM terminal_reservations WHERE producer=?1 AND producer_id=?2 AND namespace=?3 AND document_id=?4",
                params![producer, id, namespace, document_id], |row| row.get(0),
            ).optional()?;
            if saved != Some(*budget) {
                return Err(OperatorStoreError::Conflict("reservation identity changed"));
            }
        }
        if existing != slots.len() {
            return Err(OperatorStoreError::Conflict("reservation coverage changed"));
        }
        return Ok(());
    }
    let cost = slots.len() + slots.iter().map(|(_, _, n)| *n).sum::<usize>();
    if cost > 324 {
        return Err(OperatorStoreError::InvalidInput(
            "terminal producer envelope exceeds 324 units",
        ));
    }
    if exemption == Some("grandfathered_legacy") && policy.is_some() {
        let changed = tx.execute("UPDATE meta SET value=CAST(value AS INTEGER)-?1 WHERE key='document_recovery_credit:legacy' AND CAST(value AS INTEGER)>=?1", [cost])?;
        if changed != 1 {
            return Err(OperatorStoreError::Conflict(
                "grandfather completion reserve exhausted",
            ));
        }
    }
    admit_document_charge(tx, policy, cost)?;
    if exemption.is_none() {
        if let Some(policy) = policy {
            let entries: usize = tx.query_row(
                "SELECT count(*) FROM terminal_reservations WHERE producer=?1",
                [producer],
                |row| row.get(0),
            )?;
            if entries.saturating_add(slots.len()) > policy.max_rows_per_namespace {
                return Err(OperatorStoreError::Conflict(
                    "terminal ledger admission capacity reached",
                ));
            }
            for (namespace, budget) in budgets {
                let rows: usize = tx.query_row(
                    "SELECT count(*) FROM operator_documents WHERE namespace=?1",
                    [namespace],
                    |row| row.get(0),
                )?;
                let outstanding: usize = tx.query_row(
                    "SELECT COALESCE(sum(remaining),0) FROM terminal_reservations WHERE namespace=?1",
                    [namespace], |row| row.get(0),
                )?;
                if rows.saturating_add(outstanding).saturating_add(budget)
                    > policy.max_rows_per_namespace
                {
                    return Err(OperatorStoreError::Conflict(
                        "terminal document admission capacity reached",
                    ));
                }
            }
        }
    }
    let coverage = exemption.unwrap_or(if policy.is_some() {
        "reserved"
    } else {
        "grandfathered_inactive"
    });
    for (namespace, document_id, budget) in slots {
        tx.execute(
            "INSERT INTO terminal_reservations(producer,producer_id,namespace,document_id,budget,remaining,consumed,coverage,state) VALUES(?1,?2,?3,?4,?5,?5,0,?6,'reserved')",
            params![producer, id, namespace, document_id, budget, coverage],
        )?;
    }
    Ok(())
}

fn next_seq(tx: &Transaction<'_>) -> Result<i64, OperatorStoreError> {
    tx.execute("INSERT INTO changes DEFAULT VALUES", [])?;
    Ok(tx.last_insert_rowid())
}

fn finish_reservation_in_tx(
    tx: &Transaction<'_>,
    terminal: Option<(&str, &str)>,
) -> Result<(), OperatorStoreError> {
    if let Some((producer, id)) = terminal {
        tx.execute(
            "UPDATE terminal_reservations SET remaining=0,state='finished' WHERE producer=?1 AND producer_id=?2 AND state='reserved'",
            params![producer, id],
        )?;
    }
    Ok(())
}

fn stored_immutable_in_tx<T: DeserializeOwned>(
    tx: &Transaction<'_>,
    table: &str,
    where_clause: &str,
    params: impl rusqlite::Params,
    revision: u64,
) -> Result<Option<Stored<T>>, OperatorStoreError> {
    let sql = format!("SELECT payload,change_seq FROM {table} WHERE {where_clause}");
    tx.query_row(&sql, params, |row| {
        Ok(Stored {
            value: decode::<T>(&row.get::<_, String>(0)?)
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?,
            revision,
            change_seq: row.get::<_, i64>(1)? as u64,
        })
    })
    .optional()
    .map_err(Into::into)
}

fn stored_mutable_in_tx<T: DeserializeOwned>(
    tx: &Transaction<'_>,
    table: &str,
    id: &str,
) -> Result<Option<Stored<T>>, OperatorStoreError> {
    let sql = format!("SELECT payload,revision,change_seq FROM {table} WHERE id=?1");
    tx.query_row(&sql, [id], |row| {
        Ok(Stored {
            value: decode::<T>(&row.get::<_, String>(0)?)
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?,
            revision: row.get::<_, i64>(1)? as u64,
            change_seq: row.get::<_, i64>(2)? as u64,
        })
    })
    .optional()
    .map_err(Into::into)
}

fn stored_artifact_in_tx(
    tx: &Transaction<'_>,
    artifact: &ArtifactRevision,
) -> Result<Stored<ArtifactRevision>, OperatorStoreError> {
    stored_immutable_in_tx(
        tx,
        "artifacts",
        "artifact_id=?1 AND revision=?2",
        params![artifact.artifact_id, artifact.revision],
        artifact.revision as u64,
    )?
    .ok_or(OperatorStoreError::Conflict(
        "existing outcome has no linked artifact revision",
    ))
}

fn upsert_work_item_in_tx(
    tx: &Transaction<'_>,
    item: &WorkItem,
    payload: &str,
    expected_revision: Option<u64>,
) -> Result<Stored<WorkItem>, OperatorStoreError> {
    let current: Option<i64> = tx
        .query_row(
            "SELECT revision FROM work_items WHERE id=?1",
            [&item.id],
            |row| row.get(0),
        )
        .optional()?;
    if current.is_some() && expected_revision.is_none() {
        return Err(OperatorStoreError::Conflict(
            "updating a work item requires an expected revision",
        ));
    }
    if let Some(expected) = expected_revision {
        if current.map(|revision| revision as u64) != Some(expected) {
            return Err(OperatorStoreError::StaleRevision {
                current: current.unwrap_or(0) as u64,
            });
        }
    }
    if current.is_none() && expected_revision.is_some() {
        return Err(OperatorStoreError::StaleRevision {
            current: 0,
        });
    }
    let revision = current
        .unwrap_or(0)
        .checked_add(1)
        .ok_or(OperatorStoreError::InvalidInput(
            "work item revision overflow",
        ))?;
    let change_seq = next_seq(tx)?;
    tx.execute(
        "INSERT INTO work_items(id,payload,revision,change_seq) VALUES(?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload,revision=excluded.revision,change_seq=excluded.change_seq",
        params![item.id, payload, revision, change_seq],
    )?;
    Ok(Stored {
        value: item.clone(),
        revision: revision as u64,
        change_seq: change_seq as u64,
    })
}

fn retire_workflow_siblings_in_tx(
    tx: &Transaction<'_>,
    outcome: &RunOutcome,
    item: &mut WorkItem,
) -> Result<(), OperatorStoreError> {
    use refact_buddy_core::operator_types::WorkItemState;

    if item.subject.resource_kind != "workflow"
        || outcome.workflow.as_deref() != Some(item.subject.resource_id.as_str())
    {
        return Ok(());
    }
    let outcome_time = chrono::DateTime::parse_from_rfc3339(&outcome.started_at)
        .map_err(|_| OperatorStoreError::InvalidInput("invalid workflow start timestamp"))?;
    let mut records = vec![(
        Stored {
            value: item.clone(),
            revision: 0,
            change_seq: 0,
        },
        super::supersede::Candidate {
            id: item.id.clone(),
            status: outcome.status.clone(),
            observed_at: outcome_time,
            eligible: true,
        },
    )];
    let sibling_rows: Vec<(Stored<WorkItem>, Option<RunOutcome>)> = {
        let mut statement = tx.prepare(
            "SELECT payload,revision,change_seq FROM work_items WHERE id<>?1 AND json_extract(payload,'$.project.project_id')=?2 AND json_extract(payload,'$.project.canonical_root')=?3 AND json_extract(payload,'$.subject.resource_kind')='workflow' AND json_extract(payload,'$.subject.resource_id')=?4 AND json_extract(payload,'$.state') IN ('needs_attention','watching','blocked')",
        )?;
        let rows = statement.query_map(
            rusqlite::params![
                item.id,
                item.project.project_id,
                item.project.canonical_root,
                item.subject.resource_id
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, u64>(1)?,
                    row.get::<_, u64>(2)?,
                ))
            },
        )?;
        let mut candidates = Vec::new();
        for row in rows {
            let (payload, revision, change_seq) = row?;
            let sibling: WorkItem = decode(&payload)?;
            if sibling.project != item.project
                || sibling.subject.resource_kind != "workflow"
                || sibling.subject.resource_id != item.subject.resource_id
            {
                continue;
            }
            let latest: Option<String> = tx
                .query_row(
                    "SELECT payload FROM run_outcomes WHERE json_extract(payload,'$.work_id')=?1 AND json_extract(payload,'$.workflow')=?2 ORDER BY change_seq DESC LIMIT 1",
                    rusqlite::params![sibling.id, item.subject.resource_id],
                    |row| row.get(0),
                )
                .optional()?;
            candidates.push((
                Stored {
                    value: sibling,
                    revision,
                    change_seq,
                },
                latest.map(|payload| decode(&payload)).transpose()?,
            ));
        }
        candidates
    };

    for (sibling, latest) in sibling_rows {
        let (status, coverage, observed_at) = if let Some(run) = latest {
            (
                run.status,
                run.coverage,
                chrono::DateTime::parse_from_rfc3339(&run.started_at).ok(),
            )
        } else {
            let status = match &sibling.value.state {
                WorkItemState::NeedsAttention | WorkItemState::Watching => {
                    refact_buddy_core::operator_types::RunStatus::Inconclusive
                }
                WorkItemState::Blocked => refact_buddy_core::operator_types::RunStatus::Blocked,
                _ => continue,
            };
            (
                status,
                sibling.value.scope.clone().unwrap_or_default(),
                None,
            )
        };
        if super::supersede::coverage_class(&coverage)
            != super::supersede::coverage_class(&outcome.coverage)
        {
            continue;
        }
        let observed_at = match observed_at {
            Some(value) => value,
            None => {
                match chrono::DateTime::parse_from_rfc3339(&sibling.value.last_material_change_at) {
                    Ok(value) => value,
                    Err(_) => continue,
                }
            }
        };
        records.push((
            sibling.clone(),
            super::supersede::Candidate {
                id: sibling.value.id.clone(),
                status,
                observed_at,
                eligible: true,
            },
        ));
    }

    let candidates: Vec<_> = records
        .iter()
        .map(|(_, candidate)| candidate.clone())
        .collect();
    let retire_ids = super::supersede::retire_ids(&candidates);
    for (candidate, _) in records {
        if !retire_ids.contains(&candidate.value.id) {
            continue;
        }
        if candidate.value.id == item.id {
            item.state = WorkItemState::Cancelled;
        } else {
            let mut retired = candidate.value;
            retired.state = WorkItemState::Cancelled;
            upsert_work_item_in_tx(tx, &retired, &encode(&retired)?, Some(candidate.revision))?;
        }
    }
    Ok(())
}

fn insert_outcome_in_tx(
    tx: &Transaction<'_>,
    outcome: &RunOutcome,
    payload: &str,
) -> Result<Stored<RunOutcome>, OperatorStoreError> {
    let change_seq = next_seq(tx)?;
    tx.execute(
        "INSERT INTO run_outcomes(run_id,payload,change_seq) VALUES(?1,?2,?3)",
        params![outcome.run_id, payload, change_seq],
    )?;
    Ok(Stored {
        value: outcome.clone(),
        revision: 0,
        change_seq: change_seq as u64,
    })
}

fn insert_artifact_in_tx(
    tx: &Transaction<'_>,
    artifact: &ArtifactRevision,
    payload: &str,
) -> Result<Stored<ArtifactRevision>, OperatorStoreError> {
    if let Some(existing) = stored_immutable_in_tx::<ArtifactRevision>(
        tx,
        "artifacts",
        "artifact_id=?1 AND revision=?2",
        params![artifact.artifact_id, artifact.revision],
        artifact.revision as u64,
    )? {
        if existing.value != *artifact {
            return Err(OperatorStoreError::Conflict(
                "immutable record id already exists with different payload",
            ));
        }
        return Ok(existing);
    }
    let change_seq = next_seq(tx)?;
    tx.execute(
        "INSERT INTO artifacts(artifact_id,revision,payload,change_seq) VALUES(?1,?2,?3,?4)",
        params![artifact.artifact_id, artifact.revision, payload, change_seq],
    )?;
    Ok(Stored {
        value: artifact.clone(),
        revision: artifact.revision as u64,
        change_seq: change_seq as u64,
    })
}

fn private_child_directory(parent: &Path, child: &str) -> Result<PathBuf, OperatorStoreError> {
    let path = parent.join(child);
    match fs::symlink_metadata(&path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(OperatorStoreError::InvalidInput(
                    "operator store directory must be a real directory",
                ));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(&path)?;
            #[cfg(unix)]
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
        }
        Err(error) => return Err(error.into()),
    }
    let canonical = dunce::canonicalize(&path)?;
    if !canonical.starts_with(parent) {
        return Err(OperatorStoreError::InvalidInput(
            "operator store directory escapes project root",
        ));
    }
    Ok(canonical)
}

fn sidecar_path(database: &Path, suffix: &str) -> PathBuf {
    PathBuf::from(format!("{}{}", database.to_string_lossy(), suffix))
}

fn reject_symlink_or_permissive_file(
    path: &Path,
    label: &'static str,
) -> Result<(), OperatorStoreError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(OperatorStoreError::InvalidInput(
                    "operator store file is unsafe",
                ));
            }
            #[cfg(unix)]
            if metadata.permissions().mode() & 0o077 != 0 {
                return Err(OperatorStoreError::InvalidInput(label));
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn restrict_file(file: &fs::File) -> Result<(), OperatorStoreError> {
    #[cfg(unix)]
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    Ok(())
}

fn write_lock_owner(
    file: &mut fs::File,
    owner: &OperatorLockOwner,
) -> Result<(), OperatorStoreError> {
    let payload = serde_json::to_vec(owner)?;
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&payload)?;
    file.write_all(b"\n")?;
    file.flush()?;
    Ok(())
}

fn read_lock_owner(file: &mut fs::File) -> Option<OperatorLockOwner> {
    file.seek(SeekFrom::Start(0)).ok()?;
    let mut payload = String::new();
    file.read_to_string(&mut payload).ok()?;
    serde_json::from_str(payload.trim()).ok()
}

fn restrict_store_files(path: &Path) -> Result<(), OperatorStoreError> {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let file_path = sidecar_path(path, suffix);
        if let Ok(file) = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&file_path)
        {
            restrict_file(&file)?;
        }
    }
    Ok(())
}

fn bounded(limit: usize) -> Result<(), OperatorStoreError> {
    if limit == 0 || limit > MAX_PAGE_SIZE {
        Err(OperatorStoreError::InvalidInput(
            "page size must be between 1 and 200",
        ))
    } else {
        Ok(())
    }
}
fn validate_identity(i: &ProjectIdentity) -> Result<(), OperatorStoreError> {
    if i.project_id.trim().is_empty() || i.canonical_root.trim().is_empty() {
        Err(OperatorStoreError::InvalidInput(
            "project identity fields must not be empty",
        ))
    } else {
        Ok(())
    }
}
fn encode<T: Serialize>(value: &T) -> Result<String, OperatorStoreError> {
    let s = serde_json::to_string(value)?;
    if s.len() > MAX_RECORD_BYTES {
        Err(OperatorStoreError::RecordTooLarge)
    } else {
        Ok(s)
    }
}
fn decode<T: DeserializeOwned>(s: &str) -> Result<T, OperatorStoreError> {
    Ok(serde_json::from_str(s)?)
}

fn contains_trajectory_reference(value: &serde_json::Value, id: &str) -> bool {
    match value {
        serde_json::Value::String(text) => {
            text == id
                || text
                    .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
                    .any(|part| part == id)
        }
        serde_json::Value::Array(values) => values
            .iter()
            .any(|value| contains_trajectory_reference(value, id)),
        serde_json::Value::Object(values) => values
            .values()
            .any(|value| contains_trajectory_reference(value, id)),
        _ => false,
    }
}

pub(crate) fn contains_worktree_reference(
    value: &serde_json::Value,
    id: &str,
    root: &str,
    branch: Option<&str>,
) -> bool {
    match value {
        serde_json::Value::String(text) => {
            contains_trajectory_reference(value, id)
                || text
                    .split(|c: char| {
                        c.is_whitespace() || matches!(c, '"' | '\'' | '`' | '(' | ')' | '<' | '>')
                    })
                    .map(|part| part.strip_prefix("file://").unwrap_or(part))
                    .any(|part| {
                        part == root
                            || part.strip_prefix(root).is_some_and(|suffix| {
                                suffix.starts_with('/') || suffix.starts_with('\\')
                            })
                            || branch == Some(part)
                    })
        }
        serde_json::Value::Array(values) => values
            .iter()
            .any(|value| contains_worktree_reference(value, id, root, branch)),
        serde_json::Value::Object(values) => values
            .values()
            .any(|value| contains_worktree_reference(value, id, root, branch)),
        _ => false,
    }
}

fn immutable_existing<T: Serialize + DeserializeOwned + Clone>(
    tx: &Transaction<'_>,
    table: &str,
    where_clause: &str,
    params: impl rusqlite::Params,
    value: &T,
    revision: u32,
) -> Result<Stored<T>, OperatorStoreError> {
    let sql = format!("SELECT payload,change_seq FROM {table} WHERE {where_clause}");
    let (stored, seq): (String, i64) =
        tx.query_row(&sql, params, |r| Ok((r.get(0)?, r.get(1)?)))?;
    if stored != encode(value)? {
        return Err(OperatorStoreError::Conflict(
            "immutable record id already exists with different payload",
        ));
    }
    Ok(Stored {
        value: value.clone(),
        revision: revision as u64,
        change_seq: seq as u64,
    })
}
fn page<T: DeserializeOwned>(
    rows: rusqlite::MappedRows<
        '_,
        impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<(String, i64, i64)>,
    >,
    limit: usize,
) -> Result<Page<Stored<T>>, OperatorStoreError> {
    let mut entries = Vec::new();
    for row in rows {
        let (p, r, s) = row?;
        entries.push(Stored {
            value: decode(&p)?,
            revision: r as u64,
            change_seq: s as u64,
        });
    }
    let next_after = if entries.len() > limit {
        entries.pop();
        entries.last().map(|v| v.change_seq)
    } else {
        None
    };
    Ok(Page {
        entries,
        next_after,
    })
}

#[cfg(test)]
#[path = "work_order_tests.rs"]
mod work_order_tests;

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::TempDir;
    #[cfg(unix)]
    use std::os::unix::fs::{symlink, PermissionsExt};
    fn project(root: &Path) -> ProjectIdentity {
        ProjectIdentity {
            project_id: "p".into(),
            canonical_root: dunce::canonicalize(root)
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        }
    }
    fn fixture() -> serde_json::Value {
        serde_json::from_str(include_str!(
            "../../crates/refact-buddy-core/tests/fixtures/operator_contract_v1.json"
        ))
        .unwrap()
    }

    #[test]
    fn schema6_enabled_policy_migrates_once_without_rewriting_history() {
        use super::super::operator_observability::DocumentGrowthPolicy;
        let root = TempDir::new().unwrap();
        let identity = project(root.path());
        let mut store = OperatorStore::open(root.path(), &identity).unwrap();
        let value = serde_json::json!({"schema_version":1,"project":identity});
        let saved = store
            .put_document("improvement_fixed_runs", "old", &value, None)
            .unwrap();
        let policy = Some(DocumentGrowthPolicy {
            max_rows_per_namespace: 1,
            replace_editorial_aggregate: false,
        });
        store.connection.execute("INSERT INTO meta(key,value) VALUES('document_growth_policy',?1),('document_policy_epoch','9'),('document_policy_history:9',?1)", [encode(&policy).unwrap()]).unwrap();
        store
            .connection
            .pragma_update(None, "user_version", 6)
            .unwrap();
        drop(store);
        let mut store = OperatorStore::open(root.path(), &identity).unwrap();
        assert_eq!(store.policy_snapshot().unwrap().epoch, 9);
        assert_eq!(store.policy_snapshot().unwrap().policy, policy);
        assert_eq!(store.historical_document_policy(9).unwrap(), policy);
        let baseline: String = store
            .connection
            .query_row(
                "SELECT value FROM meta WHERE key='document_global_baseline'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        store
            .reserve_grandfathered_terminal_documents(
                "fixed_evaluation",
                "old",
                &[("improvement_fixed_runs", "old".into(), 1)],
            )
            .unwrap();
        let credit: String = store
            .connection
            .query_row(
                "SELECT value FROM meta WHERE key='document_recovery_credit:legacy'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        for _ in 0..3 {
            drop(store);
            store = OperatorStore::open(root.path(), &identity).unwrap();
            assert_eq!(user_version(&store.connection), 7);
            assert_eq!(
                store
                    .get_document_revision::<serde_json::Value>("improvement_fixed_runs", "old", 1)
                    .unwrap()
                    .unwrap(),
                saved
            );
            let current: (String,String) = store.connection.query_row("SELECT (SELECT value FROM meta WHERE key='document_global_baseline'),(SELECT value FROM meta WHERE key='document_recovery_credit:legacy')", [], |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
            assert_eq!(current, (baseline.clone(), credit.clone()));
        }
    }

    #[test]
    fn malformed_baseline_refuses_reopen_and_readonly_preview_without_repair() {
        use super::super::operator_observability::DocumentGrowthPolicy;
        for version in [6, 7] {
            let root = TempDir::new().unwrap();
            let identity = project(root.path());
            let mut store = OperatorStore::open(root.path(), &identity).unwrap();
            store
                .set_document_growth_policy(Some(DocumentGrowthPolicy {
                    max_rows_per_namespace: 1,
                    replace_editorial_aggregate: false,
                }))
                .unwrap();
            store
                .connection
                .execute(
                    "UPDATE meta SET value='-1' WHERE key='document_global_baseline'",
                    [],
                )
                .unwrap();
            store
                .connection
                .pragma_update(None, "user_version", version)
                .unwrap();
            let path = store.path().to_owned();
            drop(store);
            assert!(OperatorStore::read_document_policy_snapshot(root.path(), &identity).is_err());
            assert!(OperatorStore::open(root.path(), &identity).is_err());
            let connection = Connection::open(path).unwrap();
            assert_eq!(user_version(&connection), version);
            let value: String = connection
                .query_row(
                    "SELECT value FROM meta WHERE key='document_global_baseline'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(value, "-1");
        }
    }

    #[test]
    fn global_unknown_namespace_flood_is_bounded_and_same_payload_is_free() {
        use super::super::operator_observability::DocumentGrowthPolicy;
        let root = TempDir::new().unwrap();
        let identity = project(root.path());
        let mut store = OperatorStore::open(root.path(), &identity).unwrap();
        store
            .set_document_growth_policy(Some(DocumentGrowthPolicy {
                max_rows_per_namespace: 1,
                replace_editorial_aggregate: false,
            }))
            .unwrap();
        let value = serde_json::json!({"schema_version":1,"project":identity,"terminal":true});
        let first = store
            .put_document("unknown_0", "item", &value, None)
            .unwrap();
        assert_eq!(
            store
                .put_document("unknown_0", "item", &value, Some(first.revision))
                .unwrap(),
            first
        );
        let mut admitted = 1;
        for n in 1..1000 {
            if store
                .put_document(&format!("unknown_{n}"), "item", &value, None)
                .is_err()
            {
                break;
            }
            admitted += 1;
        }
        assert!(admitted < 1000);
        let summary = store.diagnostic_summary().unwrap().document_growth;
        assert!(!summary.unclassified_namespaces_may_grow);
        assert!(summary.charged_document_units <= summary.global_document_bound.unwrap());
        let before = store.document_policy_charge().unwrap();
        assert!(store
            .put_document("one_more", "item", &value, None)
            .is_err());
        assert_eq!(before, store.document_policy_charge().unwrap());
        drop(store);
        let mut store = OperatorStore::open(root.path(), &identity).unwrap();
        assert!(store
            .put_document("after_restart", "item", &value, None)
            .is_err());
        assert_eq!(
            store
                .get_document::<serde_json::Value>("unknown_0", "item")
                .unwrap()
                .unwrap(),
            first
        );
    }

    #[test]
    fn global_reserved_completion_survives_flood_and_cannot_be_spoofed() {
        use super::super::operator_observability::DocumentGrowthPolicy;
        let root = TempDir::new().unwrap();
        let identity = project(root.path());
        let mut store = OperatorStore::open(root.path(), &identity).unwrap();
        store
            .set_document_growth_policy(Some(DocumentGrowthPolicy {
                max_rows_per_namespace: 2,
                replace_editorial_aggregate: false,
            }))
            .unwrap();
        store
            .reserve_native_documents(
                "resource_apply",
                "reserved",
                &[("resource_receipt", "reserved".into(), 1)],
            )
            .unwrap();
        let value = serde_json::json!({"schema_version":1,"project":identity});
        for n in 0..1000 {
            if store
                .put_document(&format!("flood_{n}"), "row", &value, None)
                .is_err()
            {
                break;
            }
        }
        assert!(store
            .put_document("resource_receipt", "reserved", &value, None)
            .is_err());
        let before = store.document_policy_charge().unwrap();
        store
            .put_reserved_document(
                "resource_apply",
                "reserved",
                "resource_receipt",
                "reserved",
                &value,
                None,
            )
            .unwrap();
        assert_eq!(store.document_policy_charge().unwrap(), before);
        assert!(store
            .put_reserved_document(
                "resource_apply",
                "other",
                "resource_receipt",
                "reserved",
                &value,
                None
            )
            .is_err());
        assert!(store
            .reserve_native_documents(
                "resource_apply",
                "reserved",
                &[("resource_receipt", "reserved".into(), 2)]
            )
            .is_err());
    }

    #[test]
    fn corrupt_policy_refuses_growth_without_rewriting_evidence() {
        let root = TempDir::new().unwrap();
        let identity = project(root.path());
        let mut store = OperatorStore::open(root.path(), &identity).unwrap();
        let value = serde_json::json!({"schema_version":1,"project":identity});
        store.put_document("unknown", "kept", &value, None).unwrap();
        store
            .connection
            .execute(
                "INSERT INTO meta(key,value) VALUES('document_growth_policy','{broken')",
                [],
            )
            .unwrap();
        assert!(store.put_document("unknown", "new", &value, None).is_err());
        assert!(store
            .get_document::<serde_json::Value>("unknown", "kept")
            .unwrap()
            .is_some());
    }

    #[test]
    fn reference_scan_protects_disconnected_history_and_refuses_corrupt_rows() {
        let dir = TempDir::new().unwrap();
        let identity = project(dir.path());
        let mut store = OperatorStore::open(dir.path(), &identity).unwrap();
        let original = serde_json::json!({"schema_version":1,"project":identity,"evidence":["trajectory:chat-a:1"]});
        let stored = store
            .put_document("improvement_proofs", "old-proof", &original, None)
            .unwrap();
        let changed = serde_json::json!({"schema_version":1,"project":identity,"evidence":[]});
        store
            .put_document(
                "improvement_proofs",
                "old-proof",
                &changed,
                Some(stored.revision),
            )
            .unwrap();
        assert!(store.trajectory_is_referenced("chat-a").unwrap());
        assert!(!store.trajectory_is_referenced("chat-ab").unwrap());
        store.connection.execute("INSERT INTO artifacts(artifact_id,revision,payload,change_seq) VALUES('disconnected',1,?1,999)", ["{\"evidence\":[\"chat-c\"]}"]).unwrap();
        assert!(store.trajectory_is_referenced("chat-c").unwrap());
        store
            .connection
            .execute(
                "UPDATE artifacts SET payload='not-json' WHERE artifact_id='disconnected'",
                [],
            )
            .unwrap();
        assert!(store.trajectory_is_referenced("missing-chat").is_err());
    }

    #[test]
    fn worktree_reference_scan_preserves_history_without_self_blocking_archive_drafts() {
        let dir = TempDir::new().unwrap();
        let identity = project(dir.path());
        let mut store = OperatorStore::open(dir.path(), &identity).unwrap();
        let draft = serde_json::json!({"schema_version":1,"project":identity,"payload":{"operation":"archive","project":identity,"worktree_id":"tree-a"}});
        store
            .put_document("resource_draft", "archive-draft", &draft, None)
            .unwrap();
        assert!(!store
            .worktree_is_referenced("tree-a", "/checkout/a", Some("candidate/a"))
            .unwrap());
        let proof = serde_json::json!({"schema_version":1,"project":identity,"evidence":["/checkout/a/src/lib.rs"]});
        let stored = store
            .put_document("improvement_proofs", "old-proof", &proof, None)
            .unwrap();
        let empty = serde_json::json!({"schema_version":1,"project":identity,"evidence":[]});
        store
            .put_document(
                "improvement_proofs",
                "old-proof",
                &empty,
                Some(stored.revision),
            )
            .unwrap();
        assert!(store
            .worktree_is_referenced("tree-a", "/checkout/a", Some("candidate/a"))
            .unwrap());
        assert!(!store
            .worktree_is_referenced("tree-b", "/checkout/ab", Some("candidate/ab"))
            .unwrap());
        let locator = serde_json::json!({"schema_version":1,"project":identity,"evidence":["Retained `file:///checkout/c/src/lib.rs`"]});
        store
            .put_document("native_evidence", "file-uri", &locator, None)
            .unwrap();
        assert!(store
            .worktree_is_referenced("tree-c", "/checkout/c", None)
            .unwrap());
        assert!(!store
            .worktree_is_referenced("tree-c-sibling", "/checkout/c-sibling", None)
            .unwrap());
        store.connection.execute("INSERT INTO artifacts(artifact_id,revision,payload,change_seq) VALUES('disconnected',1,?1,999)", ["{\"evidence\":[\"candidate/b\"]}"]).unwrap();
        assert!(store
            .worktree_is_referenced("tree-b", "/checkout/b", Some("candidate/b"))
            .unwrap());
        store
            .connection
            .execute(
                "UPDATE artifacts SET payload='not-json' WHERE artifact_id='disconnected'",
                [],
            )
            .unwrap();
        assert!(store
            .worktree_is_referenced("tree-d", "/checkout/d", None)
            .is_err());
    }

    fn work(id: &str, project: &ProjectIdentity) -> WorkItem {
        let mut v = fixture()["work_item"].clone();
        v["id"] = serde_json::json!(id);
        v["project"] = serde_json::to_value(project).unwrap();
        serde_json::from_value(v).unwrap()
    }
    fn run(id: &str, project: &ProjectIdentity) -> RunOutcome {
        let mut v = fixture()["run_outcome"].clone();
        v["run_id"] = serde_json::json!(id);
        v["project"] = serde_json::to_value(project).unwrap();
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn capacity_blocks_new_claims_not_duplicates_terminal_updates_or_restart() {
        use super::super::operator_observability::CapacityPolicy;
        let root = TempDir::new().unwrap();
        let project = project(root.path());
        let mut store = OperatorStore::open(root.path(), &project).unwrap();
        let active = execution("active", &project);
        store.claim_execution(&active).unwrap();
        store
            .set_capacity_policy(Some(CapacityPolicy {
                max_rows_per_table: 1,
                max_database_bytes: 1,
            }))
            .unwrap();
        assert!(matches!(
            store.claim_execution(&active).unwrap(),
            ExecutionClaim::Existing(_)
        ));
        assert!(store.claim_execution(&execution("new", &project)).is_err());
        store
            .update_execution_state("active", 1, ActionExecutionState::Running)
            .unwrap();
        drop(store);
        let mut store = OperatorStore::open(root.path(), &project).unwrap();
        let recovered = store.get_execution("active").unwrap().unwrap();
        assert_eq!(recovered.value.status, ActionExecutionState::OutcomeUnknown);
        assert_eq!(
            store
                .diagnostic_summary()
                .unwrap()
                .unknown_executions_lower_bound,
            1
        );
        store
            .update_execution_state(
                "active",
                recovered.revision,
                ActionExecutionState::Cancelled,
            )
            .unwrap();
        assert!(matches!(
            store.claim_execution(&active).unwrap(),
            ExecutionClaim::Existing(_)
        ));
    }

    fn execution(id: &str, project: &ProjectIdentity) -> ActionExecution {
        let mut v = fixture()["action_execution"].clone();
        v["execution_id"] = serde_json::json!(id);
        v["idempotency_key"] = serde_json::json!(format!("key-{id}"));
        v["project"] = serde_json::to_value(project).unwrap();
        serde_json::from_value(v).unwrap()
    }
    fn artifact(
        id: &str,
        revision: u32,
        run_id: &str,
        project: &ProjectIdentity,
    ) -> ArtifactRevision {
        let mut value = fixture()["artifact_revision"].clone();
        value["artifact_id"] = serde_json::json!(id);
        value["revision"] = serde_json::json!(revision);
        value["project"] = serde_json::to_value(project).unwrap();
        value["source_run_ids"] = serde_json::json!([run_id]);
        value["provenance"]["source_run_ids"] = serde_json::json!([run_id]);
        serde_json::from_value(value).unwrap()
    }
    fn linked_records(project: &ProjectIdentity) -> (RunOutcome, WorkItem, ArtifactRevision) {
        let mut outcome = run("compound-run", project);
        outcome.work_id = "compound-work".into();
        let artifact = artifact("compound-artifact", 1, &outcome.run_id, project);
        let mut item = work(&outcome.work_id, project);
        item.current_artifact_id = Some(artifact.artifact_id.clone());
        item.current_artifact_revision = Some(artifact.revision);
        (outcome, item, artifact)
    }

    #[test]
    fn reserved_supervision_publication_rolls_back_all_rows_and_charge_then_reopens() {
        use super::super::operator_observability::{CapacityPolicy, DocumentGrowthPolicy};
        let dir = TempDir::new().unwrap();
        let project = project(dir.path());
        let mut store = OperatorStore::open(dir.path(), &project).unwrap();
        // Completion owns all three terminal slots before the tight document
        // policy is enabled; policy initialization then accounts for them in
        // its baseline rather than trying to admit them under the new ceiling.
        store.reserve_supervision_completion("mandate").unwrap();
        store
            .set_document_growth_policy(Some(DocumentGrowthPolicy {
                max_rows_per_namespace: 1,
                replace_editorial_aggregate: false,
            }))
            .unwrap();
        let (outcome, item, artifact) = linked_records(&project);
        let completion = SupervisionCompletion {
            mandate_id: "mandate".into(),
            outcome: outcome.clone(),
        };
        store
            .set_capacity_policy(Some(CapacityPolicy {
                max_rows_per_table: 1,
                max_database_bytes: 1,
            }))
            .unwrap();
        let value = serde_json::json!({"schema_version":1,"project":project});
        for n in 0..1000 {
            if store
                .put_document(&format!("fill_{n}"), "value", &value, None)
                .is_err()
            {
                break;
            }
        }
        assert!(store.put_document("full", "value", &value, None).is_err());
        assert!(store
            .append_outcome_work_item_artifact(&outcome, &item, None, Some(&artifact))
            .is_err());
        let charge = store.document_policy_charge().unwrap();
        let seq = store.retention_preview().unwrap().sequence_high_watermark;
        store.connection.execute_batch("CREATE TEMP TRIGGER reject_reserved_artifact BEFORE INSERT ON artifacts BEGIN SELECT RAISE(ABORT, 'injected reserved publication failure'); END;").unwrap();
        assert!(matches!(
            store.append_outcome_work_item_artifact_owned(
                &outcome,
                &item,
                None,
                Some(&artifact),
                Some(&completion)
            ),
            Err(OperatorStoreError::Sql(_))
        ));
        store.require_supervision_completion("mandate").unwrap();
        assert!(store.get_run_outcome(&outcome.run_id).unwrap().is_none());
        assert!(store.get_work_item(&item.id).unwrap().is_none());
        assert!(store
            .get_artifact_revision(&artifact.artifact_id, Some(1))
            .unwrap()
            .is_none());
        assert_eq!(store.document_policy_charge().unwrap(), charge);
        assert_eq!(
            store.retention_preview().unwrap().sequence_high_watermark,
            seq
        );
        store
            .connection
            .execute_batch("DROP TRIGGER reject_reserved_artifact;")
            .unwrap();
        drop(store);
        let mut store = OperatorStore::open(dir.path(), &project).unwrap();
        store.require_supervision_completion("mandate").unwrap();
        assert_eq!(store.document_policy_charge().unwrap(), charge);
        let saved = store
            .append_outcome_work_item_artifact_owned(
                &outcome,
                &item,
                None,
                Some(&artifact),
                Some(&completion),
            )
            .unwrap();
        assert!(store.require_supervision_completion("mandate").is_err());
        assert_eq!(store.document_policy_charge().unwrap(), charge);
        assert!(store
            .put_document("still_full", "value", &value, None)
            .is_err());
        assert_eq!(
            store
                .append_outcome_work_item_artifact_owned(
                    &outcome,
                    &item,
                    None,
                    Some(&artifact),
                    Some(&completion)
                )
                .unwrap(),
            saved
        );
        drop(store);
        let store = OperatorStore::open(dir.path(), &project).unwrap();
        assert_eq!(
            store.get_run_outcome(&outcome.run_id).unwrap().unwrap(),
            saved.0
        );
        assert_eq!(store.get_work_item(&item.id).unwrap().unwrap(), saved.1);
        assert_eq!(store.document_policy_charge().unwrap(), charge);
        assert!(store.require_supervision_completion("mandate").is_err());
    }

    fn begun(
        project: &ProjectIdentity,
        id: &str,
    ) -> super::super::outcome_runtime::WorkflowOutcomeStart {
        let outcome = run(id, project);
        super::super::outcome_runtime::WorkflowOutcomeStart {
            run_id: id.into(),
            work_id: "workflow:check".into(),
            parent_run_id: None,
            project: project.clone(),
            worker: outcome.worker,
            origin: "buddy_workflow".into(),
            workflow: "check".into(),
            input_fingerprint: "check-input".into(),
            coverage: "check scope".into(),
            started_at: outcome.started_at,
        }
    }

    #[test]
    fn interrupted_runs_publish_retained_evidence_once_without_success() {
        use super::super::outcome_runtime::{WorkflowOutcomeCheckpoint, MAX_OUTCOME_EVIDENCE};
        use refact_buddy_core::operator_types::{RunStatus, WorkItemState};
        let dir = TempDir::new().unwrap();
        let project = project(dir.path());
        let mut store = OperatorStore::open(dir.path(), &project).unwrap();
        let start = begun(&project, "interrupted");
        store.begin_workflow(&start).unwrap();
        store.begin_workflow(&start).unwrap();
        let evidence = run("fixture", &project).evidence[0].clone();
        let checkpoint = WorkflowOutcomeCheckpoint::from_evidence((0..30).map(|index| {
            let mut evidence = evidence.clone();
            evidence.evidence_id = format!("checkpoint-{index:02}");
            evidence
        }));
        store
            .checkpoint_workflow(&start, checkpoint.clone())
            .unwrap();
        store.begin_workflow(&start).unwrap();
        let empty = begun(&project, "empty-interruption");
        store.begin_workflow(&empty).unwrap();
        drop(store);
        let store = OperatorStore::open(dir.path(), &project).unwrap();
        let saved = store.get_run_outcome(&start.run_id).unwrap().unwrap();
        assert_eq!(saved.value.status, RunStatus::Partial);
        assert_eq!(saved.value.evidence, checkpoint.evidence);
        assert_eq!(saved.value.evidence.len(), MAX_OUTCOME_EVIDENCE);
        assert_eq!(
            saved.value.failure_class.as_deref(),
            Some("engine_interrupted")
        );
        assert!(saved.value.effects.is_empty());
        assert!(store.active_run(&start.run_id).unwrap().is_none());
        let blocked = store.get_run_outcome(&empty.run_id).unwrap().unwrap();
        assert_eq!(blocked.value.status, RunStatus::Blocked);
        let work = store.get_work_item(&saved.value.work_id).unwrap().unwrap();
        assert!(matches!(
            work.value.state,
            WorkItemState::NeedsAttention | WorkItemState::Blocked
        ));
        let report = store
            .artifact_for_run(work.value.current_artifact_id.as_deref().unwrap(), &saved)
            .unwrap()
            .unwrap();
        assert!(report.value.body.contains("No effects were replayed"));
        assert!(report.value.body.contains("unknown"));
        assert_eq!(report.value.evidence, checkpoint.evidence);
        assert_eq!(
            store
                .list_work_outcomes(&work.value.id, None, 10)
                .unwrap()
                .entries
                .len(),
            2
        );
        drop(store);
        let store = OperatorStore::open(dir.path(), &project).unwrap();
        assert_eq!(
            store.get_run_outcome(&start.run_id).unwrap().unwrap(),
            saved
        );
        assert_eq!(
            store.get_run_outcome(&empty.run_id).unwrap().unwrap(),
            blocked
        );
        assert_eq!(store.get_work_item(&work.value.id).unwrap().unwrap(), work);
    }

    #[test]
    fn failed_terminal_transaction_retains_active_run_and_finalized_run_stays_final() {
        use super::super::outcome_runtime::{
            build_workflow_outcome, WorkflowOutcomeCheckpoint, WorkflowTerminal,
        };
        let dir = TempDir::new().unwrap();
        let project = project(dir.path());
        let mut store = OperatorStore::open(dir.path(), &project).unwrap();
        let start = begun(&project, "transaction");
        store.begin_workflow(&start).unwrap();
        let outcome = build_workflow_outcome(
            start.clone(),
            WorkflowOutcomeCheckpoint::from_evidence([]),
            WorkflowTerminal::Completed,
            "2026-09-05T00:01:00Z".into(),
        );
        let mut item = work(&outcome.work_id, &project);
        let artifact = artifact("transaction-artifact", 1, &outcome.run_id, &project);
        item.current_artifact_id = Some(artifact.artifact_id.clone());
        item.current_artifact_revision = Some(1);
        store.connection.execute_batch("CREATE TEMP TRIGGER reject_artifact BEFORE INSERT ON artifacts BEGIN SELECT RAISE(ABORT, 'injected publication failure'); END;").unwrap();
        assert!(store
            .append_outcome_work_item_artifact(&outcome, &item, None, Some(&artifact))
            .is_err());
        assert!(store.active_run(&start.run_id).unwrap().is_some());
        assert!(store.get_run_outcome(&start.run_id).unwrap().is_none());
        assert!(store.get_work_item(&item.id).unwrap().is_none());
        store
            .connection
            .execute_batch("DROP TRIGGER reject_artifact;")
            .unwrap();
        let saved = store
            .append_outcome_work_item_artifact(&outcome, &item, None, Some(&artifact))
            .unwrap()
            .0;
        assert!(store.active_run(&start.run_id).unwrap().is_none());
        assert!(store.begin_workflow(&start).is_err());
        drop(store);
        let store = OperatorStore::open(dir.path(), &project).unwrap();
        assert_eq!(
            store.get_run_outcome(&start.run_id).unwrap().unwrap(),
            saved
        );
    }

    #[test]
    fn restart_publication_failure_keeps_durable_active_state_recoverable() {
        let dir = TempDir::new().unwrap();
        let project = project(dir.path());
        let mut store = OperatorStore::open(dir.path(), &project).unwrap();
        let start = begun(&project, "recoverable");
        store.begin_workflow(&start).unwrap();
        store.connection.execute_batch("CREATE TRIGGER reject_restart BEFORE INSERT ON artifacts BEGIN SELECT RAISE(ABORT, 'injected restart publication failure'); END;").unwrap();
        drop(store);
        assert!(OperatorStore::open(dir.path(), &project).is_err());
        let database = Connection::open(dir.path().join(".refact/buddy/operator.sqlite")).unwrap();
        let count: i64 = database
            .query_row("SELECT count(*) FROM active_runs", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
        let outcomes: i64 = database
            .query_row("SELECT count(*) FROM run_outcomes", [], |row| row.get(0))
            .unwrap();
        assert_eq!(outcomes, 0);
        database
            .execute_batch("DROP TRIGGER reject_restart;")
            .unwrap();
        drop(database);
        let store = OperatorStore::open(dir.path(), &project).unwrap();
        assert!(store.active_run(&start.run_id).unwrap().is_none());
        assert_eq!(
            store
                .get_run_outcome(&start.run_id)
                .unwrap()
                .unwrap()
                .value
                .status,
            refact_buddy_core::operator_types::RunStatus::Blocked
        );
    }

    #[test]
    fn active_runs_reject_foreign_identity_and_enforce_capacity() {
        use super::super::outcome_runtime::WorkflowOutcomeCheckpoint;
        let dir = TempDir::new().unwrap();
        let project = project(dir.path());
        let mut store = OperatorStore::open(dir.path(), &project).unwrap();
        let start = begun(&project, "owned");
        store.begin_workflow(&start).unwrap();
        let mut foreign = start.clone();
        foreign.project.project_id = "foreign".into();
        assert!(store.begin_workflow(&foreign).is_err());
        assert!(store
            .checkpoint_workflow(&foreign, WorkflowOutcomeCheckpoint::from_evidence([]))
            .is_err());
        for index in 1..MAX_ACTIVE_RUNS {
            store
                .begin_workflow(&begun(&project, &format!("run-{index}")))
                .unwrap();
        }
        assert!(store.begin_workflow(&begun(&project, "overflow")).is_err());
        assert!(store.get_run_outcome("overflow").unwrap().is_none());
    }

    #[test]
    fn unsupported_active_record_is_preserved_instead_of_rewritten_on_restart() {
        let dir = TempDir::new().unwrap();
        let project = project(dir.path());
        let mut store = OperatorStore::open(dir.path(), &project).unwrap();
        store.begin_workflow(&begun(&project, "future")).unwrap();
        store.connection.execute(
            "UPDATE active_runs SET payload=json_set(payload,'$.start.future_authority','unsupported') WHERE run_id='future'", [],
        ).unwrap();
        drop(store);
        assert!(OperatorStore::open(dir.path(), &project).is_err());
        let database = Connection::open(dir.path().join(".refact/buddy/operator.sqlite")).unwrap();
        let value: String = database.query_row(
            "SELECT json_extract(payload,'$.start.future_authority') FROM active_runs WHERE run_id='future'", [], |row| row.get(0),
        ).unwrap();
        assert_eq!(value, "unsupported");
        let terminal_count: i64 = database
            .query_row("SELECT count(*) FROM run_outcomes", [], |row| row.get(0))
            .unwrap();
        assert_eq!(terminal_count, 0);
    }

    #[test]
    fn closing_owner_releases_lock_even_when_a_spawn_inherited_the_descriptor() {
        let dir = TempDir::new().unwrap();
        let project = project(dir.path());
        let store = OperatorStore::open(dir.path(), &project).unwrap();
        let inherited = store._writer_lock.0.try_clone().unwrap();
        assert!(matches!(
            OperatorStore::open(dir.path(), &project),
            Err(OperatorStoreError::AlreadyOwned(_))
        ));
        drop(store);
        let reopened = OperatorStore::open(dir.path(), &project).unwrap();
        assert!(reopened
            .list_work_items(None, 10)
            .unwrap()
            .entries
            .is_empty());
        drop(inherited);
    }

    #[test]
    fn compound_outcome_work_artifact_is_atomic_and_idempotent() {
        let directory = TempDir::new().unwrap();
        let project = project(directory.path());
        let mut store = OperatorStore::open(directory.path(), &project).unwrap();
        let (outcome, item, artifact) = linked_records(&project);
        let (stored_outcome, stored_item, stored_artifact) = store
            .append_outcome_work_item_artifact(&outcome, &item, None, Some(&artifact))
            .unwrap();
        assert_eq!(stored_outcome.value, outcome);
        assert_eq!(stored_item.revision, 1);
        assert_eq!(stored_artifact.unwrap().value, artifact);
        let retry = store
            .append_outcome_work_item_artifact(&outcome, &item, None, Some(&artifact))
            .unwrap();
        assert_eq!(retry.0.change_seq, stored_outcome.change_seq);
        assert_eq!(retry.1, stored_item);
        assert_eq!(retry.2.unwrap().change_seq, 3);
    }
    #[test]
    fn compound_stale_or_invalid_link_rolls_back_everything() {
        let directory = TempDir::new().unwrap();
        let project = project(directory.path());
        let mut store = OperatorStore::open(directory.path(), &project).unwrap();
        let (outcome, item, artifact) = linked_records(&project);
        assert!(matches!(
            store.append_outcome_work_item_artifact(&outcome, &item, Some(1), Some(&artifact)),
            Err(OperatorStoreError::StaleRevision {
                current: 0
            })
        ));
        assert!(store.get_run_outcome(&outcome.run_id).unwrap().is_none());
        assert!(store.get_work_item(&item.id).unwrap().is_none());
        assert!(store
            .get_artifact_revision(&artifact.artifact_id, Some(1))
            .unwrap()
            .is_none());

        let mut invalid_item = item.clone();
        invalid_item.current_artifact_revision = Some(2);
        assert!(matches!(
            store.append_outcome_work_item_artifact(&outcome, &invalid_item, None, Some(&artifact)),
            Err(OperatorStoreError::Conflict(_))
        ));
        assert!(store.get_run_outcome(&outcome.run_id).unwrap().is_none());
        assert!(store.get_work_item(&item.id).unwrap().is_none());
    }

    #[test]
    fn version_one_and_two_stores_migrate_to_a_usable_current_schema() {
        for version in [1, 2, 3] {
            let d = TempDir::new().unwrap();
            let p = project(d.path());
            let mut store = OperatorStore::open(d.path(), &p).unwrap();
            let (outcome, item, artifact) = linked_records(&p);
            let (_, original, _) = store
                .append_outcome_work_item_artifact(&outcome, &item, None, Some(&artifact))
                .unwrap();
            downgrade_to_version(&store.connection, version);
            drop(store);

            let mut store = OperatorStore::open(d.path(), &p).unwrap();
            assert_eq!(user_version(&store.connection), SCHEMA_VERSION);
            assert_eq!(store.get_work_item(&item.id).unwrap().unwrap(), original);
            assert_eq!(store.get_read_state(&item.id).unwrap(), None);
            assert!(store.get_work_snooze(&item.id).unwrap().is_none());
            let read = store
                .mark_work_read(&item.id, &artifact.artifact_id, 1)
                .unwrap();
            let snooze = store
                .snooze_work(&item.id, &artifact.artifact_id, 1, Some(60))
                .unwrap();
            assert!(snooze.is_some());
            drop(store);

            let store = OperatorStore::open(d.path(), &p).unwrap();
            assert_eq!(user_version(&store.connection), SCHEMA_VERSION);
            assert_eq!(store.get_read_state(&item.id).unwrap(), Some(read));
            assert_eq!(store.get_work_snooze(&item.id).unwrap(), snooze);
            assert_eq!(store.get_work_item(&item.id).unwrap().unwrap(), original);
        }
    }

    #[test]
    fn migration_refuses_schema_that_contradicts_the_recorded_version() {
        refuses_and_preserves("PRAGMA user_version=1;", 1);
        refuses_and_preserves("DROP TABLE read_states; PRAGMA user_version=1;", 1);
        refuses_and_preserves("DROP TABLE read_states;", SCHEMA_VERSION);
        refuses_and_preserves("DROP TABLE work_snoozes;", SCHEMA_VERSION);
        refuses_and_preserves("DROP TABLE active_runs;", SCHEMA_VERSION);
        refuses_and_preserves("PRAGMA user_version=3;", 3);
        refuses_and_preserves("PRAGMA user_version=0;", 0);
    }

    fn user_version(connection: &Connection) -> i64 {
        connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap()
    }

    fn downgrade_to_version(connection: &Connection, version: i64) {
        if version < 6 {
            connection
                .execute_batch("DROP TABLE terminal_reservations;")
                .unwrap();
        }

        if version < 5 {
            connection
                .execute_batch("DROP TABLE operator_documents;")
                .unwrap();
        }
        if version < 4 {
            connection.execute_batch("DROP TABLE active_runs;").unwrap();
        }
        if version < 3 {
            connection
                .execute_batch("DROP TABLE work_snoozes;")
                .unwrap();
        }
        if version < 2 {
            connection.execute_batch("DROP TABLE read_states;").unwrap();
        }
        connection
            .execute_batch(&format!("PRAGMA user_version={version};"))
            .unwrap();
    }

    fn refuses_and_preserves(mutation: &str, expected_version: i64) {
        let d = TempDir::new().unwrap();
        let p = project(d.path());
        let mut store = OperatorStore::open(d.path(), &p).unwrap();
        let saved = store.upsert_work_item(&work("kept", &p), None).unwrap();
        store.connection.execute_batch(mutation).unwrap();
        drop(store);
        assert!(matches!(
            OperatorStore::open(d.path(), &p),
            Err(OperatorStoreError::CorruptSchema(_))
        ));
        let database = Connection::open(d.path().join(".refact/buddy/operator.sqlite")).unwrap();
        assert_eq!(user_version(&database), expected_version);
        let payload: String = database
            .query_row(
                "SELECT payload FROM work_items WHERE id='kept'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(decode::<WorkItem>(&payload).unwrap(), saved.value);
    }

    #[test]
    fn restart_marks_interrupted_effects_unknown_without_replaying_or_rewriting_terminal_receipts()
    {
        let d = TempDir::new().unwrap();
        let p = project(d.path());
        let mut store = OperatorStore::open(d.path(), &p).unwrap();
        let mut active = execution("active", &p);
        active.status = ActionExecutionState::Running;
        let mut done = execution("done", &p);
        done.status = ActionExecutionState::Completed;
        store.claim_execution(&active).unwrap();
        store.claim_execution(&done).unwrap();
        drop(store);
        let store = OperatorStore::open(d.path(), &p).unwrap();
        let unknown = store.get_execution("active").unwrap().unwrap();
        assert_eq!(unknown.value.status, ActionExecutionState::OutcomeUnknown);
        assert_eq!(unknown.value.effects, active.effects);
        assert_eq!(store.get_execution("done").unwrap().unwrap().value, done);
        drop(store);
        let store = OperatorStore::open(d.path(), &p).unwrap();
        assert_eq!(store.get_execution("active").unwrap().unwrap(), unknown);
    }

    #[test]
    fn search_filters_before_pagination_and_treats_query_as_literal() {
        let d = TempDir::new().unwrap();
        let p = project(d.path());
        let mut store = OperatorStore::open(d.path(), &p).unwrap();
        for (id, title, state) in [
            (
                "a",
                "Provider one",
                refact_buddy_core::operator_types::WorkItemState::NeedsAttention,
            ),
            (
                "b",
                "Provider two",
                refact_buddy_core::operator_types::WorkItemState::Delivered,
            ),
            (
                "c",
                "Provider three",
                refact_buddy_core::operator_types::WorkItemState::NeedsAttention,
            ),
        ] {
            let mut item = work(id, &p);
            item.title = title.into();
            item.state = state;
            store.upsert_work_item(&item, None).unwrap();
        }
        let first = store
            .search_work_items(None, 1, Some("needs_you"), Some("PROVIDER"))
            .unwrap();
        assert_eq!(first.entries[0].value.id, "c");
        let second = store
            .search_work_items(first.next_after, 1, Some("needs_you"), Some("provider"))
            .unwrap();
        assert_eq!(second.entries[0].value.id, "a");
        assert!(second.next_after.is_none());
        assert!(store
            .search_work_items(None, 10, None, Some("%' OR 1=1 --"))
            .unwrap()
            .entries
            .is_empty());
        assert!(store
            .search_work_items(None, 10, Some("unknown"), None)
            .is_err());
    }

    #[test]
    fn private_backup_contains_committed_wal_records_and_does_not_replace_previous_backup() {
        let d = TempDir::new().unwrap();
        let p = project(d.path());
        let mut store = OperatorStore::open(d.path(), &p).unwrap();
        let original = store.upsert_work_item(&work("saved", &p), None).unwrap();
        let backup = store.backup().unwrap();
        let original_bytes = fs::read(&backup).unwrap();
        let db = Connection::open_with_flags(&backup, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
        let payload: String = db
            .query_row(
                "SELECT payload FROM work_items WHERE id='saved'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(decode::<WorkItem>(&payload).unwrap(), original.value);
        store.upsert_work_item(&work("new", &p), None).unwrap();
        assert_ne!(store.backup().unwrap(), backup);
        assert_eq!(fs::read(&backup).unwrap(), original_bytes);
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(&backup).unwrap().permissions().mode() & 0o077,
            0
        );
    }
    #[test]
    fn newer_schema_and_corruption_preserve_bytes() {
        let d = TempDir::new().unwrap();
        let p = d.path().join(".refact/buddy");
        fs::create_dir_all(&p).unwrap();
        let db = p.join("operator.sqlite");
        let c = Connection::open(&db).unwrap();
        c.execute_batch("PRAGMA user_version=99").unwrap();
        drop(c);
        #[cfg(unix)]
        fs::set_permissions(&db, fs::Permissions::from_mode(0o600)).unwrap();
        let before = fs::read(&db).unwrap();
        assert!(matches!(
            OperatorStore::open(d.path(), &project(d.path())),
            Err(OperatorStoreError::SchemaTooNew(99))
        ));
        assert_eq!(before, fs::read(db).unwrap());

        let corrupted = d.path().join("corrupted");
        fs::create_dir_all(corrupted.join(".refact/buddy")).unwrap();
        let corrupt_db = corrupted.join(".refact/buddy/operator.sqlite");
        fs::write(&corrupt_db, b"not a sqlite database\0preserve me").unwrap();
        #[cfg(unix)]
        fs::set_permissions(&corrupt_db, fs::Permissions::from_mode(0o600)).unwrap();
        let corrupt_before = fs::read(&corrupt_db).unwrap();
        assert!(matches!(
            OperatorStore::open(&corrupted, &project(&corrupted)),
            Err(OperatorStoreError::Sql(_))
        ));
        assert_eq!(corrupt_before, fs::read(corrupt_db).unwrap());
    }

    #[test]
    fn malformed_lock_file_still_yields_already_owned() {
        let root = TempDir::new().unwrap();
        let identity = project(root.path());
        let first = OperatorStore::open(root.path(), &identity).unwrap();
        let lock_path = root.path().join(".refact/buddy/operator.lock");
        #[cfg(not(windows))]
        fs::write(&lock_path, b"not-json").unwrap();
        #[cfg(windows)]
        assert!(fs::write(&lock_path, b"not-json").is_err());
        let Err(error) = OperatorStore::open(root.path(), &identity) else {
            panic!("expected contended lock");
        };
        assert!(matches!(error, OperatorStoreError::AlreadyOwned(None)));
        drop(first);
    }
    #[test]
    fn recovery_reservation_is_atomic_charged_and_unknown_is_not_replayed() {
        let d = TempDir::new().unwrap();
        let p = project(d.path());
        let mut store = OperatorStore::open(d.path(), &p).unwrap();
        let mut original = execution("original", &p);
        original.status = ActionExecutionState::Completed;
        original.recovery = Some(refact_buddy_core::operator_types::RecoveryRef {
            recovery_id: original.execution_id.clone(),
            action_id: original.action_id.clone(),
            expected_post_version: "2".into(),
            available: true,
        });
        store
            .claim_execution_inner(&original, false, Some(None))
            .unwrap();
        store
            .set_document_growth_policy(Some(
                super::super::operator_observability::DocumentGrowthPolicy {
                    max_rows_per_namespace: 1,
                    replace_editorial_aggregate: false,
                },
            ))
            .unwrap();
        store
            .set_capacity_policy(Some(super::super::operator_observability::CapacityPolicy {
                max_rows_per_table: 1,
                max_database_bytes: 1,
            }))
            .unwrap();
        let mut recovery = execution("restore", &p);
        recovery.status = ActionExecutionState::Claimed;
        recovery.action_id = "recovery:original:original".into();
        assert!(matches!(
            store
                .claim_resource_recovery_execution(&recovery, "original")
                .unwrap(),
            ExecutionClaim::Claimed(_)
        ));
        let unknown = store
            .update_execution_state("restore", 1, ActionExecutionState::OutcomeUnknown)
            .unwrap();
        assert!(
            matches!(store.claim_resource_recovery_execution(&recovery, "original").unwrap(), ExecutionClaim::Existing(saved) if saved == unknown)
        );
        let summary = store
            .diagnostic_summary()
            .unwrap()
            .document_growth
            .terminal_reservations;
        assert_eq!(summary.retained_entries, 2);
        assert_eq!(summary.outstanding_slots, 2);
        assert!(store
            .reserve_terminal_documents(
                "resource_apply",
                "new",
                &[("resource_receipt", "new".into(), 1)]
            )
            .is_err());
        // A conflicting execution primary key must roll back the newly reserved slot.
        let mut collision = recovery.clone();
        collision.idempotency_key = "collision".into();
        store
            .connection
            .execute(
                "DELETE FROM terminal_reservations WHERE producer_id='restore'",
                [],
            )
            .unwrap();
        assert!(store
            .claim_resource_recovery_execution(&collision, "original")
            .is_err());
        let count: usize = store
            .connection
            .query_row(
                "SELECT count(*) FROM terminal_reservations WHERE producer_id='restore'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn legacy_resource_cutoff_survives_active_completion_but_not_same_spec_new_claims() {
        let d = TempDir::new().unwrap();
        let p = project(d.path());
        let mut store = OperatorStore::open(d.path(), &p).unwrap();
        let mut old = execution("legacy", &p);
        old.status = ActionExecutionState::Running;
        store.claim_execution(&old).unwrap();
        downgrade_to_version(&store.connection, 5);
        drop(store);
        let mut store = OperatorStore::open(d.path(), &p).unwrap();
        assert!(store.is_legacy_resource_execution("legacy").unwrap());
        let restored = store.get_execution("legacy").unwrap().unwrap();
        assert_eq!(restored.revision, 2);
        old.status = ActionExecutionState::Completed;
        assert!(matches!(
            store.replace_execution(&old, 1),
            Err(OperatorStoreError::StaleRevision {
                current: 2
            })
        ));
        store.replace_execution(&old, restored.revision).unwrap();
        assert!(store.is_legacy_resource_execution("legacy").unwrap());
        let mut native = old.clone();
        native.execution_id = "native".into();
        native.idempotency_key = "native-key".into();
        store.claim_execution(&native).unwrap();
        assert!(!store.is_legacy_resource_execution("native").unwrap());
        assert!(!store.is_legacy_resource_execution("missing").unwrap());
        assert!(store
            .reserve_grandfathered_terminal_documents(
                "resource_apply",
                "native",
                &[("resource_receipt", "native".into(), 1)]
            )
            .is_err());
    }

    #[test]
    fn execution_key_and_cas_preserve_immutable_identity() {
        let d = TempDir::new().unwrap();
        let p = project(d.path());
        let mut store = OperatorStore::open(d.path(), &p).unwrap();
        let original = execution("one", &p);
        assert!(matches!(
            store.claim_execution(&original).unwrap(),
            ExecutionClaim::Claimed(_)
        ));
        let mut changed = original.clone();
        changed.execution_id = "different-execution".into();
        changed.action_id = "different-action".into();
        assert!(matches!(
            store.claim_execution(&changed),
            Err(OperatorStoreError::Conflict(_))
        ));
        let updated = store
            .update_execution_state(&original.execution_id, 1, ActionExecutionState::Completed)
            .unwrap();
        assert_eq!(updated.value.action_id, original.action_id);
        assert_eq!(updated.value.requester, original.requester);
        assert!(matches!(
            store.update_execution_state(&original.execution_id, 1, ActionExecutionState::Failed),
            Err(OperatorStoreError::StaleRevision {
                current: 2
            })
        ));
    }

    #[test]
    fn foreign_project_cannot_write_or_advance_sequence() {
        let d = TempDir::new().unwrap();
        let p = project(d.path());
        let mut store = OperatorStore::open(d.path(), &p).unwrap();
        let before = store.upsert_work_item(&work("owned", &p), None).unwrap();
        let foreign = ProjectIdentity {
            project_id: "other".into(),
            canonical_root: p.canonical_root.clone(),
        };
        assert!(matches!(
            store.upsert_work_item(&work("foreign", &foreign), None),
            Err(OperatorStoreError::ProjectMismatch)
        ));
        assert!(store.get_work_item("foreign").unwrap().is_none());
        let after = store.upsert_work_item(&work("next", &p), None).unwrap();
        assert_eq!(after.change_seq, before.change_seq + 1);
    }
    #[cfg(unix)]
    #[test]
    fn creates_private_database_and_lock_files() {
        let d = TempDir::new().unwrap();
        let p = project(d.path());
        let store = OperatorStore::open(d.path(), &p).unwrap();
        for file in [
            store.path().to_path_buf(),
            store.path().with_file_name("operator.lock"),
        ] {
            assert_eq!(fs::metadata(file).unwrap().permissions().mode() & 0o077, 0);
        }
    }
    #[cfg(unix)]
    #[test]
    fn rejects_symlink_and_permissive_database_without_touching_target() {
        let d = TempDir::new().unwrap();
        let outside = TempDir::new().unwrap();
        let refact = d.path().join(".refact");
        symlink(outside.path(), &refact).unwrap();
        assert!(matches!(
            OperatorStore::open(d.path(), &project(d.path())),
            Err(OperatorStoreError::InvalidInput(_))
        ));

        fs::remove_file(&refact).unwrap_or(());
        fs::remove_dir(&refact).unwrap_or(());
        let buddy = d.path().join(".refact/buddy");
        fs::create_dir_all(&buddy).unwrap();
        let db = buddy.join("operator.sqlite");
        fs::write(&db, b"private bytes").unwrap();
        fs::set_permissions(&db, fs::Permissions::from_mode(0o644)).unwrap();
        let before = fs::read(&db).unwrap();
        assert!(matches!(
            OperatorStore::open(d.path(), &project(d.path())),
            Err(OperatorStoreError::InvalidInput(_))
        ));
        assert_eq!(before, fs::read(db).unwrap());
    }
}
