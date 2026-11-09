use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::sync::atomic::{AtomicBool, Ordering};

use refact_scheduler_core::automation::{
    legacy_job_digest, AutomationJournal, ProjectAutomationLease, RunClaim, RunStatus,
};
use serde_json::Value;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::files_correction::get_active_project_path;
use crate::global_context::SharedGlobalContext;

use super::types::Job;

pub(crate) struct ExecutionOwner {
    pub capacity: Arc<Semaphore>,
    durable: Option<(AutomationJournal, Arc<ProjectAutomationLease>)>,
    active_jobs: Mutex<HashMap<String, ()>>,
    pub delivery_lock: tokio::sync::Mutex<()>,
}

pub(crate) struct Admission {
    owner: Arc<ExecutionOwner>,
    job_id: String,
    pub claim: Option<RunClaim>,
    _permit: OwnedSemaphorePermit,
    started: AtomicBool,
    completed: AtomicBool,
}

impl Drop for Admission {
    fn drop(&mut self) {
        if self.started.load(Ordering::Relaxed) && !self.completed.load(Ordering::Relaxed) {
            if let (Some((journal, _)), Some(claim)) = (&self.owner.durable, &self.claim) {
                let _ = journal.finish_run(
                    claim,
                    RunStatus::Unknown,
                    None,
                    Some("execution interrupted after effect start"),
                );
            }
        }
        if let Ok(mut active) = self.owner.active_jobs.lock() {
            active.remove(&self.job_id);
        }
    }
}

impl ExecutionOwner {
    pub async fn retry_deliveries(&self, gcx: &SharedGlobalContext) {
        let Ok(_guard) = self.delivery_lock.try_lock() else {
            return;
        };
        let Some((journal, _lease)) = &self.durable else {
            return;
        };
        let now = chrono::Utc::now().timestamp_millis().max(0) as u64;
        let pending =
            match journal.pending_deliveries_for_targets(&["legacy_cron".to_string()], 256, now) {
                Ok(pending) => pending,
                Err(error) => {
                    tracing::warn!("scheduler delivery recovery unavailable: {error}");
                    return;
                }
            };
        let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
        for delivery in pending.into_iter().filter(|d| d.target == "legacy_cron") {
            let result = async {
                let id = delivery.data["job_id"]
                    .as_str()
                    .ok_or("missing delivery job")?;
                let jobs = journal.load_legacy_jobs()?;
                let snapshot: Job = serde_json::from_value(delivery.data["job"].clone())
                    .map_err(|e| e.to_string())?;
                let mut job = jobs.into_iter().find(|j| j.id == id).unwrap_or(snapshot);
                if delivery.data["config_digest"].as_str()
                    != Some(legacy_job_digest(&job)?.as_str())
                {
                    return Err("delivery configuration changed; needs review".to_string());
                }
                job.last_status = delivery.data["status"].as_str().map(str::to_string);
                job.last_fired_at_ms = delivery.data["fired_at_ms"].as_u64();
                super::delivery::deliver(
                    &app,
                    &job,
                    delivery.data["output"].as_str().unwrap_or_default(),
                )
                .await
            }
            .await;
            match result {
                Ok(()) => {
                    let _ = journal.acknowledge_delivery(delivery.event_seq, &delivery.target);
                }
                Err(error) => {
                    let _ = journal.retry_delivery(
                        delivery.event_seq,
                        &delivery.target,
                        now.saturating_add(60_000),
                        &error,
                    );
                }
            }
        }
    }

    /// A successful agent run records queue acceptance, not LLM completion.
    /// A retry of that exact occurrence is accepted without sending it again.
    pub fn agent_enqueue_accepted(&self, job: &Job, occurrence: &str) -> Result<bool, String> {
        if job.command_spec().is_some() {
            return Ok(false);
        }
        match &self.durable {
            Some((journal, lease)) => journal.is_run_succeeded(
                &job.id,
                occurrence,
                &legacy_job_digest(job)?,
                lease.epoch(),
            ),
            None => Ok(false),
        }
    }

    pub fn admit(self: &Arc<Self>, job: &Job, occurrence: &str) -> Result<Admission, String> {
        let permit = self
            .capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| "scheduler at capacity".to_string())?;
        let mut active = self
            .active_jobs
            .lock()
            .map_err(|_| "scheduler admission poisoned")?;
        if active.contains_key(&job.id) {
            return Err("scheduled task already executing".into());
        }
        let claim = if let Some((journal, lease)) = &self.durable {
            if !job.durable {
                journal.put_session_job(job, lease.epoch())?;
            }
            let digest = legacy_job_digest(job)?;
            journal
                .claim_run_explicit(
                    &job.id,
                    occurrence,
                    &digest,
                    lease.epoch(),
                    false,
                    job.trigger_at_ms.is_some(),
                )?
                .ok_or_else(|| {
                    "scheduled occurrence already claimed or held for review".to_string()
                })?
                .into()
        } else {
            None
        };
        active.insert(job.id.clone(), ());
        Ok(Admission {
            owner: self.clone(),
            job_id: job.id.clone(),
            claim,
            _permit: permit,
            started: AtomicBool::new(false),
            completed: AtomicBool::new(false),
        })
    }
}

impl Admission {
    pub fn started(&self) -> Result<(), String> {
        if let (Some((journal, _)), Some(claim)) = (&self.owner.durable, &self.claim) {
            if !journal.mark_effect_started(claim)? {
                return Err("scheduled execution authorization changed before start".into());
            }
        }
        self.started.store(true, Ordering::Relaxed);
        Ok(())
    }

    /// The chat queue explicitly refused this agent turn before inserting it.
    /// Never use this for command failures, cancellation, or an ambiguous error.
    pub fn rejected_agent_enqueue(&self) -> Result<(), String> {
        if let (Some((journal, _)), Some(claim)) = (&self.owner.durable, &self.claim) {
            if !journal.release_rejected_agent_enqueue(claim)? {
                return Err("scheduled execution owner changed before queue rejection".into());
            }
        }
        self.completed.store(true, Ordering::Relaxed);
        Ok(())
    }

    pub fn finish(&self, success: bool, result: Option<&Value>) -> Result<(), String> {
        self.finish_status(
            if success {
                RunStatus::Succeeded
            } else {
                RunStatus::Failed
            },
            result,
        )
    }

    pub fn finish_status(&self, status: RunStatus, result: Option<&Value>) -> Result<(), String> {
        if let (Some((journal, _)), Some(claim)) = (&self.owner.durable, &self.claim) {
            if !journal.finish_run(claim, status, result, None)? {
                return Err("scheduled execution owner changed before completion".into());
            }
        }
        self.completed.store(true, Ordering::Relaxed);
        Ok(())
    }

    pub fn delivered(&self, sequence: Option<i64>) -> Result<(), String> {
        if let (Some((journal, _)), Some(sequence)) = (&self.owner.durable, sequence) {
            journal.acknowledge_delivery(sequence, "legacy_cron")?;
        }
        Ok(())
    }

    pub fn complete_with_delivery(
        &self,
        status: RunStatus,
        payload: &Value,
    ) -> Result<Option<i64>, String> {
        match (&self.owner.durable, &self.claim) {
            (Some((journal, _)), Some(claim)) => {
                if !journal.complete_run_and_enqueue_delivery(
                    claim,
                    status,
                    Some(payload),
                    None,
                    Some((payload, "legacy_cron")),
                )? {
                    return Err("scheduled execution owner changed before completion".into());
                }
                self.completed.store(true, Ordering::Relaxed);
                let sequence = journal.run_delivery_id(claim)?;
                if sequence.is_none() {
                    return Err("committed delivery awaits recovery".into());
                }
                Ok(sequence)
            }
            _ => {
                self.completed.store(true, Ordering::Relaxed);
                Ok(None)
            }
        }
    }
}

pub(crate) async fn execution_owner(
    gcx: &SharedGlobalContext,
) -> Result<Arc<ExecutionOwner>, String> {
    static OWNERS: OnceLock<Mutex<HashMap<PathBuf, Arc<ExecutionOwner>>>> = OnceLock::new();
    let root = get_active_project_path(gcx.clone()).await;
    let key = match &root {
        Some(root) => std::fs::canonicalize(root)
            .map_err(|e| format!("scheduler project unavailable: {e}"))?,
        None => PathBuf::from(format!("session:{:p}", Arc::as_ptr(gcx))),
    };
    let mut owners = OWNERS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| "scheduler owner registry poisoned")?;
    if let Some(owner) = owners.get(&key) {
        return Ok(owner.clone());
    }
    let durable = if root.is_some() {
        let lease = ProjectAutomationLease::acquire(&key)?;
        let journal = AutomationJournal::open(&key)?;
        journal.activate_owner(&lease)?;
        journal.recover_inflight(lease.epoch())?;
        Some((journal, lease))
    } else {
        None
    };
    let capacity = durable
        .as_ref()
        .map(|(_, lease)| {
            lease.capacity("execution", gcx.scheduler_config.max_concurrent_runs.max(1))
        })
        .unwrap_or_else(|| {
            Arc::new(Semaphore::new(
                gcx.scheduler_config.max_concurrent_runs.max(1),
            ))
        });
    let owner = Arc::new(ExecutionOwner {
        capacity,
        durable,
        active_jobs: Mutex::new(HashMap::new()),
        delivery_lock: tokio::sync::Mutex::new(()),
    });
    owners.insert(key, owner.clone());
    Ok(owner)
}
