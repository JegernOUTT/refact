use std::sync::OnceLock;

use chrono::Utc;
use refact_buddy_core::operator_types::{
    EvidenceRef, NativeSourceIdentity, NativeSourceKind, ProjectIdentity,
};
use refact_chat_api::{ChatCommand, GoalBudget, GoalCriterion, GoalSnapshot, GoalStatus};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::agents::spawn::{
    spawn_background_agent, NotifyParent, SpawnGoal, SpawnRequest, SpawnWorktreeMode,
};
use crate::app_state::AppState;
use super::operator_store::{OperatorStore, OperatorStoreError, Stored};
use super::outcome_runtime::{OutcomeRuntime, WorkflowOutcomeStart};
use super::supervision_authority::{self as authority, MandateRecord};

pub const LINKS: &str = "supervision_links";
static DISPATCH: OnceLock<Mutex<()>> = OnceLock::new();
/// Bounded per-mandate control history (message/pause/resume/cancel). Every
/// entry is an ordinary charged write; cancel is never blocked by this bound.
pub const MAX_CONTROLS: usize = 32;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupervisionLink {
    pub schema_version: u16,
    pub project: ProjectIdentity,
    pub mandate_id: String,
    pub owner_epoch: u64,
    pub chat_id: String,
    pub goal_version: Option<u32>,
    #[serde(default)]
    pub child_goal_version: Option<u32>,
    #[serde(default)]
    pub question_delivery: Option<QuestionDelivery>,
    #[serde(default)]
    pub question_history: Vec<QuestionDelivery>,
    pub intake_content: String,
    pub intake_criteria: Vec<GoalCriterion>,
    pub worker_agent_id: Option<String>,
    pub child_chat_id: Option<String>,
    pub dispatch_attempted: bool,
    pub command_acknowledged: bool,
    pub revoked: bool,
    pub escalation: Option<String>,
    pub escalation_count: u32,
    pub next_attention_at_ms: i64,
    pub delivery_run_id: Option<String>,
    #[serde(default)]
    pub delivery_proof: Option<GoalSnapshot>,
    #[serde(default)]
    pub engineering_proof: Option<super::supervision_proof::EngineeringProof>,
    #[serde(default)]
    pub control_history: Vec<ControlRecord>,
    /// Owner intent only. Whether the worker actually stopped dispatching is
    /// reported live by `SupervisionStatus::native_pause`.
    #[serde(default)]
    pub pause_requested: bool,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ControlKind {
    Message,
    Pause,
    Resume,
    Cancel,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ControlRecord {
    pub control_id: String,
    pub kind: ControlKind,
    pub note_id: Option<String>,
    pub requested_by: NativeSourceIdentity,
    pub at_ms: i64,
    pub outcome: String,
}

/// Typed control bodies. There is deliberately no free-text field: a message
/// can only reference a note the user pre-wrote into the mandate.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlRequest {
    pub control_id: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageRequest {
    pub control_id: String,
    pub note_id: String,
}

/// Explicit user request to assess an *existing* supervised engineering candidate with one
/// code workflow. Only an idempotency key and a workflow id: no chat, worktree, model, tool,
/// budget, or authority field is accepted from the body.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateAssessmentRequest {
    pub control_id: String,
    pub workflow_id: String,
}

/// Durable result of an admitted candidate assessment: the retained report identity and, when
/// the report named the bound candidate, the prepared (user-confirmation-required) action.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CandidateAssessment {
    pub schema_version: u16,
    pub project: ProjectIdentity,
    pub worker: refact_buddy_core::operator_types::WorkerIdentity,
    pub requested_by: NativeSourceIdentity,
    pub mandate_id: String,
    pub control_id: String,
    pub workflow_id: String,
    pub run_id: String,
    pub worktree_id: String,
    pub candidate_chat_id: String,
    pub candidate_version: String,
    /// `admitted` (claimed before publication), `reported` (retained report exists), or
    /// `outcome_unknown` (publication outcome unknown; never replayed).
    pub state: String,
    pub work_id: Option<String>,
    pub artifact_id: Option<String>,
    pub artifact_revision: Option<u32>,
    pub action_spec_hash: Option<String>,
    pub error: Option<String>,
}

pub const CANDIDATE_ASSESSMENTS: &str = "supervision_candidate_assessments";

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalIntake {
    pub content: String,
    pub criteria: Vec<GoalCriterion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuestionDelivery {
    pub tool_call_id: String,
    pub asked_at_ms: u64,
    pub answers: Vec<refact_chat_api::QuestionAnswer>,
    pub state: String,
    pub resolution_id: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SupervisionStatus {
    pub link: SupervisionLink,
    pub native_goal: Option<GoalSnapshot>,
    pub native_worker: Option<crate::agents::types::BackgroundAgent>,
    pub native_worker_goal: Option<GoalSnapshot>,
    /// Live dispatch barrier state of the worker in this process; `None`
    /// means the worker is not running here (unknown/offline), never paused.
    pub native_pause: Option<crate::agents::registry::PauseStatus>,
    pub allowed_operations: Vec<String>,
    pub steering_notes: Vec<authority::SteeringNote>,
    pub coverage: String,
    pub host_wake_supported: bool,
    pub daemon_wake_supported: bool,
}

fn dispatch_lock() -> &'static Mutex<()> {
    DISPATCH.get_or_init(|| Mutex::new(()))
}

pub fn native_budget(record: &MandateRecord) -> Result<GoalBudget, String> {
    let budget = record
        .mandate
        .budget_limit
        .as_ref()
        .ok_or("finite budget required")?;
    if !matches!(budget.max_turns, Some(1..=100))
        || !matches!(budget.max_tokens, Some(1..=1_000_000))
        || !matches!(budget.max_duration_seconds, Some(60..=86_400))
        || budget.max_duration_seconds.is_some_and(|v| v % 60 != 0)
        || budget
            .max_cost_micros
            .is_some_and(|v| v == 0 || v % 10_000 != 0)
    {
        return Err("budget cannot be represented exactly by the native owner".into());
    }
    Ok(GoalBudget {
        max_turns: budget.max_turns,
        max_tokens: budget.max_tokens,
        max_minutes: budget.max_duration_seconds.map(|v| (v / 60) as u32),
        max_cost_cents: budget.max_cost_micros.map(|v| v / 10_000),
        explicit: true,
        ..GoalBudget::default()
    })
}

async fn authorized(
    runtime: &OutcomeRuntime,
    id: &str,
    operation: &str,
) -> Result<MandateRecord, String> {
    let record = authority::load(runtime, id.to_string()).await?.value;
    authority::validate_record(&record, runtime)?;
    if !record
        .mandate
        .allowed_operations
        .iter()
        .any(|v| v == operation)
    {
        return Err("operation not granted by mandate".into());
    }
    Ok(record)
}

pub async fn load_link(
    runtime: &OutcomeRuntime,
    id: String,
) -> Result<Stored<SupervisionLink>, String> {
    let mandate = authority::load(runtime, id.clone()).await?;
    let stored = runtime
        .with_store(move |store| store.get_document::<SupervisionLink>(LINKS, &id))
        .await
        .map_err(|e| e.to_string())?
        .ok_or("supervision link not found")?;
    if stored.value.schema_version != 1
        || stored.value.project != runtime.project()
        || stored.value.mandate_id != mandate.value.mandate.mandate_id
    {
        return Err("supervision source binding mismatch".into());
    }
    Ok(stored)
}

/// Read a completion that was durably proved by the original worker. This is
/// intentionally narrower than `load_link`: it never makes the old mandate
/// live in a new runtime, and is only usable for the one-way retained report
/// publication path.
async fn load_retained_completion(
    runtime: &OutcomeRuntime,
    id: String,
) -> Result<(Stored<SupervisionLink>, Stored<MandateRecord>), String> {
    let mandate = authority::load_historical(runtime, id.clone()).await?;
    let stored = runtime
        .with_store(move |store| store.get_document::<SupervisionLink>(LINKS, &id))
        .await
        .map_err(|e| e.to_string())?
        .ok_or("supervision link not found")?;
    let link = &stored.value;
    if link.schema_version != 1
        || link.project != runtime.project()
        || link.mandate_id != mandate.value.mandate.mandate_id
        || !(link.owner_epoch == mandate.value.mandate.owner_epoch
            || (mandate.value.mandate.revoked_at.is_some()
                && link.owner_epoch.checked_add(1) == Some(mandate.value.mandate.owner_epoch)))
        || !link.dispatch_attempted
        || link.worker_agent_id.is_none()
    {
        return Err("retained supervision source binding mismatch".into());
    }
    let goal = link
        .delivery_proof
        .as_ref()
        .ok_or("retained delivery proof unavailable")?;
    if !verified_goal(link, goal) {
        return Err("retained delivery proof is stale".into());
    }
    match (
        &link.engineering_proof,
        authority::is_engineering(&mandate.value),
    ) {
        (Some(proof), true) => super::supervision_proof::validate_retained(
            proof,
            &mandate.value,
            link.child_chat_id
                .as_deref()
                .ok_or("retained proof chat unavailable")?,
            goal,
        )?,
        (None, false) => {}
        _ => return Err("retained engineering proof binding mismatch".into()),
    }
    Ok((stored, mandate))
}

async fn save_link(
    runtime: &OutcomeRuntime,
    link: SupervisionLink,
    revision: Option<u64>,
) -> Result<Stored<SupervisionLink>, String> {
    runtime
        .with_store(move |store| persist_link(store, link, revision))
        .await
        .map_err(|e| e.to_string())
}

// Only bounded, one-way owner transitions consume terminal credit. Polling,
// retries, escalation text, and dispatch admission must pay ordinary capacity.
fn question_terminal(delivery: &QuestionDelivery) -> bool {
    matches!(
        delivery.state.as_str(),
        "resolved" | "resolved_differently" | "cancelled"
    )
}

fn reserved_transition(old: &SupervisionLink, new: &SupervisionLink) -> bool {
    (!old.revoked && new.revoked)
        || (old.worker_agent_id.is_none() && new.worker_agent_id.is_some())
        || (old.delivery_proof.is_none() && new.delivery_proof.is_some())
        || (old.delivery_run_id.is_none() && new.delivery_run_id.is_some())
        || new
            .question_history
            .iter()
            .chain(new.question_delivery.iter())
            .any(|next| {
                let previous = old
                    .question_history
                    .iter()
                    .chain(old.question_delivery.iter())
                    .find(|previous| {
                        previous.tool_call_id == next.tool_call_id
                            && previous.asked_at_ms == next.asked_at_ms
                    });
                match previous {
                    None => next.state == "dispatch_outcome_unknown",
                    Some(previous) => {
                        !question_terminal(previous)
                            && (question_terminal(next)
                                || (previous.state == "dispatch_outcome_unknown"
                                    && next.state == "enqueued"))
                    }
                }
            })
}

fn persist_link(
    store: &mut OperatorStore,
    link: SupervisionLink,
    revision: Option<u64>,
) -> Result<Stored<SupervisionLink>, OperatorStoreError> {
    let id = &link.mandate_id;
    let old = store.get_document::<SupervisionLink>(LINKS, id)?;
    if old.as_ref().map(|s| s.revision) != revision {
        return Err(OperatorStoreError::Conflict("supervision revision changed"));
    }
    if let Some(old) = &old {
        if old.value.project != link.project
            || old.value.owner_epoch != link.owner_epoch
            || old.value.chat_id != link.chat_id
            || old.value.intake_content != link.intake_content
            || old.value.intake_criteria != link.intake_criteria
            || (old.value.delivery_proof.is_some()
                && old.value.delivery_proof != link.delivery_proof)
            || (old.value.engineering_proof.is_some()
                && old.value.engineering_proof != link.engineering_proof)
            || (old.value.delivery_proof.is_some()
                && old.value.engineering_proof != link.engineering_proof)
            || (old.value.child_chat_id.is_some() && old.value.child_chat_id != link.child_chat_id)
            || (old.value.worker_agent_id.is_some()
                && old.value.worker_agent_id != link.worker_agent_id)
            || (old.value.dispatch_attempted
                && (!link.dispatch_attempted
                    || old.value.goal_version != link.goal_version
                    || old.value.child_goal_version != link.child_goal_version))
        {
            return Err(OperatorStoreError::Conflict(
                "supervision native binding is immutable",
            ));
        }
        if serde_json::to_value(&old.value)? == serde_json::to_value(&link)? {
            return Ok(old.clone());
        }
    }
    authority::ensure_terminal_reservation(store, id)?;
    // Admission precedes even parent-goal effects. Existing ambiguous dispatches
    // cannot acquire new completion authority on retry or restart.
    if old.is_none() {
        store.reserve_supervision_completion(id)?;
    }
    if old
        .as_ref()
        .is_some_and(|old| reserved_transition(&old.value, &link))
    {
        store.put_reserved_document("supervision", id, LINKS, id, &link, revision)
    } else {
        store.put_native_ordinary_document("supervision", id, LINKS, id, &link, revision)
    }
}

pub(crate) async fn intake(
    app: &AppState,
    runtime: &OutcomeRuntime,
    id: String,
    input: GoalIntake,
) -> Result<SupervisionLink, String> {
    let _dispatch = dispatch_lock().lock().await;
    let record = authorized(runtime, &id, "create_goal").await?;
    if input.content.trim().is_empty()
        || input.content.len() > 16_000
        || input.criteria.is_empty()
        || input.criteria.len() > 12
        || input
            .criteria
            .iter()
            .any(|c| c.id.trim().is_empty() || c.text.trim().is_empty() || c.text.len() > 2_000)
        || input
            .criteria
            .iter()
            .enumerate()
            .any(|(i, c)| input.criteria[..i].iter().any(|other| other.id == c.id))
    {
        return Err("bounded content and unique explicit criteria required".into());
    }
    let chat_id = uuid::Uuid::new_v4().to_string();
    let link = SupervisionLink {
        schema_version: 1,
        project: runtime.project(),
        mandate_id: id.clone(),
        owner_epoch: record.mandate.owner_epoch,
        chat_id: chat_id.clone(),
        goal_version: None,
        child_goal_version: None,
        question_delivery: None,
        question_history: vec![],
        intake_content: input.content.clone(),
        intake_criteria: input.criteria.clone(),
        worker_agent_id: None,
        child_chat_id: None,
        dispatch_attempted: false,
        command_acknowledged: false,
        revoked: false,
        escalation: None,
        escalation_count: 0,
        next_attention_at_ms: 0,
        delivery_run_id: None,
        delivery_proof: None,
        engineering_proof: None,
        control_history: vec![],
        pause_requested: false,
    };
    let mut saved = save_link(runtime, link, None).await?;
    authority::bind_chat(&id, chat_id.clone())?;
    app.chat
        .facade
        .create_session(refact_runtime_api::CreateSessionRequest {
            chat_id: chat_id.clone(),
            thread: refact_chat_api::ThreadParams {
                id: chat_id.clone(),
                title: "Supervised objective".into(),
                model: record.mandate.model_constraints[0].clone(),
                mode: "ask".into(),
                tool_use: "none".into(),
                ..Default::default()
            },
            messages: vec![],
        })
        .await?;
    authority::validate_record(&record, runtime)?;
    app.chat
        .facade
        .push_supervised_command(
            &chat_id,
            ChatCommand::SetGoal {
                plan: None,
                content: input.content,
                criteria: Some(input.criteria),
                budget: Some(native_budget(&record)?),
            },
            authority::guard(&record),
        )
        .await?;
    app.chat
        .facade
        .push_supervised_command(
            &chat_id,
            ChatCommand::GoalControl {
                action: "pause".into(),
            },
            authority::guard(&record),
        )
        .await?;
    saved.value.command_acknowledged = true;
    Ok(save_link(runtime, saved.value, Some(saved.revision))
        .await?
        .value)
}

fn matches_intake(link: &SupervisionLink, goal: &GoalSnapshot) -> bool {
    goal.content == link.intake_content
        && goal.criteria == link.intake_criteria
        && link.goal_version.is_none_or(|v| v == goal.version)
}

pub(crate) async fn spawn_worker(
    app: &AppState,
    runtime: &OutcomeRuntime,
    id: String,
) -> Result<SupervisionLink, String> {
    let _dispatch = dispatch_lock().lock().await;
    let record = authorized(runtime, &id, "spawn_worker").await?;
    let mut saved = load_link(runtime, id.clone()).await?;
    if saved.value.delivery_proof.is_some() && saved.value.delivery_run_id.is_none() {
        saved.value.delivery_run_id = Some(publish_delivery(runtime, &saved.value).await?);
        return Ok(save_link(runtime, saved.value, Some(saved.revision))
            .await?
            .value);
    }
    if saved.value.revoked
        || saved.value.dispatch_attempted
        || saved.value.owner_epoch != record.mandate.owner_epoch
    {
        return Err(
            "dispatch consumed, revoked, or stale; uncertain attempts are never replayed".into(),
        );
    }
    let snapshot = app
        .chat
        .facade
        .session_snapshot(&saved.value.chat_id)
        .await?;
    let goal = snapshot.goal.ok_or("native goal not resolved yet")?;
    if !matches_intake(&saved.value, &goal)
        || goal.status != GoalStatus::Paused
        || !goal.active
        || snapshot.thread.model != record.mandate.model_constraints[0]
        || goal.budget != native_budget(&record)?
    {
        authority::invalidate(&id)?;
        return Err("native goal/model/budget changed or user took over".into());
    }
    let project_dirs = crate::files_correction::get_project_dirs(app.gcx.clone()).await;
    if project_dirs.len() != 1
        || std::fs::canonicalize(&project_dirs[0])
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            != record.project.canonical_root
    {
        return Err("multi-root native worker spawn is unsupported".into());
    }
    let links = runtime
        .with_store(|store| {
            store.list_documents::<SupervisionLink>(LINKS, None, authority::MAX_MANDATES)
        })
        .await
        .map_err(|e| e.to_string())?;
    if links.entries.iter().any(|other| {
        other.value.mandate_id != id
            && other.value.dispatch_attempted
            && other.value.delivery_run_id.is_none()
            && !other.value.revoked
    }) {
        return Err("project resource claim held by another supervised operation".into());
    }
    let completion_id = id.clone();
    runtime
        .with_store(move |store| store.require_supervision_completion(&completion_id))
        .await
        .map_err(|error| error.to_string())?;
    saved.value.dispatch_attempted = true;
    saved.value.goal_version = Some(goal.version);
    // Native SpawnGoal installs version 1 in a fresh child; never infer it from
    // the parent's independently versioned goal or from a later worker claim.
    saved.value.child_goal_version = Some(1);
    saved = save_link(runtime, saved.value, Some(saved.revision)).await?;
    authority::validate_record(&record, runtime)?;
    let spawn_goal = SpawnGoal {
        content: goal.content,
        criteria: goal.criteria,
        budget: Some(native_budget(&record)?),
    };
    let request = SpawnRequest {
        completion_push: Default::default(),
        native_supervision_guard: Some(authority::NativeSpawnGuard::mint(
            &record,
            saved.value.chat_id.clone(),
            spawn_goal.clone(),
        )?),
        kind: crate::agents::types::BgAgentKind::Subagent,
        parent_chat_id: saved.value.chat_id.clone(),
        parent_root_chat_id: Some(saved.value.chat_id.clone()),
        parent_tool_call_id: None,
        config_name: "subagent".into(),
        title: if authority::is_engineering(&record) {
            "Supervised engineering candidate"
        } else {
            "Supervised read-only analysis"
        }
        .into(),
        prompt: spawn_goal.content.clone(),
        // A supervised worker is launched from the mandate alone: no operator
        // prose, no inherited history, no reusable profile, and no deliverable
        // contract beyond the goal criteria the mandate already carries.
        initial_context_messages: Vec::new(),
        history_source_model: None,
        custom_instructions: None,
        deliverable: None,
        profile: None,
        tools: Some(authority::worker_tools(&record)?),
        target_files: authority::worker_target_files(&record)?,
        max_steps: native_budget(&record)?.max_turns.unwrap_or(1) as usize,
        model: record.mandate.model_constraints[0].clone(),
        model_type: None,
        context_tokens_cap: None,
        goal: Some(spawn_goal),
        plan: None,
        worktree_mode: SpawnWorktreeMode::Isolated {
            auto_merge: false,
        },
        parent_subchat_tx: None,
        parent_worktree: snapshot.thread.worktree,
        parent_task_meta: snapshot.thread.task_meta,
        subchat_depth: 0,
        notify_parent: NotifyParent::Silent,
    };
    let handle = spawn_background_agent(app.clone(), request).await?;
    saved.value.worker_agent_id = Some(handle.agent_id.clone());
    saved.value.child_chat_id = Some(handle.child_chat_id);
    if authority::validate_record(&record, runtime).is_err() {
        let _ = app
            .agents
            .cancel(
                &saved.value.chat_id,
                &handle.agent_id,
                Some("mandate invalidated during spawn".into()),
            )
            .await;
        saved.value.revoked = true;
    }
    Ok(save_link(runtime, saved.value, Some(saved.revision))
        .await?
        .value)
}

fn verified_goal(link: &SupervisionLink, goal: &GoalSnapshot) -> bool {
    goal.content == link.intake_content
        && goal.criteria == link.intake_criteria
        && link.child_goal_version == Some(goal.version)
        && goal.status == GoalStatus::Completed
        && goal.attempts.last().is_some_and(|attempt| {
            attempt.verdict.eq_ignore_ascii_case("met")
                && !attempt.verifier_reply.trim().is_empty()
                && attempt.gaps.is_empty()
                && !goal.criteria.is_empty()
                && attempt.criteria_verdicts.len() == goal.criteria.len()
                && goal.criteria.iter().all(|criterion| {
                    attempt
                        .criteria_verdicts
                        .iter()
                        .filter(|v| v.id == criterion.id && v.met)
                        .count()
                        == 1
                })
        })
}

pub(crate) async fn status(
    app: &AppState,
    runtime: &OutcomeRuntime,
    id: String,
) -> Result<SupervisionStatus, String> {
    let (record, link, historical) = match authority::load(runtime, id.clone()).await {
        Ok(record) => (record.value, load_link(runtime, id).await?.value, false),
        // A retained proof may be inspected after the worker epoch is gone,
        // but it exposes neither operations nor steering notes and cannot
        // restore the process-local grant.
        Err(_) => {
            let (link, record) = load_retained_completion(runtime, id).await?;
            (record.value, link.value, true)
        }
    };
    let snapshot = app.chat.facade.session_snapshot(&link.chat_id).await.ok();
    let worker = match link.worker_agent_id.as_deref() {
        Some(agent) => app.agents.get(&link.chat_id, agent).await.ok(),
        None => None,
    };
    let native_pause = match (link.worker_agent_id.as_deref(), worker.as_ref()) {
        (Some(agent), Some(w)) if !w.status.is_terminal() => app.agents.pause_status(agent).await,
        _ => None,
    };
    let native_worker_goal = match link.child_chat_id.as_deref() {
        Some(child)
            if worker
                .as_ref()
                .is_some_and(|w| w.child_chat_id.as_deref() == Some(child)) =>
        {
            app.chat
                .facade
                .session_snapshot(child)
                .await
                .ok()
                .and_then(|s| s.goal)
        }
        _ => None,
    };
    Ok(SupervisionStatus {
        link,
        native_goal: snapshot.and_then(|s| s.goal),
        native_worker: worker,
        native_worker_goal,
        native_pause,
        allowed_operations: (!historical)
            .then_some(record.mandate.allowed_operations.clone())
            .unwrap_or_default(),
        steering_notes: (!historical)
            .then_some(record.steering_notes.clone())
            .unwrap_or_default(),
        coverage: if historical {
            "Retained historical verification is offline after the original worker epoch ended. It exposes no live authority, automatic control, grant, restart, tool reapproval, or edit scope.".into()
        } else {
            "Native owner observations only. Missing state is unknown/offline, never completed. An explicit read_project grant permits the bounded native read catalog (cat, tree, search_pattern, glob); it is not a claim that every source file was read. No host or daemon wake, automatic restart, tool reapproval, or automatic merge.".into()
        },
        host_wake_supported: false,
        daemon_wake_supported: false,
    })
}

pub(crate) async fn stop(
    app: &AppState,
    runtime: &OutcomeRuntime,
    id: String,
) -> Result<SupervisionLink, String> {
    stop_with_control(app, runtime, id, None).await
}

async fn stop_with_control(
    app: &AppState,
    runtime: &OutcomeRuntime,
    id: String,
    control: Option<ControlRecord>,
) -> Result<SupervisionLink, String> {
    authority::load(runtime, id.clone()).await?;
    if let Some(control) = &control {
        let current = load_link(runtime, id.clone()).await?;
        if duplicate_control(
            &current.value,
            &control.control_id,
            control.kind,
            control.note_id.as_deref(),
            &control.requested_by,
        )? {
            return Ok(current.value);
        }
    }
    authority::invalidate(&id)?;
    let _dispatch = dispatch_lock().lock().await;
    let mut saved = load_link(runtime, id.clone()).await?;
    let control_id = control.as_ref().map(|value| value.control_id.clone());
    let was_revoked = saved.value.revoked;
    // Cancel is the ultimate stop: a saturated control history never blocks
    // it, the entry is simply not recorded.
    if let Some(control) = control {
        if duplicate_control(
            &saved.value,
            &control.control_id,
            control.kind,
            control.note_id.as_deref(),
            &control.requested_by,
        )? {
            return Ok(saved.value);
        }
        if saved.value.control_history.len() < MAX_CONTROLS {
            saved.value.control_history.push(control);
        }
    }
    let revoke_result = authority::revoke(runtime, id).await;
    saved.value.revoked = true;
    let mut failures = Vec::new();
    if let Some(agent) = saved.value.worker_agent_id.as_deref() {
        if let Err(error) = app
            .agents
            .cancel_subtree(
                &saved.value.chat_id,
                agent,
                true,
                Some("explicit user stop or takeover".into()),
            )
            .await
        {
            failures.push(error);
        }
    }
    if let Err(error) = app
        .chat
        .facade
        .push_priority_command(&saved.value.chat_id, ChatCommand::Abort {})
        .await
    {
        failures.push(error);
    }
    if let Err(error) = app
        .chat
        .facade
        .push_priority_command(
            &saved.value.chat_id,
            ChatCommand::GoalControl {
                action: "stop".into(),
            },
        )
        .await
    {
        failures.push(error);
    }
    if let Err(error) = revoke_result {
        failures.push(error);
    }
    if !failures.is_empty() {
        if let Some(control) = saved.value.control_history.iter_mut().find(|entry| {
            Some(&entry.control_id) == control_id.as_ref()
                && entry.kind == ControlKind::Cancel
                && entry.outcome == "revoked_and_cancelled"
        }) {
            control.outcome = "revoked_cancellation_unconfirmed".into();
        }
    }
    if !was_revoked && !failures.is_empty() {
        saved.value.escalation = Some(
            "Authority invalidated; native cancellation or durable revocation unconfirmed".into(),
        );
    }
    let result = save_link(runtime, saved.value, Some(saved.revision))
        .await?
        .value;
    Ok(result)
}

fn validate_control_id(control_id: &str) -> Result<(), String> {
    if control_id.trim() != control_id || control_id.is_empty() || control_id.len() > 64 {
        return Err("control_id must be a trimmed 1..=64 character identity".into());
    }
    Ok(())
}

/// Same control identity is a no-op; the same identity with a different
/// kind, note, or requester is an error, never a silently different effect.
fn duplicate_control(
    link: &SupervisionLink,
    control_id: &str,
    kind: ControlKind,
    note_id: Option<&str>,
    requester: &NativeSourceIdentity,
) -> Result<bool, String> {
    match link
        .control_history
        .iter()
        .find(|existing| existing.control_id == control_id)
    {
        None => Ok(false),
        Some(existing)
            if existing.kind == kind
                && existing.note_id.as_deref() == note_id
                && existing.requested_by == *requester =>
        {
            Ok(true)
        }
        Some(_) => Err("control identity reused with a different kind, note, or requester".into()),
    }
}

/// Prepare a control: authorize, bind the requester to the mandate owner,
/// de-duplicate, and enforce the bounded history. Returns the live link and
/// the record to append, or `None` when the control was already applied.
async fn admit_control(
    runtime: &OutcomeRuntime,
    id: &str,
    operation: &str,
    requester: &NativeSourceIdentity,
    control_id: &str,
    kind: ControlKind,
    note_id: Option<&str>,
) -> Result<
    (
        MandateRecord,
        Stored<SupervisionLink>,
        Option<ControlRecord>,
    ),
    String,
> {
    validate_control_id(control_id)?;
    let record = authorized(runtime, id, operation).await?;
    if record.mandate.owner != *requester {
        return Err("controls are accepted only from the mandate owner".into());
    }
    let saved = load_link(runtime, id.to_string()).await?;
    if duplicate_control(&saved.value, control_id, kind, note_id, requester)? {
        return Ok((record, saved, None));
    }
    if saved.value.control_history.len() >= MAX_CONTROLS {
        return Err("control capacity reached".into());
    }
    if saved.value.revoked || saved.value.owner_epoch != record.mandate.owner_epoch {
        return Err("supervision revoked or stale; explicit user grant required".into());
    }
    Ok((
        record,
        saved,
        Some(ControlRecord {
            control_id: control_id.to_string(),
            kind,
            note_id: note_id.map(str::to_string),
            requested_by: requester.clone(),
            at_ms: Utc::now().timestamp_millis(),
            outcome: String::new(),
        }),
    ))
}

/// The live native worker of a link: registered, non-terminal, bound to the
/// link's child chat, and still on the mandated model.
async fn live_worker(
    app: &AppState,
    record: &MandateRecord,
    link: &SupervisionLink,
) -> Result<(String, String), String> {
    let agent_id = link
        .worker_agent_id
        .clone()
        .ok_or("native worker not available")?;
    let child = link
        .child_chat_id
        .clone()
        .ok_or("native worker not available")?;
    let agent = app.agents.get(&link.chat_id, &agent_id).await?;
    if agent.child_chat_id.as_deref() != Some(child.as_str()) || agent.status.is_terminal() {
        return Err("native worker finished or ownership changed".into());
    }
    if !app.agents.has_runtime(&agent_id).await {
        return Err("native worker is not running in this process (restarted or offline)".into());
    }
    let snapshot = app.chat.facade.session_snapshot(&child).await?;
    if snapshot.thread.model != record.mandate.model_constraints[0] {
        return Err("native worker model differs from the mandate".into());
    }
    Ok((agent_id, child))
}

pub(crate) async fn message_worker(
    app: &AppState,
    runtime: &OutcomeRuntime,
    id: String,
    requester: NativeSourceIdentity,
    request: MessageRequest,
) -> Result<SupervisionLink, String> {
    let _dispatch = dispatch_lock().lock().await;
    let (record, mut saved, control) = admit_control(
        runtime,
        &id,
        "steer_worker",
        &requester,
        &request.control_id,
        ControlKind::Message,
        Some(&request.note_id),
    )
    .await?;
    let Some(mut control) = control else {
        return Ok(saved.value);
    };
    let (agent_id, child) = live_worker(app, &record, &saved.value).await?;
    // Exact mandate-listed note only; the capability re-authorizes at consume.
    let capability = authority::NativeSteeringCapability::mint(&record, child, &request.note_id)?;
    control.outcome = "dispatch_outcome_unknown".into();
    saved.value.control_history.push(control);
    // Persist intent before enqueue so a retry is a no-op, never a duplicate.
    saved = save_link(runtime, saved.value, Some(saved.revision)).await?;
    authority::validate_record(&record, runtime)?;
    let queued = app
        .agents
        .push_native_note(
            &agent_id,
            capability.into_native_note(request.control_id.clone()),
        )
        .await;
    if let Err(error) = queued {
        if let Some(entry) = saved
            .value
            .control_history
            .iter_mut()
            .find(|entry| entry.control_id == request.control_id)
        {
            entry.outcome = format!("enqueue_failed: {error}");
        }
        return Ok(save_link(runtime, saved.value, Some(saved.revision))
            .await?
            .value);
    }
    Ok(saved.value)
}

pub(crate) async fn pause_worker(
    app: &AppState,
    runtime: &OutcomeRuntime,
    id: String,
    requester: NativeSourceIdentity,
    request: ControlRequest,
) -> Result<SupervisionLink, String> {
    let _dispatch = dispatch_lock().lock().await;
    let (record, mut saved, control) = admit_control(
        runtime,
        &id,
        "steer_worker",
        &requester,
        &request.control_id,
        ControlKind::Pause,
        None,
    )
    .await?;
    let Some(mut control) = control else {
        return Ok(saved.value);
    };
    let (agent_id, _) = live_worker(app, &record, &saved.value).await?;
    let gate = app
        .agents
        .pause_gate(&agent_id)
        .await
        .ok_or("native worker runtime unavailable")?;
    control.outcome = "dispatch_outcome_unknown".into();
    saved.value.control_history.push(control);
    saved.value.pause_requested = true;
    saved = save_link(runtime, saved.value, Some(saved.revision)).await?;
    authority::validate_record(&record, runtime)?;
    // Only the native dispatch barrier is touched. The parent goal is already
    // paused since intake and no GoalControl is issued: a goal nudge pause
    // would not stop the worker loop and must not be reported as a pause.
    authority::with_command_guard(
        Some(&authority::guard(&record)),
        &saved.value.chat_id,
        || {
            gate.request();
            Ok(())
        },
    )?;
    Ok(saved.value)
}

pub(crate) async fn resume_worker(
    app: &AppState,
    runtime: &OutcomeRuntime,
    id: String,
    requester: NativeSourceIdentity,
    request: ControlRequest,
) -> Result<SupervisionLink, String> {
    let _dispatch = dispatch_lock().lock().await;
    // Fails closed: revoked, expired, restarted, or unbound grants cannot
    // resume, and a stopped goal is never implicitly restarted.
    let (record, mut saved, control) = admit_control(
        runtime,
        &id,
        "steer_worker",
        &requester,
        &request.control_id,
        ControlKind::Resume,
        None,
    )
    .await?;
    let Some(mut control) = control else {
        return Ok(saved.value);
    };
    let (agent_id, child) = live_worker(app, &record, &saved.value).await?;
    let child_goal = app
        .chat
        .facade
        .session_snapshot(&child)
        .await?
        .goal
        .ok_or("native worker goal unavailable")?;
    if child_goal.status != GoalStatus::Active
        || !child_goal.active
        || Some(child_goal.version) != saved.value.child_goal_version
    {
        return Err(
            "native worker goal is not active; resume never restarts a stopped goal".into(),
        );
    }
    let gate = app
        .agents
        .pause_gate(&agent_id)
        .await
        .ok_or("native worker runtime unavailable")?;
    if !gate.is_requested() {
        return Err("native worker has no pause request to resume".into());
    }
    control.outcome = "dispatch_outcome_unknown".into();
    saved.value.control_history.push(control);
    saved.value.pause_requested = false;
    saved = save_link(runtime, saved.value, Some(saved.revision)).await?;
    authority::validate_record(&record, runtime)?;
    authority::with_command_guard(
        Some(&authority::guard(&record)),
        &saved.value.chat_id,
        || {
            gate.release();
            Ok(())
        },
    )?;
    Ok(saved.value)
}

pub(crate) async fn cancel_worker(
    app: &AppState,
    runtime: &OutcomeRuntime,
    id: String,
    requester: NativeSourceIdentity,
    request: ControlRequest,
) -> Result<SupervisionLink, String> {
    validate_control_id(&request.control_id)?;
    let record = authority::load(runtime, id.clone()).await?.value;
    if record.mandate.owner != requester {
        return Err("controls are accepted only from the mandate owner".into());
    }
    stop_with_control(
        app,
        runtime,
        id,
        Some(ControlRecord {
            control_id: request.control_id,
            kind: ControlKind::Cancel,
            note_id: None,
            requested_by: requester,
            at_ms: Utc::now().timestamp_millis(),
            outcome: "revoked_and_cancelled".into(),
        }),
    )
    .await
}

pub(crate) async fn reconcile(
    app: &AppState,
    runtime: &OutcomeRuntime,
    id: String,
) -> Result<SupervisionLink, String> {
    let _dispatch = dispatch_lock().lock().await;
    // A persisted proof is evidence, not authority. Publish it before reading
    // live status: after a restart the original worker epoch is intentionally
    // unavailable, but the strictly bound historical record can still spend
    // its already-reserved one-way completion slot.
    if let Ok((mut retained, _mandate)) = load_retained_completion(runtime, id.clone()).await {
        if retained.value.delivery_run_id.is_none() {
            retained.value.delivery_run_id =
                Some(publish_delivery(runtime, &retained.value).await?);
            retained = save_link(runtime, retained.value, Some(retained.revision)).await?;
        }
        return Ok(retained.value);
    }
    let mut saved = load_link(runtime, id.clone()).await?;
    let observed = status(app, runtime, id.clone()).await?;
    let record = authority::load(runtime, id.clone()).await?.value;
    if let Some(delivery) = saved.value.question_delivery.as_mut() {
        if let Some(child) = saved.value.child_chat_id.as_ref() {
            if let Some(session) = app.chat.sessions.read().await.get(child).cloned() {
                let session = session.lock().await;
                observe_question(delivery, &session.pending_questions);
            }
        }
    }
    if let Some(child) = saved.value.child_chat_id.as_ref() {
        if let Some(session) = app.chat.sessions.read().await.get(child).cloned() {
            let session = session.lock().await;
            for delivery in &mut saved.value.question_history {
                observe_question(delivery, &session.pending_questions);
            }
        }
    }
    let parent_changed = saved.value.dispatch_attempted
        && observed.native_goal.as_ref().is_some_and(|g| {
            !matches_intake(&saved.value, g)
                || g.status != GoalStatus::Paused
                || g.budget != native_budget(&record).unwrap_or_default()
        });
    let mut model_changed = false;
    for chat in std::iter::once(&saved.value.chat_id).chain(saved.value.child_chat_id.iter()) {
        if let Ok(snapshot) = app.chat.facade.session_snapshot(chat).await {
            model_changed |=
                record.mandate.model_constraints.first() != Some(&snapshot.thread.model);
        }
    }
    let child_changed = observed.native_worker_goal.as_ref().is_some_and(|goal| {
        goal.content != saved.value.intake_content
            || goal.criteria != saved.value.intake_criteria
            || Some(goal.version) != saved.value.child_goal_version
            || goal.budget != native_budget(&record).unwrap_or_default()
    });
    if saved.value.revoked
        || parent_changed
        || child_changed
        || model_changed
        || authority::validate_record(&record, runtime).is_err()
    {
        authority::invalidate(&id)?;
        saved.value.revoked = true;
        if let Some(agent) = saved.value.worker_agent_id.as_deref() {
            if let Err(error) = app
                .agents
                .cancel_subtree(
                    &saved.value.chat_id,
                    agent,
                    true,
                    Some("supervision expired, revoked, or taken over".into()),
                )
                .await
            {
                coalesce(
                    &mut saved.value,
                    &format!("Authority invalidated; child cancellation unconfirmed: {error}"),
                    Utc::now().timestamp_millis(),
                );
                return Ok(save_link(runtime, saved.value, Some(saved.revision))
                    .await?
                    .value);
            }
        }
        coalesce(
            &mut saved.value,
            "Authority unavailable; explicit user grant required; no automatic replay",
            Utc::now().timestamp_millis(),
        );
    } else if observed
        .native_goal
        .as_ref()
        .is_some_and(|g| matches_intake(&saved.value, g) && g.status == GoalStatus::Paused)
        && observed.native_worker.as_ref().is_some_and(|w| {
            w.status.is_terminal() && w.model == record.mandate.model_constraints[0]
        })
        && observed.native_worker_goal.as_ref().is_some_and(|g| {
            verified_goal(&saved.value, g) && g.budget == native_budget(&record).unwrap_or_default()
        })
    {
        if saved.value.delivery_run_id.is_none() {
            if saved.value.delivery_proof.is_none() {
                if authority::is_engineering(&record) {
                    let child = saved
                        .value
                        .child_chat_id
                        .as_deref()
                        .ok_or("native child missing")?;
                    let Some(proof) = super::supervision_proof::get_authorized_proof(child) else {
                        coalesce(&mut saved.value, "Native engineering source proof unavailable; worker claims are not verification", Utc::now().timestamp_millis());
                        return Ok(save_link(runtime, saved.value, Some(saved.revision))
                            .await?
                            .value);
                    };
                    super::supervision_proof::validate_retained(
                        &proof,
                        &record,
                        child,
                        observed
                            .native_worker_goal
                            .as_ref()
                            .ok_or("native goal missing")?,
                    )?;
                    let comparison = super::supervision_proof::compare_current_source(
                        app.paths.cache_dir.clone(),
                        &proof,
                    )
                    .await?;
                    if !comparison.unchanged {
                        coalesce(&mut saved.value, "Native engineering proof is stale: isolated source changed after verification", Utc::now().timestamp_millis());
                        return Ok(save_link(runtime, saved.value, Some(saved.revision))
                            .await?
                            .value);
                    }
                    authority::validate_record(&record, runtime)?;
                    saved.value.engineering_proof = Some(proof);
                }
                saved.value.delivery_proof = observed.native_worker_goal.clone();
                saved = save_link(runtime, saved.value, Some(saved.revision)).await?;
                if saved.value.engineering_proof.is_some() {
                    if let Some(child) = saved.value.child_chat_id.as_deref() {
                        super::supervision_proof::acknowledge_authorized(child);
                    }
                }
            }
            saved.value.delivery_run_id = Some(publish_delivery(runtime, &saved.value).await?);
        }
    } else {
        coalesce(
            &mut saved.value,
            "Native completion proof unavailable; worker claims do not complete the objective",
            Utc::now().timestamp_millis(),
        );
    }
    Ok(save_link(runtime, saved.value, Some(saved.revision))
        .await?
        .value)
}

async fn publish_delivery(
    runtime: &OutcomeRuntime,
    link: &SupervisionLink,
) -> Result<String, String> {
    runtime.publish_supervision_report(link.clone()).await
}

pub(crate) fn delivery_report(
    link: &SupervisionLink,
    worker: refact_buddy_core::operator_types::WorkerIdentity,
) -> Result<(WorkflowOutcomeStart, EvidenceRef, String), String> {
    let goal = link
        .delivery_proof
        .as_ref()
        .ok_or("delivery proof unavailable")?;
    if !verified_goal(link, goal) {
        return Err("retained proof is stale".into());
    }
    let proof_chat_id = link
        .child_chat_id
        .as_ref()
        .ok_or("native proof chat unavailable")?;
    let attempt = goal.attempts.last().ok_or("native verifier unavailable")?;
    let proof_at = chrono::DateTime::from_timestamp_millis(
        i64::try_from(attempt.at_ms).map_err(|_| "invalid verifier timestamp")?,
    )
    .ok_or("native verifier timestamp unavailable")?
    .to_rfc3339();
    let run_id = format!("supervision:{}:{}", link.mandate_id, goal.version);
    let evidence = EvidenceRef {
        evidence_id: format!("{run_id}:verifier"),
        source: NativeSourceIdentity {
            source_kind: NativeSourceKind::Goal,
            source_id: proof_chat_id.clone(),
            source_version: goal.version.to_string(),
        },
        locator: format!("chat://{}/goal/{}", proof_chat_id, goal.version),
        observed_at: proof_at.clone(),
        digest: Some(format!(
            "sha256:{}",
            sha256(&serde_json::to_vec(goal).map_err(|e| e.to_string())?)
        )),
    };
    let coverage = match &link.engineering_proof {
        Some(proof) => format!(
            "native worker goal {} revision {} with isolated worktree {} HEAD {}, patch {} and content {} observed {} through {}; retained historical verification, not a claim of current source, executed tests, merge, installation or deployment",
            proof_chat_id, goal.version, proof.source.worktree_id, proof.source.current_head,
            proof.source.patch_digest, proof.source.content_digest, proof.observed_before_at, proof.observed_after_at,
        ),
        None => format!("native worker goal {} revision {} and explicit criteria; parent remains paused; bounded read-only analysis only", proof_chat_id, goal.version),
    };
    let report = match &link.engineering_proof {
        Some(_) => format!("{}\n\nCoverage: {}. Native criteria verdicts and unchanged isolated source were captured together; this does not independently certify tests not evidenced by the native verdict.", attempt.verifier_reply, coverage),
        None => format!("{}\n\nCoverage: bounded read-only analysis only. This is not verification of project files, tests, or an applied engineering change.", attempt.verifier_reply),
    };
    let start = WorkflowOutcomeStart {
        run_id: run_id.clone(),
        work_id: String::new(),
        parent_run_id: None,
        project: link.project.clone(),
        worker,
        origin: "native_goal_verifier".into(),
        workflow: format!("supervision:{}", link.mandate_id),
        input_fingerprint: format!(
            "{}:{}:{}:{}",
            link.chat_id,
            link.goal_version.ok_or("parent goal version unavailable")?,
            proof_chat_id,
            goal.version
        ),
        coverage,
        started_at: proof_at,
    };
    Ok((start, evidence, report))
}

async fn load_candidate_assessment(
    runtime: &OutcomeRuntime,
    id: &str,
) -> Result<Option<Stored<CandidateAssessment>>, String> {
    let id = id.to_string();
    runtime
        .with_store(move |store| {
            store.get_document::<CandidateAssessment>(CANDIDATE_ASSESSMENTS, &id)
        })
        .await
        .map_err(|e| e.to_string())
}

async fn save_candidate_assessment(
    runtime: &OutcomeRuntime,
    value: CandidateAssessment,
    revision: Option<u64>,
) -> Result<Stored<CandidateAssessment>, String> {
    let id = format!("{}:{}", value.mandate_id, value.control_id);
    runtime
        .with_store(move |store| store.put_document(CANDIDATE_ASSESSMENTS, &id, &value, revision))
        .await
        .map_err(|e| e.to_string())
}

/// The finished engineering candidate of a mandate: retained verified proof, the worker agent
/// registered *and* terminal (an unknown or missing agent fails closed), and the isolated
/// worktree still registered to the worker chat under this project. Returns the worktree id,
/// the candidate chat id, and the candidate's current patch version. Nothing is created.
async fn finished_engineering_candidate(
    app: &AppState,
    record: &MandateRecord,
    link: &SupervisionLink,
) -> Result<(String, String, String), String> {
    let proof = link
        .engineering_proof
        .as_ref()
        .ok_or("mandate has no retained engineering proof")?;
    let goal = link
        .delivery_proof
        .as_ref()
        .ok_or("retained delivery proof unavailable")?;
    if !verified_goal(link, goal) {
        return Err("retained delivery proof is stale".into());
    }
    let child = link
        .child_chat_id
        .clone()
        .ok_or("native worker chat unavailable")?;
    super::supervision_proof::validate_retained(proof, record, &child, goal)?;
    let agent_id = link
        .worker_agent_id
        .clone()
        .ok_or("native worker not available")?;
    let agent = app
        .agents
        .get(&link.chat_id, &agent_id)
        .await
        .map_err(|e| format!("native worker state unknown; candidate not assessed: {e}"))?;
    if agent.child_chat_id.as_deref() != Some(child.as_str()) {
        return Err("native worker ownership changed".into());
    }
    if !agent.status.is_terminal() || app.agents.has_runtime(&agent_id).await {
        return Err("native worker is still busy; candidate not assessed".into());
    }
    let service = crate::worktrees::service::WorktreeService::new_async(
        app.paths.cache_dir.clone(),
        record.project.canonical_root.clone().into(),
    )
    .await
    .map_err(|e| e.to_string())?;
    let view = service
        .get_worktree(&proof.source.worktree_id)
        .await
        .map_err(|e| e.to_string())?;
    if view.meta.id != proof.source.worktree_id
        || view.meta.root.to_string_lossy() != proof.source.worktree_root
        || view.meta.source_workspace_root.to_string_lossy() != proof.source.source_workspace_root
        || view.meta.source_workspace_root != *service.source_workspace_root()
        || !view
            .references
            .iter()
            .any(|reference| reference.chat_id.as_deref() == Some(child.as_str()))
    {
        return Err(
            "isolated candidate is no longer registered to the supervised worker chat".into(),
        );
    }
    let version = super::resources::project_code::registered_candidate_version(
        &service,
        &proof.source.worktree_id,
    )
    .await
    .map_err(|e| format!("{e:?}"))?;
    Ok((proof.source.worktree_id.clone(), child, version))
}

pub(crate) async fn validate_candidate_assessment(
    app: &AppState,
    runtime: &OutcomeRuntime,
    binding: &super::workflow_deliverables::NativeCandidateBinding,
) -> Result<String, String> {
    use refact_buddy_core::autonomous_workflows::{workflow_action_family, WorkflowActionFamily};

    let assessment_id = format!("{}:{}", binding.mandate_id, binding.control_id);
    let expected_run_id = format!(
        "buddy-{}-{}",
        binding.workflow_id,
        sha256(assessment_id.as_bytes())
            .chars()
            .take(32)
            .collect::<String>()
    );
    if binding.schema_version != 1
        || binding.project != runtime.project()
        || binding.worker != runtime.worker()
        || !crate::buddy::actor::validate_workflow_id(&binding.workflow_id)
        || !matches!(
            workflow_action_family(&binding.workflow_id),
            WorkflowActionFamily::IsolatedCandidateCheckpoint
        )
        || validate_control_id(&binding.control_id).is_err()
        || binding.run_id != expected_run_id
    {
        return Err(
            "candidate assessment binding belongs to another project, worker, or workflow".into(),
        );
    }
    let stored =
        super::workflow_deliverables::NativeCandidateBinding::load(runtime, &binding.run_id)
            .await?
            .ok_or("candidate assessment binding is unavailable")?;
    if stored != *binding {
        return Err("candidate assessment binding changed after admission".into());
    }
    let record = authorized(runtime, &binding.mandate_id, "engineer_project").await?;
    if record.mandate.owner_epoch != binding.owner_epoch
        || record.mandate.owner != binding.requested_by
        || !authority::is_engineering(&record)
    {
        return Err("candidate assessment mandate no longer matches its admission".into());
    }
    let (link, retained_record) =
        load_retained_completion(runtime, binding.mandate_id.clone()).await?;
    if retained_record.value.project != record.project
        || retained_record.value.worker != record.worker
        || retained_record.value.mandate.mandate_id != record.mandate.mandate_id
        || retained_record.value.mandate.owner_epoch != record.mandate.owner_epoch
        || retained_record.value.mandate.owner != record.mandate.owner
        || link.value.revoked
        || link.value.owner_epoch != binding.owner_epoch
        || link.value.mandate_id != binding.mandate_id
    {
        return Err("retained candidate authority no longer matches its admission".into());
    }
    let proof = link
        .value
        .engineering_proof
        .as_ref()
        .ok_or("mandate has no retained engineering proof")?;
    let comparison =
        super::supervision_proof::compare_current_source(app.paths.cache_dir.clone(), proof)
            .await?;
    if !comparison.unchanged {
        return Err("isolated candidate source changed since retained proof".into());
    }
    let (worktree_id, candidate_chat_id, candidate_version) =
        finished_engineering_candidate(app, &record, &link.value).await?;
    if worktree_id != binding.worktree_id
        || candidate_chat_id != binding.candidate_chat_id
        || candidate_version != binding.candidate_version
    {
        return Err("finished engineering candidate changed after admission".into());
    }
    let goal = link
        .value
        .delivery_proof
        .as_ref()
        .ok_or("retained delivery proof unavailable")?;
    Ok(super::jobs::autonomous_chats::redact_and_cap_text(
        &format!(
            "Retained native evidence only; no fresh scan. Worker goal {} version {} verifier: {}. Isolated worktree {} HEAD {} patch {} content {}.",
            binding.candidate_chat_id,
            goal.version,
            goal.attempts.last().map(|attempt| attempt.verifier_reply.as_str()).unwrap_or("unavailable"),
            proof.source.worktree_id,
            proof.source.current_head,
            proof.source.patch_digest,
            proof.source.content_digest,
        ),
        8_000,
    ))
}

pub(crate) async fn assess_candidate(
    app: &AppState,
    runtime: &OutcomeRuntime,
    id: String,
    requester: NativeSourceIdentity,
    request: CandidateAssessmentRequest,
) -> Result<CandidateAssessment, String> {
    use refact_buddy_core::autonomous_workflows::{workflow_action_family, WorkflowActionFamily};
    use super::jobs::autonomous_chats::AutonomousBuddyChatSpec;
    use super::workflow_deliverables::NativeCandidateBinding;

    validate_control_id(&request.control_id)?;
    if !crate::buddy::actor::validate_workflow_id(&request.workflow_id)
        || !matches!(
            workflow_action_family(&request.workflow_id),
            WorkflowActionFamily::IsolatedCandidateCheckpoint
        )
    {
        return Err("workflow does not assess isolated code candidates".into());
    }
    let _dispatch = dispatch_lock().lock().await;
    let record = authorized(runtime, &id, "engineer_project").await?;
    if record.mandate.owner != requester {
        return Err("candidate assessment is accepted only from the mandate owner".into());
    }
    let saved = load_link(runtime, id.clone()).await?;
    let link = &saved.value;
    if link.revoked || link.owner_epoch != record.mandate.owner_epoch || !link.dispatch_attempted {
        return Err(
            "supervision revoked, stale, or never dispatched; explicit user grant required".into(),
        );
    }
    let assessment_id = format!("{id}:{}", request.control_id);
    let run_id = format!(
        "buddy-{}-{}",
        request.workflow_id,
        sha256(assessment_id.as_bytes())
            .chars()
            .take(32)
            .collect::<String>()
    );
    if let Some(existing) = load_candidate_assessment(runtime, &assessment_id).await? {
        if existing.value.schema_version != 1
            || existing.value.project != runtime.project()
            || existing.value.worker != runtime.worker()
            || existing.value.requested_by != requester
            || existing.value.mandate_id != id
            || existing.value.control_id != request.control_id
            || existing.value.run_id != run_id
            || existing.value.workflow_id != request.workflow_id
        {
            return Err("control identity reused with a different workflow".into());
        }
        // Claimed before generation: whatever happened, it is never replayed.
        let mut value = existing.value;
        if value.state == "admitted" {
            value.state = "outcome_unknown".into();
        }
        return Ok(value);
    }
    if let Some(binding) = NativeCandidateBinding::load(runtime, &run_id).await? {
        if binding.mandate_id != id
            || binding.control_id != request.control_id
            || binding.workflow_id != request.workflow_id
            || binding.requested_by != requester
        {
            return Err("candidate assessment binding conflicts with this request".into());
        }
        return Err(
            "candidate assessment admission is pending or outcome unknown; it is never replayed"
                .into(),
        );
    }
    // The live mandate is the authority check above; the source proof is deliberately read
    // through the retained completion path so no worker transcript or live runtime is reused.
    let (retained_link, retained_record) = load_retained_completion(runtime, id.clone()).await?;
    if retained_record.value.mandate.owner != requester
        || retained_record.value.mandate.owner_epoch != record.mandate.owner_epoch
        || retained_link.value.revoked
    {
        return Err("retained candidate authority no longer matches the live mandate owner".into());
    }
    let proof = retained_link
        .value
        .engineering_proof
        .as_ref()
        .ok_or("mandate has no retained engineering proof")?;
    let comparison =
        super::supervision_proof::compare_current_source(app.paths.cache_dir.clone(), proof)
            .await?;
    if !comparison.unchanged {
        return Err("isolated candidate source changed since retained proof".into());
    }
    let (worktree_id, candidate_chat_id, candidate_version) =
        finished_engineering_candidate(app, &record, &retained_link.value).await?;
    let prior = runtime
        .with_store(|store| {
            store.list_documents::<CandidateAssessment>(
                CANDIDATE_ASSESSMENTS,
                None,
                authority::MAX_MANDATES,
            )
        })
        .await
        .map_err(|e| e.to_string())?;
    if prior.next_after.is_some() {
        return Err("candidate assessment history exceeds the bounded admission scan".into());
    }
    if prior.entries.iter().any(|entry| {
        entry.value.mandate_id == id
            && entry.value.worktree_id == worktree_id
            && entry.value.candidate_version == candidate_version
    }) {
        return Err("this completed native candidate has already been assessed".into());
    }
    let binding = NativeCandidateBinding {
        schema_version: 1,
        run_id: run_id.clone(),
        workflow_id: request.workflow_id.clone(),
        project: runtime.project(),
        worker: runtime.worker(),
        mandate_id: id.clone(),
        owner_epoch: record.mandate.owner_epoch,
        candidate_chat_id: candidate_chat_id.clone(),
        worktree_id: worktree_id.clone(),
        candidate_version: candidate_version.clone(),
        requested_by: requester,
        control_id: request.control_id.clone(),
        admitted_at: Utc::now().to_rfc3339(),
    };
    let mut assessment = CandidateAssessment {
        schema_version: 1,
        project: runtime.project(),
        worker: runtime.worker(),
        requested_by: binding.requested_by.clone(),
        mandate_id: id.clone(),
        control_id: request.control_id.clone(),
        workflow_id: request.workflow_id.clone(),
        run_id: run_id.clone(),
        worktree_id,
        candidate_chat_id,
        candidate_version,
        state: "admitted".into(),
        work_id: None,
        artifact_id: None,
        artifact_revision: None,
        action_spec_hash: None,
        error: None,
    };
    let claimed = save_candidate_assessment(runtime, assessment.clone(), None).await?;
    binding.save(runtime).await?;
    authority::validate_record(&record, runtime)?;
    let spec = AutonomousBuddyChatSpec::new(
        request.workflow_id.clone(),
        format!("Retained evidence for supervised worker {}", binding.candidate_chat_id),
        "Deliver retained native candidate evidence only; no new investigation or execution.",
        format!(
            "Isolated candidate worktree {} owned by supervised chat {} at patch version {}; mandate {} targets {:?}.",
            binding.worktree_id,
            binding.candidate_chat_id,
            binding.candidate_version,
            id,
            record.worker_target_files
        ),
    )
    .with_project_root(record.project.canonical_root.clone())
    .with_candidate_binding(binding);
    match super::jobs::autonomous_chats::run_candidate_assessment(
        app.clone(),
        runtime.clone(),
        spec,
    )
    .await
    {
        Ok(report_run_id) if report_run_id == run_id => {
            if let Ok(Some(outcome)) = runtime.get_run_outcome(run_id.clone()).await {
                assessment.state = "reported".into();
                assessment.work_id = Some(outcome.value.work_id.clone());
                if let Ok(Some(work)) = runtime.get_work_item(outcome.value.work_id).await {
                    assessment.artifact_id = work.value.current_artifact_id.clone();
                    assessment.artifact_revision = work.value.current_artifact_revision;
                    assessment.action_spec_hash = runtime
                        .desk_actions(work.value.id.clone())
                        .await
                        .ok()
                        .and_then(|actions| {
                            actions
                                .into_iter()
                                .find(|action| action.spec.authority_class == refact_buddy_core::operator_types::AuthorityClass::Mutating)
                                .map(|action| action.spec_hash)
                        });
                }
            } else {
                assessment.state = "outcome_unknown".into();
            }
        }
        Ok(_) => {
            assessment.state = "outcome_unknown".into();
            assessment.error =
                Some("candidate assessment returned an unexpected report run".into());
        }
        Err(error) => {
            assessment.state = "outcome_unknown".into();
            assessment.error = Some(super::jobs::autonomous_chats::redact_and_cap_text(
                &error, 240,
            ));
        }
    }
    Ok(
        save_candidate_assessment(runtime, assessment, Some(claimed.revision))
            .await?
            .value,
    )
}

pub(crate) async fn reconcile_active(
    app: &AppState,
    runtime: &OutcomeRuntime,
) -> Result<(), String> {
    let links = runtime
        .with_store(|store| {
            store.list_documents::<SupervisionLink>(LINKS, None, authority::MAX_MANDATES)
        })
        .await
        .map_err(|e| e.to_string())?;
    for stored in links.entries {
        if stored.value.delivery_run_id.is_none() {
            // Each operation independently rechecks source project/worker identity.
            if let Err(error) = reconcile(app, runtime, stored.value.mandate_id).await {
                tracing::warn!("supervision reconciliation unavailable: {error}");
            }
        }
    }
    Ok(())
}

/// References for the daemon's observation projection. Never exports a grant,
/// goal body, question answer, tool consent, or replayable command.
pub(crate) async fn daemon_references(
    runtime: &OutcomeRuntime,
) -> Result<Vec<crate::daemon::buddy_reconciliation::SupervisionRef>, String> {
    let links = runtime
        .with_store(|store| {
            store.list_documents::<SupervisionLink>(LINKS, None, authority::MAX_MANDATES)
        })
        .await
        .map_err(|e| e.to_string())?;
    let mut refs = Vec::new();
    for stored in links.entries {
        let link = stored.value;
        if link.project != runtime.project() || link.revoked || link.delivery_run_id.is_some() {
            continue;
        }
        let Ok(record) = authority::load(runtime, link.mandate_id.clone()).await else {
            continue;
        };
        if authority::validate_record(&record.value, runtime).is_err()
            || record.value.mandate.owner_epoch != link.owner_epoch
        {
            continue;
        }
        refs.push(crate::daemon::buddy_reconciliation::SupervisionRef {
            mandate_id: link.mandate_id,
            owner_epoch: link.owner_epoch,
            chat_id: link.chat_id,
            worker_agent_id: link.worker_agent_id,
        });
    }
    refs.sort_by(|a, b| a.mandate_id.cmp(&b.mandate_id));
    Ok(refs)
}

/// Read-only projection for the owning daemon. The rollout and LIVE mandate
/// are rechecked in this worker; no durable reference is treated as a grant.
pub(crate) async fn daemon_projection(
    app: &AppState,
) -> Result<Vec<crate::daemon::buddy_reconciliation::SupervisionRef>, String> {
    let runtime = super::operator_runtime::runtime(
        app,
        super::operator_runtime::OperatorCapability::Supervision,
    )
    .await?;
    daemon_references(&runtime).await
}

fn observe_question(delivery: &mut QuestionDelivery, sets: &[refact_chat_api::PendingQuestionSet]) {
    // Native terminal observations never regress on missing/stale snapshots.
    if question_terminal(delivery) {
        return;
    }
    let Some(set) = sets.iter().find(|set| {
        set.tool_call_id == delivery.tool_call_id && set.asked_at_ms == delivery.asked_at_ms
    }) else {
        if delivery.resolution_id.is_none() {
            delivery.state = "source_unavailable".into();
        }
        return;
    };
    if set.status == refact_chat_api::PendingQuestionStatus::Pending {
        // A repeated pending observation is not another delivery decision.
        return;
    }
    delivery.resolution_id = set.resolution_id.clone();
    delivery.state = match set.status {
        refact_chat_api::PendingQuestionStatus::Pending => "awaiting_native_resolution",
        refact_chat_api::PendingQuestionStatus::Answered
            if set.response.is_none()
                && set.answers == delivery.answers
                && set.resolution_id.is_some() =>
        {
            "resolved"
        }
        refact_chat_api::PendingQuestionStatus::Answered => "resolved_differently",
        refact_chat_api::PendingQuestionStatus::Cancelled => "cancelled",
    }
    .into();
}

fn routine_answers(
    choices: &[authority::RoutineChoice],
    set: &refact_chat_api::PendingQuestionSet,
) -> Result<Vec<refact_chat_api::QuestionAnswer>, String> {
    if set.status != refact_chat_api::PendingQuestionStatus::Pending
        || set.questions.is_empty()
        || set.questions.len() > 12
    {
        return Err("native question is not pending or is out of bounds".into());
    }
    set.questions
        .iter()
        .map(|question| {
            let choice = choices
                .iter()
                .find(|choice| {
                    choice.question_id == question.id
                        && choice.question_text == question.text
                        && question.question_type == "single_select"
                        && question.options.as_ref() == Some(&choice.options)
                        && choice.options.contains(&choice.selected)
                })
                .ok_or("question requires user escalation; no exact typed mandate choice")?;
            Ok(refact_chat_api::QuestionAnswer {
                id: question.id.clone(),
                answer: serde_json::json!(choice.selected),
            })
        })
        .collect()
}

pub(crate) async fn answer_routine_question(
    app: &AppState,
    runtime: &OutcomeRuntime,
    id: String,
) -> Result<SupervisionLink, String> {
    let _dispatch = dispatch_lock().lock().await;
    let record = authorized(runtime, &id, "answer_question").await?;
    let mut saved = load_link(runtime, id).await?;
    let chat_id = saved
        .value
        .child_chat_id
        .as_ref()
        .ok_or("native worker not available")?
        .clone();
    let agent = app
        .agents
        .get(
            &saved.value.chat_id,
            saved
                .value
                .worker_agent_id
                .as_deref()
                .ok_or("native worker not available")?,
        )
        .await?;
    if agent.child_chat_id.as_deref() != Some(&chat_id)
        || agent.status.is_terminal()
        || saved.value.revoked
    {
        return Err("native worker ownership changed".into());
    }
    let session = app
        .chat
        .sessions
        .read()
        .await
        .get(&chat_id)
        .cloned()
        .ok_or("native worker is offline")?;
    let set = session
        .lock()
        .await
        .pending_questions
        .iter()
        .find(|set| set.status == refact_chat_api::PendingQuestionStatus::Pending)
        .cloned()
        .ok_or("no pending native question")?;
    let answers = match routine_answers(&record.routine_choices, &set) {
        Ok(answers) => answers,
        Err(reason) => {
            coalesce(&mut saved.value, &reason, Utc::now().timestamp_millis());
            return Ok(save_link(runtime, saved.value, Some(saved.revision))
                .await?
                .value);
        }
    };
    if saved.value.owner_epoch != record.mandate.owner_epoch {
        return Err("stale supervision owner".into());
    }
    if saved
        .value
        .question_delivery
        .as_ref()
        .is_some_and(|delivery| {
            delivery.tool_call_id == set.tool_call_id && delivery.asked_at_ms == set.asked_at_ms
        })
    {
        observe_question(saved.value.question_delivery.as_mut().unwrap(), &[set]);
        return Ok(save_link(runtime, saved.value, Some(saved.revision))
            .await?
            .value);
    }
    if saved.value.question_history.iter().any(|delivery| {
        delivery.tool_call_id == set.tool_call_id && delivery.asked_at_ms == set.asked_at_ms
    }) {
        return Err("question delivery already attempted; reconcile its native resolution".into());
    }
    if saved.value.question_history.len() >= authority::MAX_QUESTION_DELIVERIES - 1 {
        return Err("mandate question delivery capacity reached; user action required".into());
    }
    if let Some(previous) = saved.value.question_delivery.take() {
        saved.value.question_history.push(previous);
    }
    saved.value.question_delivery = Some(QuestionDelivery {
        tool_call_id: set.tool_call_id.clone(),
        asked_at_ms: set.asked_at_ms,
        answers: answers.clone(),
        state: "dispatch_outcome_unknown".into(),
        resolution_id: None,
    });
    // Persist intent before enqueue; uncertain deliveries never automatically replay.
    saved = save_link(runtime, saved.value, Some(saved.revision)).await?;
    authority::validate_record(&record, runtime)?;
    app.chat
        .facade
        .push_supervised_command(
            &chat_id,
            ChatCommand::AnswerQuestions {
                tool_call_id: set.tool_call_id,
                answers,
                asked_at_ms: Some(set.asked_at_ms),
            },
            authority::guard(&record),
        )
        .await?;
    saved.value.question_delivery.as_mut().unwrap().state = "enqueued".into();
    coalesce(
        &mut saved.value,
        "Typed answer enqueued; native resolution remains authoritative",
        Utc::now().timestamp_millis(),
    );
    Ok(save_link(runtime, saved.value, Some(saved.revision))
        .await?
        .value)
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

fn coalesce(link: &mut SupervisionLink, reason: &str, now: i64) {
    if link.escalation.as_deref() == Some(reason) {
        return;
    }
    link.escalation = Some(reason.into());
    link.escalation_count = link.escalation_count.saturating_add(1);
    link.next_attention_at_ms =
        now.saturating_add(60_000_i64.saturating_mul(1_i64 << link.escalation_count.min(6)));
}

#[cfg(test)]
mod tests {
    use super::*;

    struct CandidateFixture {
        source: super::super::supervision_proof::tests::Fixture,
        app: AppState,
        runtime: OutcomeRuntime,
        agent_id: String,
    }

    impl CandidateFixture {
        async fn new(terminal: bool) -> Self {
            Self::with_registered_agent(terminal, true).await
        }

        async fn with_registered_agent(terminal: bool, registered: bool) -> Self {
            use super::super::supervision_proof as proof_owner;
            use refact_buddy_core::operator_types::{
                AuthorityClass, BudgetLimit, Mandate, ResourceRef, WorkerIdentity,
            };
            let source = proof_owner::tests::fixture(&["src/a.rs"]).await;
            std::fs::write(source.root.join("src/a.rs"), "fn edited() {}\n").unwrap();
            let project = ProjectIdentity {
                project_id: "p".into(),
                canonical_root: source.source.to_string_lossy().into_owned(),
            };
            let worker = WorkerIdentity {
                worker_id: "w".into(),
                engine_instance_id: "e".into(),
                epoch: 1,
            };
            let mut value = link();
            value.project = project.clone();
            value.mandate_id = source.mandate_id.clone();
            value.chat_id = format!("parent-of-{}", source.chat_id);
            value.child_chat_id = Some(source.chat_id.clone());
            value.dispatch_attempted = true;
            let facts = proof_owner::SessionFacts {
                chat_id: source.chat_id.clone(),
                goal_version: 1,
                goal_content: value.intake_content.clone(),
                goal_criteria: value.intake_criteria.clone(),
            };
            let capture = proof_owner::capture_before_stream(source.cache_dir.clone(), &facts)
                .await
                .unwrap()
                .unwrap();
            let reply = "GOAL: MET\nCRITERION C1: MET — isolated source observed";
            proof_owner::bind_after_stream(capture, reply)
                .await
                .unwrap();
            let at_ms = Utc::now().timestamp_millis() as u64;
            assert!(proof_owner::finalize_met(
                &source.chat_id,
                1,
                &facts.goal_content,
                &facts.goal_criteria,
                reply,
                at_ms
            ));
            value.engineering_proof =
                Some(proof_owner::get_authorized_proof(&source.chat_id).unwrap());
            value.delivery_proof = Some(GoalSnapshot {
                content: value.intake_content.clone(),
                criteria: value.intake_criteria.clone(),
                version: 1,
                status: GoalStatus::Completed,
                attempts: vec![refact_chat_api::GoalAttempt {
                    at_ms,
                    verdict: "met".into(),
                    verifier_reply: reply.into(),
                    criteria_verdicts: vec![refact_chat_api::CriterionVerdict {
                        id: "C1".into(),
                        met: true,
                        note: "source observed".into(),
                        source: refact_chat_api::VerdictSource::Judged,
                        evidence: None,
                    }],
                    ..Default::default()
                }],
                ..Default::default()
            });
            let mut gcx = crate::global_context::tests::make_test_gcx_with_dirs(
                source.cache_dir.clone(),
                source.temp.path().join("config"),
            )
            .await;
            std::sync::Arc::get_mut(&mut gcx)
                .unwrap()
                .cmdline
                .daemon_auth_token = Some("fixture-secret".into());
            *gcx.documents_state.workspace_folders.lock().unwrap() = vec![source.source.clone()];
            let app = AppState::from_gcx(gcx).await;
            let owner = NativeSourceIdentity {
                source_kind: NativeSourceKind::External,
                source_id: "authenticated-user".into(),
                source_version: "operator-http-v1".into(),
            };
            let record = MandateRecord {
                schema_version: 1,
                project: project.clone(),
                worker: worker.clone(),
                granted_by: owner.clone(),
                max_workers: 1,
                routine_choices: vec![],
                steering_notes: vec![],
                worker_tools: vec!["update_textdoc".into()],
                worker_target_files: source.targets.clone(),
                mandate: Mandate {
                    schema_version: 1,
                    mandate_id: source.mandate_id.clone(),
                    project: project.clone(),
                    owner,
                    owner_epoch: 1,
                    resource_scopes: vec![ResourceRef {
                        resource_kind: "project".into(),
                        resource_id: "p".into(),
                        version: "1".into(),
                    }],
                    allowed_operations: vec![
                        "read_project".into(),
                        "engineer_project".into(),
                        "spawn_worker".into(),
                    ],
                    allowed_authority_classes: vec![
                        AuthorityClass::ReadOnly,
                        AuthorityClass::Mutating,
                    ],
                    model_constraints: vec!["test-model".into()],
                    budget_limit: Some(BudgetLimit {
                        max_turns: Some(2),
                        max_tokens: Some(8192),
                        max_duration_seconds: Some(60),
                        max_cost_micros: None,
                    }),
                    expires_at: Some((Utc::now() + chrono::Duration::minutes(5)).to_rfc3339()),
                    revoked_at: None,
                },
            };
            let (agent, _, _) = app
                .agents
                .create(crate::agents::types::CreateAgentRequest {
                    parent_chat_id: value.chat_id.clone(),
                    parent_root_chat_id: Some(value.chat_id.clone()),
                    parent_tool_call_id: None,
                    kind: crate::agents::types::BgAgentKind::Subagent,
                    config_name: "candidate-fixture".into(),
                    title: "candidate fixture".into(),
                    prompt: "fixture".into(),
                    target_files: source.targets.clone(),
                    model: "test-model".into(),
                    model_type: None,
                    goal_summary: None,
                    plan_present: false,
                    worktree_id: None,
                    worktree_branch: None,
                })
                .await
                .unwrap();
            app.agents
                .mark_running(&agent.agent_id, source.chat_id.clone())
                .await
                .unwrap();
            if terminal {
                app.agents
                    .mark_completed(
                        &agent.agent_id,
                        crate::agents::types::AgentCompletion {
                            result_summary: "retained source proof".into(),
                            edited_files: source.targets.clone(),
                            diff_summary: None,
                            conflict_summary: None,
                            child_chat_id: Some(source.chat_id.clone()),
                            // This fixture declares no deliverable contract and
                            // makes no handoff claim: `None` is `unknown`.
                            deliverable_status: None,
                            handoff: None,
                        },
                    )
                    .await
                    .unwrap();
            }
            value.worker_agent_id = Some(if registered {
                agent.agent_id.clone()
            } else {
                "unknown-worker".into()
            });
            proof_owner::validate_retained(
                value.engineering_proof.as_ref().unwrap(),
                &record,
                &source.chat_id,
                value.delivery_proof.as_ref().unwrap(),
            )
            .unwrap();
            let mut store = OperatorStore::open(&source.source, &project).unwrap();
            store
                .reserve_native_documents(
                    "supervision",
                    &source.mandate_id,
                    &authority::reservation_slots(&source.mandate_id),
                )
                .unwrap();
            store
                .put_native_ordinary_document(
                    "supervision",
                    &source.mandate_id,
                    authority::MANDATES,
                    &source.mandate_id,
                    &record,
                    None,
                )
                .unwrap();
            persist_link(&mut store, value, None).unwrap();
            let runtime = OutcomeRuntime::from_store(store, project, worker);
            authority::validate_record(&record, &runtime).unwrap();
            let mut service = crate::buddy::actor::BuddyService::new(
                source.source.clone(),
                crate::buddy::state::default_buddy_state(),
                Default::default(),
                vec![],
                crate::buddy::runtime_queue::RuntimeQueue::new(),
                tokio::sync::broadcast::channel(64).0,
                None,
            );
            service.operator_features.storage = true;
            service.operator_features.supervision = true;
            service.outcome_runtime = Some(runtime.clone());
            *app.buddy.buddy.lock().await = Some(service);
            Self {
                source,
                app,
                runtime,
                agent_id: agent.agent_id,
            }
        }

        async fn post(
            &self,
            body: serde_json::Value,
            override_header: Option<(&str, &str)>,
        ) -> (axum::http::StatusCode, serde_json::Value) {
            use tower::ServiceExt;
            let mut headers = axum::http::HeaderMap::new();
            for (key, value) in [
                ("authorization", "Bearer fixture-secret"),
                ("x-refact-project-id", "p"),
                ("x-refact-worker-id", "w"),
                ("x-refact-engine-instance", "e"),
                ("x-refact-worker-epoch", "1"),
                ("content-type", "application/json"),
            ] {
                headers.insert(
                    axum::http::header::HeaderName::from_bytes(key.as_bytes()).unwrap(),
                    value.parse().unwrap(),
                );
            }
            if let Some((key, value)) = override_header {
                headers.insert(
                    axum::http::header::HeaderName::from_bytes(key.as_bytes()).unwrap(),
                    value.parse().unwrap(),
                );
            }
            let mut request = axum::http::Request::builder().method("POST").uri(format!(
                "/buddy/supervision/mandates/{}/assess-candidate",
                self.source.mandate_id
            ));
            *request.headers_mut().unwrap() = headers;
            let response = crate::http::routers::v1::make_v1_router(self.app.clone())
                .with_state(self.app.clone())
                .oneshot(
                    request
                        .body(axum::body::Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status();
            let bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
            (
                status,
                serde_json::from_slice(&bytes).unwrap_or_else(
                    |_| serde_json::json!({"error": String::from_utf8_lossy(&bytes)}),
                ),
            )
        }

        async fn seq(&self) -> u64 {
            self.runtime
                .with_store(|store| Ok(store.retention_preview()?.sequence_high_watermark))
                .await
                .unwrap()
        }

        async fn amend_mandate(&self, change: fn(&mut MandateRecord)) {
            let id = self.source.mandate_id.clone();
            self.runtime
                .with_store(move |store| {
                    let mut record = store
                        .get_document::<MandateRecord>(authority::MANDATES, &id)?
                        .unwrap();
                    change(&mut record.value);
                    store.put_native_ordinary_document(
                        "supervision",
                        &id,
                        authority::MANDATES,
                        &id,
                        &record.value,
                        Some(record.revision),
                    )?;
                    Ok(())
                })
                .await
                .unwrap();
        }

        async fn close(self) {
            self.runtime.shutdown().await;
            authority::invalidate(&self.source.mandate_id).unwrap();
            super::super::supervision_proof::acknowledge_authorized(&self.source.chat_id);
        }
    }

    #[tokio::test]
    async fn candidate_assessment_router_connects_all_code_families_without_execution_or_replay() {
        use refact_buddy_core::autonomous_workflows::*;
        use refact_buddy_core::operator_types::{AuthorityClass, ConfirmationClass, RunStatus};
        let families = [
            REFACT_SELF_CRITIC_WORKFLOW_ID,
            REFACT_COMPILE_SNIFFER_WORKFLOW_ID,
            BUDDY_REFACTOR_HUNTER_WORKFLOW_ID,
            BUDDY_TEST_COVERAGE_WATCHER_WORKFLOW_ID,
            DOCS_GARDENER_WORKFLOW_ID,
            ARCHITECTURE_DRIFT_WORKFLOW_ID,
            SECURITY_WHISPERER_WORKFLOW_ID,
            DEPENDENCY_RADAR_WORKFLOW_ID,
        ];
        for workflow in families {
            let fx = CandidateFixture::new(true).await;
            let before_bytes = std::fs::read(fx.source.root.join("src/a.rs")).unwrap();
            let before_head = crate::worktrees::git::head_rev(&fx.source.root).unwrap();
            let before_sessions = fx.app.chat.sessions.read().await.len();
            let native = crate::worktrees::service::WorktreeService::new(
                fx.source.cache_dir.clone(),
                fx.source.source.clone(),
            )
            .unwrap();
            let saved = load_link(&fx.runtime, fx.source.mandate_id.clone())
                .await
                .unwrap();
            let worktree_id = saved
                .value
                .engineering_proof
                .as_ref()
                .unwrap()
                .source
                .worktree_id
                .clone();
            let before_version =
                super::super::resources::project_code::registered_candidate_version(
                    &native,
                    &worktree_id,
                )
                .await
                .unwrap();
            assert!(fx
                .runtime
                .with_store(|store| store
                    .list_documents::<super::super::workflow_deliverables::NativeCandidateBinding>(
                    super::super::workflow_deliverables::CANDIDATE_BINDINGS,
                    None,
                    10
                ))
                .await
                .unwrap()
                .entries
                .is_empty());
            let body = serde_json::json!({"control_id":"assess-once", "workflow_id":workflow});
            let (status, response) = fx.post(body.clone(), None).await;
            assert_eq!(status, axum::http::StatusCode::OK, "{workflow}: {response}");
            assert_eq!(response["state"], "reported", "{workflow}: {response}");
            let run_id = response["run_id"].as_str().unwrap();
            assert_ne!(run_id, fx.source.chat_id);
            let binding = super::super::workflow_deliverables::NativeCandidateBinding::load(
                &fx.runtime,
                run_id,
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(binding.candidate_chat_id, fx.source.chat_id);
            assert_eq!(binding.workflow_id, workflow);
            let outcome = fx
                .runtime
                .get_run_outcome(run_id.into())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(outcome.value.status, RunStatus::Inconclusive);
            assert_eq!(outcome.value.origin, "native_candidate_evidence_delivery");
            assert!(outcome.value.effects.is_empty());
            assert!(outcome
                .value
                .evidence
                .iter()
                .any(|e| e.source.source_id == fx.source.chat_id));
            let work = fx
                .runtime
                .get_work_item(outcome.value.work_id.clone())
                .await
                .unwrap()
                .unwrap();
            let artifact = fx
                .runtime
                .get_artifact_revision(
                    work.value.current_artifact_id.clone().unwrap(),
                    work.value.current_artifact_revision,
                )
                .await
                .unwrap()
                .unwrap();
            assert!(artifact
                .value
                .body
                .contains("Retained native evidence only"));
            assert!(artifact
                .value
                .body
                .contains("no new tests or verification were performed"));
            let actions = fx
                .runtime
                .desk_actions(work.value.id.clone())
                .await
                .unwrap();
            let action = actions
                .iter()
                .find(|a| a.spec.adapter == "project_code")
                .expect("connected candidate action");
            assert_eq!(response["action_spec_hash"], action.spec_hash);
            assert_eq!(
                action.spec.confirmation,
                ConfirmationClass::UserConfirmationRequired
            );
            assert_eq!(action.spec.authority_class, AuthorityClass::Mutating);
            assert_eq!(action.spec.expected_version, before_version);
            assert!(action.latest_execution.is_none());
            assert!(super::super::resource_execution::load_prepared(
                &fx.runtime,
                action.spec_hash.clone()
            )
            .await
            .is_ok());
            let sequence = fx.seq().await;
            assert_eq!(fx.post(body, None).await, (status, response.clone()));
            assert_eq!(fx.seq().await, sequence);
            let other = if workflow == DEPENDENCY_RADAR_WORKFLOW_ID {
                SECURITY_WHISPERER_WORKFLOW_ID
            } else {
                DEPENDENCY_RADAR_WORKFLOW_ID
            };
            assert_eq!(
                fx.post(
                    serde_json::json!({"control_id":"assess-once", "workflow_id":other}),
                    None
                )
                .await
                .0,
                axum::http::StatusCode::CONFLICT
            );
            assert_eq!(
                fx.post(
                    serde_json::json!({"control_id":"assess-again", "workflow_id":workflow}),
                    None
                )
                .await
                .0,
                axum::http::StatusCode::CONFLICT
            );
            assert_eq!(fx.seq().await, sequence);
            assert_eq!(
                std::fs::read(fx.source.root.join("src/a.rs")).unwrap(),
                before_bytes
            );
            assert_eq!(
                crate::worktrees::git::head_rev(&fx.source.root).unwrap(),
                before_head
            );
            assert_eq!(
                super::super::resources::project_code::registered_candidate_version(
                    &native,
                    &worktree_id
                )
                .await
                .unwrap(),
                before_version
            );
            assert_eq!(fx.app.chat.sessions.read().await.len(), before_sessions);
            assert!(!fx.app.chat.sessions.read().await.contains_key(run_id));
            std::fs::write(
                fx.source.root.join("src/a.rs"),
                "fn changed_after_delivery() {}\n",
            )
            .unwrap();
            assert!(
                validate_candidate_assessment(&fx.app, &fx.runtime, &binding)
                    .await
                    .is_err()
            );
            let artifact_after = fx
                .runtime
                .get_artifact_revision(
                    artifact.value.artifact_id.clone(),
                    Some(artifact.value.revision),
                )
                .await
                .unwrap()
                .unwrap();
            assert_eq!(artifact_after, artifact);
            assert_eq!(fx.seq().await, sequence);
            fx.close().await;
        }
    }

    #[tokio::test]
    async fn candidate_assessment_interrupted_claim_survives_reopen_without_replay() {
        let mut fx = CandidateFixture::new(true).await;
        let workflow = refact_buddy_core::autonomous_workflows::DEPENDENCY_RADAR_WORKFLOW_ID;
        let control_id = "interrupted";
        let key = format!("{}:{control_id}", fx.source.mandate_id);
        let run_id = format!("buddy-{workflow}-{}", &sha256(key.as_bytes())[..32]);
        let record = authority::load(&fx.runtime, fx.source.mandate_id.clone())
            .await
            .unwrap()
            .value;
        let link = load_link(&fx.runtime, fx.source.mandate_id.clone())
            .await
            .unwrap()
            .value;
        let (worktree_id, candidate_chat_id, candidate_version) =
            finished_engineering_candidate(&fx.app, &record, &link)
                .await
                .unwrap();
        save_candidate_assessment(
            &fx.runtime,
            CandidateAssessment {
                schema_version: 1,
                project: fx.runtime.project(),
                worker: fx.runtime.worker(),
                requested_by: record.mandate.owner,
                mandate_id: fx.source.mandate_id.clone(),
                control_id: control_id.into(),
                workflow_id: workflow.into(),
                run_id: run_id.clone(),
                worktree_id,
                candidate_chat_id,
                candidate_version,
                state: "admitted".into(),
                work_id: None,
                artifact_id: None,
                artifact_revision: None,
                action_spec_hash: None,
                error: None,
            },
            None,
        )
        .await
        .unwrap();
        let project = fx.runtime.project();
        let worker = fx.runtime.worker();
        fx.runtime.shutdown().await;
        fx.runtime = OutcomeRuntime::open(&fx.source.source, project, worker).unwrap();
        fx.app
            .buddy
            .buddy
            .lock()
            .await
            .as_mut()
            .unwrap()
            .outcome_runtime = Some(fx.runtime.clone());
        let before = fx.seq().await;
        let body = serde_json::json!({"control_id":control_id, "workflow_id":workflow});
        let (status, response) = fx.post(body.clone(), None).await;
        assert_eq!(status, axum::http::StatusCode::OK, "{response}");
        assert_eq!(response["state"], "outcome_unknown");
        assert!(response["action_spec_hash"].is_null());
        assert!(fx
            .runtime
            .get_run_outcome(run_id.clone())
            .await
            .unwrap()
            .is_none());
        assert!(
            super::super::workflow_deliverables::NativeCandidateBinding::load(&fx.runtime, &run_id)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(fx.post(body, None).await, (status, response));
        assert_eq!(fx.seq().await, before);
        fx.close().await;
    }

    #[tokio::test]
    async fn candidate_assessment_router_refuses_foreign_auth_binding_and_malformed_body_without_writes(
    ) {
        let fx = CandidateFixture::new(true).await;
        let body = serde_json::json!({"control_id":"refusal", "workflow_id":refact_buddy_core::autonomous_workflows::DEPENDENCY_RADAR_WORKFLOW_ID});
        let seq = fx.seq().await;
        for (header, value, expected) in [
            (
                "authorization",
                "Bearer wrong",
                axum::http::StatusCode::UNAUTHORIZED,
            ),
            (
                "x-refact-project-id",
                "foreign",
                axum::http::StatusCode::CONFLICT,
            ),
            (
                "x-refact-worker-id",
                "foreign",
                axum::http::StatusCode::CONFLICT,
            ),
            (
                "x-refact-engine-instance",
                "foreign",
                axum::http::StatusCode::CONFLICT,
            ),
            (
                "x-refact-worker-epoch",
                "2",
                axum::http::StatusCode::CONFLICT,
            ),
        ] {
            assert_eq!(
                fx.post(body.clone(), Some((header, value))).await.0,
                expected
            );
            assert_eq!(fx.seq().await, seq);
        }
        let mut malformed = body.clone();
        malformed["worktree_id"] = "foreign".into();
        assert_eq!(
            fx.post(malformed, None).await.0,
            axum::http::StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(fx.seq().await, seq);
        fx.close().await;
    }

    #[tokio::test]
    async fn candidate_assessment_router_refuses_stale_authority_source_and_worker_without_writes()
    {
        use refact_buddy_core::operator_types::AuthorityClass;
        for case in [
            "owner",
            "revoked",
            "expired",
            "read-only",
            "busy",
            "unknown",
            "source",
        ] {
            let fx =
                CandidateFixture::with_registered_agent(case != "busy", case != "unknown").await;
            match case {
                "owner" => {
                    fx.amend_mandate(|r| r.mandate.owner.source_id = "foreign".into())
                        .await
                }
                "revoked" => {
                    fx.amend_mandate(|r| r.mandate.revoked_at = Some(Utc::now().to_rfc3339()))
                        .await
                }
                "expired" => {
                    fx.amend_mandate(|r| r.mandate.expires_at = Some("2000-01-01T00:00:00Z".into()))
                        .await
                }
                "read-only" => {
                    fx.amend_mandate(|r| {
                        r.mandate
                            .allowed_operations
                            .retain(|op| op != "engineer_project");
                        r.mandate.allowed_authority_classes = vec![AuthorityClass::ReadOnly];
                    })
                    .await
                }
                "unknown" => {
                    let stored = load_link(&fx.runtime, fx.source.mandate_id.clone())
                        .await
                        .unwrap();
                    assert!(fx
                        .app
                        .agents
                        .get(&stored.value.chat_id, "unknown-worker")
                        .await
                        .is_err());
                }
                "source" => std::fs::write(
                    fx.source.root.join("src/a.rs"),
                    "fn changed_after_proof() {}\n",
                )
                .unwrap(),
                "busy" => assert!(!fx
                    .app
                    .agents
                    .get(&format!("parent-of-{}", fx.source.chat_id), &fx.agent_id)
                    .await
                    .unwrap()
                    .status
                    .is_terminal()),
                _ => unreachable!(),
            }
            let seq = fx.seq().await;
            let bytes = std::fs::read(fx.source.root.join("src/a.rs")).unwrap();
            let (status, response) = fx.post(serde_json::json!({"control_id":"refusal", "workflow_id":refact_buddy_core::autonomous_workflows::DEPENDENCY_RADAR_WORKFLOW_ID}), None).await;
            assert_eq!(
                status,
                axum::http::StatusCode::CONFLICT,
                "{case}: {response}"
            );
            assert_eq!(fx.seq().await, seq, "{case}");
            assert_eq!(
                std::fs::read(fx.source.root.join("src/a.rs")).unwrap(),
                bytes
            );
            fx.close().await;
        }
    }

    struct NativeControlFixture {
        _root: tempfile::TempDir,
        app: AppState,
        runtime: OutcomeRuntime,
        mandate_id: String,
        owner: NativeSourceIdentity,
        parent: String,
        child: String,
        agent_id: String,
    }

    impl NativeControlFixture {
        async fn new() -> Self {
            use crate::agents::types::CreateAgentRequest;
            use refact_buddy_core::operator_types::{
                AuthorityClass, Mandate, ResourceRef, WorkerIdentity,
            };

            let root = tempfile::tempdir().unwrap();
            let project = ProjectIdentity {
                project_id: format!("native-control-{}", uuid::Uuid::new_v4()),
                canonical_root: dunce::canonicalize(root.path())
                    .unwrap()
                    .to_string_lossy()
                    .into(),
            };
            let worker = WorkerIdentity {
                worker_id: "native-control-worker".into(),
                engine_instance_id: "native-control-engine".into(),
                epoch: 1,
            };
            let runtime = OutcomeRuntime::open(root.path(), project.clone(), worker).unwrap();
            let app = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
            let owner = NativeSourceIdentity {
                source_kind: NativeSourceKind::Chat,
                source_id: "native-control-owner".into(),
                source_version: "1".into(),
            };
            let mandate_id = format!("native-control-{}", uuid::Uuid::new_v4());
            authority::create(
                &runtime,
                owner.clone(),
                authority::MandateRequest {
                    mandate: Mandate {
                        schema_version: 1,
                        mandate_id: mandate_id.clone(),
                        project: project.clone(),
                        owner: owner.clone(),
                        owner_epoch: 1,
                        resource_scopes: vec![ResourceRef {
                            resource_kind: "project".into(),
                            resource_id: project.project_id.clone(),
                            version: "1".into(),
                        }],
                        allowed_operations: vec![
                            "create_goal".into(),
                            "read_project".into(),
                            "steer_worker".into(),
                        ],
                        allowed_authority_classes: vec![AuthorityClass::ReadOnly],
                        model_constraints: vec!["test-model".into()],
                        budget_limit: Some(refact_buddy_core::operator_types::BudgetLimit {
                            max_turns: Some(2),
                            max_tokens: Some(8192),
                            max_duration_seconds: Some(60),
                            max_cost_micros: None,
                        }),
                        expires_at: Some((Utc::now() + chrono::Duration::minutes(5)).to_rfc3339()),
                        revoked_at: None,
                    },
                    max_workers: 1,
                    routine_choices: vec![],
                    worker_tools: vec![],
                    worker_target_files: vec![],
                    steering_notes: vec![authority::SteeringNote {
                        note_id: "focus".into(),
                        text: "Use the exact mandate note.".into(),
                    }],
                },
            )
            .await
            .unwrap();
            let parent = intake(
                &app,
                &runtime,
                mandate_id.clone(),
                GoalIntake {
                    content: "Inspect the native control boundary.".into(),
                    criteria: vec![GoalCriterion {
                        id: "C1".into(),
                        text: "Report control effects.".into(),
                        verify_hint: None,
                        verify_command: None,
                    }],
                },
            )
            .await
            .unwrap()
            .chat_id;
            for _ in 0..32 {
                if app
                    .chat
                    .facade
                    .session_snapshot(&parent)
                    .await
                    .unwrap()
                    .goal
                    .is_some_and(|goal| goal.status == GoalStatus::Paused)
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
            let child = format!("native-control-child-{}", uuid::Uuid::new_v4());
            authority::bind_chat(&mandate_id, child.clone()).unwrap();
            app.chat
                .facade
                .create_session(refact_runtime_api::CreateSessionRequest {
                    chat_id: child.clone(),
                    thread: refact_chat_api::ThreadParams {
                        id: child.clone(),
                        model: "test-model".into(),
                        ..Default::default()
                    },
                    messages: vec![],
                })
                .await
                .unwrap();
            let record = authority::load(&runtime, mandate_id.clone())
                .await
                .unwrap()
                .value;
            app.chat
                .facade
                .push_supervised_command(
                    &child,
                    ChatCommand::SetGoal {
                        plan: None,
                        content: "Inspect the native control boundary.".into(),
                        criteria: Some(vec![GoalCriterion {
                            id: "C1".into(),
                            text: "Report control effects.".into(),
                            verify_hint: None,
                            verify_command: None,
                        }]),
                        budget: Some(native_budget(&record).unwrap()),
                    },
                    authority::guard(&record),
                )
                .await
                .unwrap();
            for _ in 0..32 {
                if app
                    .chat
                    .facade
                    .session_snapshot(&child)
                    .await
                    .unwrap()
                    .goal
                    .is_some()
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
            let (agent, _, _) = app
                .agents
                .create(CreateAgentRequest {
                    parent_chat_id: parent.clone(),
                    parent_root_chat_id: Some(parent.clone()),
                    parent_tool_call_id: None,
                    kind: crate::agents::types::BgAgentKind::Subagent,
                    config_name: "native-control".into(),
                    title: "native control".into(),
                    prompt: "fixture".into(),
                    target_files: vec![],
                    model: "test-model".into(),
                    model_type: None,
                    goal_summary: None,
                    plan_present: false,
                    worktree_id: None,
                    worktree_branch: None,
                })
                .await
                .unwrap();
            let agent = app
                .agents
                .mark_running(&agent.agent_id, child.clone())
                .await
                .unwrap();
            let mut link = load_link(&runtime, mandate_id.clone()).await.unwrap();
            link.value.worker_agent_id = Some(agent.agent_id.clone());
            link.value.child_chat_id = Some(child.clone());
            link.value.dispatch_attempted = true;
            link.value.child_goal_version = Some(1);
            save_link(&runtime, link.value, Some(link.revision))
                .await
                .unwrap();
            Self {
                _root: root,
                app,
                runtime,
                mandate_id,
                owner,
                parent,
                child,
                agent_id: agent.agent_id,
            }
        }
    }

    #[tokio::test]
    async fn native_controls_pause_message_resume_and_reject_replayed_or_offline_workers() {
        let fixture = NativeControlFixture::new().await;
        let pause = ControlRequest {
            control_id: "pause-once".into(),
        };
        pause_worker(
            &fixture.app,
            &fixture.runtime,
            fixture.mandate_id.clone(),
            fixture.owner.clone(),
            pause.clone(),
        )
        .await
        .unwrap();
        let observed = status(&fixture.app, &fixture.runtime, fixture.mandate_id.clone())
            .await
            .unwrap();
        assert!(observed.native_pause.as_ref().unwrap().requested);
        assert!(!observed.native_pause.as_ref().unwrap().quiescent);
        let gate = fixture
            .app
            .agents
            .pause_gate(&fixture.agent_id)
            .await
            .unwrap();
        let held = tokio::spawn({
            let gate = gate.clone();
            async move { gate.hold(None).await }
        });
        gate.quiesced().await;
        assert!(
            status(&fixture.app, &fixture.runtime, fixture.mandate_id.clone())
                .await
                .unwrap()
                .native_pause
                .unwrap()
                .quiescent
        );
        let duplicate = pause_worker(
            &fixture.app,
            &fixture.runtime,
            fixture.mandate_id.clone(),
            fixture.owner.clone(),
            pause,
        )
        .await
        .unwrap();
        assert_eq!(duplicate.control_history.len(), 1);
        assert!(message_worker(
            &fixture.app,
            &fixture.runtime,
            fixture.mandate_id.clone(),
            fixture.owner.clone(),
            MessageRequest {
                control_id: "pause-once".into(),
                note_id: "focus".into()
            }
        )
        .await
        .is_err());
        let mut other = fixture.owner.clone();
        other.source_id = "other".into();
        assert!(resume_worker(
            &fixture.app,
            &fixture.runtime,
            fixture.mandate_id.clone(),
            other,
            ControlRequest {
                control_id: "resume-other".into()
            }
        )
        .await
        .is_err());
        message_worker(
            &fixture.app,
            &fixture.runtime,
            fixture.mandate_id.clone(),
            fixture.owner.clone(),
            MessageRequest {
                control_id: "message-once".into(),
                note_id: "focus".into(),
            },
        )
        .await
        .unwrap();
        let note = fixture
            .app
            .agents
            .drain_native_notes(&fixture.agent_id)
            .await
            .pop()
            .unwrap();
        assert_eq!(
            note.consume(&fixture.child, "test-model").unwrap(),
            "Use the exact mandate note."
        );
        message_worker(
            &fixture.app,
            &fixture.runtime,
            fixture.mandate_id.clone(),
            fixture.owner.clone(),
            MessageRequest {
                control_id: "revoked-note".into(),
                note_id: "focus".into(),
            },
        )
        .await
        .unwrap();
        resume_worker(
            &fixture.app,
            &fixture.runtime,
            fixture.mandate_id.clone(),
            fixture.owner.clone(),
            ControlRequest {
                control_id: "resume-once".into(),
            },
        )
        .await
        .unwrap();
        assert!(!held.await.unwrap());
        assert_eq!(
            fixture
                .app
                .chat
                .facade
                .session_snapshot(&fixture.parent)
                .await
                .unwrap()
                .goal
                .unwrap()
                .status,
            GoalStatus::Paused
        );
        authority::invalidate(&fixture.mandate_id).unwrap();
        let revoked = fixture
            .app
            .agents
            .drain_native_notes(&fixture.agent_id)
            .await
            .pop()
            .unwrap();
        assert!(revoked.consume(&fixture.child, "test-model").is_err());
        fixture
            .app
            .agents
            .mark_interrupted(&fixture.agent_id, "restart".into())
            .await
            .unwrap();
        assert!(pause_worker(
            &fixture.app,
            &fixture.runtime,
            fixture.mandate_id.clone(),
            fixture.owner,
            ControlRequest {
                control_id: "offline".into()
            }
        )
        .await
        .is_err());
    }

    fn link() -> SupervisionLink {
        SupervisionLink {
            schema_version: 1,
            project: ProjectIdentity {
                project_id: "p".into(),
                canonical_root: "/p".into(),
            },
            mandate_id: "m".into(),
            owner_epoch: 1,
            chat_id: "c".into(),
            goal_version: Some(1),
            child_goal_version: Some(1),
            question_delivery: None,
            question_history: vec![],
            intake_content: "objective".into(),
            intake_criteria: vec![GoalCriterion {
                id: "C1".into(),
                text: "proof".into(),
                verify_hint: None,
                verify_command: None,
            }],
            worker_agent_id: None,
            child_chat_id: None,
            dispatch_attempted: false,
            command_acknowledged: false,
            revoked: false,
            escalation: None,
            escalation_count: 0,
            next_attention_at_ms: 0,
            delivery_run_id: None,
            delivery_proof: None,
            engineering_proof: None,
            control_history: vec![],
            pause_requested: false,
        }
    }

    fn saturated_store(
        legacy: bool,
    ) -> (tempfile::TempDir, OperatorStore, Stored<SupervisionLink>) {
        use super::super::operator_observability::DocumentGrowthPolicy;
        let root = tempfile::TempDir::new().unwrap();
        let mut value = link();
        value.project.canonical_root = dunce::canonicalize(root.path())
            .unwrap()
            .to_string_lossy()
            .into();
        let mut store = OperatorStore::open(root.path(), &value.project).unwrap();
        let owner = NativeSourceIdentity {
            source_kind: NativeSourceKind::Chat,
            source_id: "user".into(),
            source_version: "1".into(),
        };
        let mandate = MandateRecord {
            schema_version: 1,
            project: value.project.clone(),
            worker: refact_buddy_core::operator_types::WorkerIdentity {
                worker_id: "w".into(),
                engine_instance_id: "e".into(),
                epoch: 1,
            },
            granted_by: owner.clone(),
            max_workers: 1,
            routine_choices: vec![],
            worker_tools: vec![],
            worker_target_files: vec![],
            steering_notes: vec![],
            mandate: refact_buddy_core::operator_types::Mandate {
                schema_version: 1,
                mandate_id: "m".into(),
                project: value.project.clone(),
                owner,
                owner_epoch: 1,
                resource_scopes: vec![],
                allowed_operations: vec![],
                allowed_authority_classes: vec![],
                model_constraints: vec![],
                budget_limit: None,
                expires_at: None,
                revoked_at: None,
            },
        };
        let saved = if legacy {
            store
                .put_document(authority::MANDATES, "m", &mandate, None)
                .unwrap();
            store.put_document(LINKS, "m", &value, None).unwrap()
        } else {
            store
                .reserve_native_documents("supervision", "m", &authority::reservation_slots("m"))
                .unwrap();
            store
                .put_native_ordinary_document(
                    "supervision",
                    "m",
                    authority::MANDATES,
                    "m",
                    &mandate,
                    None,
                )
                .unwrap();
            persist_link(&mut store, value.clone(), None).unwrap()
        };
        store
            .set_document_growth_policy(Some(DocumentGrowthPolicy {
                max_rows_per_namespace: 2,
                replace_editorial_aggregate: false,
            }))
            .unwrap();
        store
            .set_capacity_policy(Some(super::super::operator_observability::CapacityPolicy {
                max_rows_per_table: 1,
                max_database_bytes: 1,
            }))
            .unwrap();
        for n in 0..10_000 {
            if store
                .put_document(&format!("flood_{n}"), "row", &mandate, None)
                .is_err()
            {
                break;
            }
        }
        assert!(store
            .put_document("overflow", "row", &mandate, None)
            .is_err());
        (root, store, saved)
    }

    #[tokio::test]
    async fn concurrent_cancel_is_single_record_and_payload_reuse_is_rejected() {
        let (_root, store, saved) = saturated_store(false);
        let record = store
            .get_document::<MandateRecord>(authority::MANDATES, "m")
            .unwrap()
            .unwrap()
            .value;
        let requester = record.mandate.owner.clone();
        let runtime = OutcomeRuntime::from_store(store, saved.value.project, record.worker);
        let app = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
        let request = ControlRequest {
            control_id: "cancel-once".into(),
        };
        let (first, second) = tokio::join!(
            cancel_worker(
                &app,
                &runtime,
                "m".into(),
                requester.clone(),
                request.clone()
            ),
            cancel_worker(&app, &runtime, "m".into(), requester.clone(), request),
        );
        assert_eq!(
            serde_json::to_value(first.unwrap()).unwrap(),
            serde_json::to_value(second.unwrap()).unwrap()
        );
        let stopped = load_link(&runtime, "m".into()).await.unwrap();
        assert!(stopped.value.revoked);
        assert_eq!(stopped.value.control_history.len(), 1);
        assert!(matches!(
            stopped.value.control_history[0].outcome.as_str(),
            "revoked_and_cancelled" | "revoked_cancellation_unconfirmed"
        ));
        assert!(duplicate_control(
            &stopped.value,
            "cancel-once",
            ControlKind::Pause,
            None,
            &requester
        )
        .is_err());
        assert!(duplicate_control(
            &stopped.value,
            "cancel-once",
            ControlKind::Cancel,
            Some("other"),
            &requester
        )
        .is_err());
        let mut foreign = requester;
        foreign.source_id = "other-user".into();
        assert!(cancel_worker(
            &app,
            &runtime,
            "m".into(),
            foreign,
            ControlRequest {
                control_id: "cancel-once".into()
            }
        )
        .await
        .is_err());
        assert_eq!(
            load_link(&runtime, "m".into()).await.unwrap().revision,
            stopped.revision
        );
        runtime.shutdown().await;
    }

    #[tokio::test]
    async fn actual_stop_and_revoke_are_idempotent_at_saturation() {
        let (_root, store, saved) = saturated_store(false);
        let project = saved.value.project.clone();
        let worker = store
            .get_document::<MandateRecord>(authority::MANDATES, "m")
            .unwrap()
            .unwrap()
            .value
            .worker;
        let runtime = OutcomeRuntime::from_store(store, project, worker);
        let app = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
        let stopped = stop(&app, &runtime, "m".into()).await.unwrap();
        assert!(stopped.revoked);
        let first = authority::load(&runtime, "m".into()).await.unwrap();
        let again = authority::revoke(&runtime, "m".into()).await.unwrap();
        assert_eq!(again.revision, first.revision);
        assert_eq!(
            again.value.mandate.revoked_at,
            first.value.mandate.revoked_at
        );
        let before = load_link(&runtime, "m".into()).await.unwrap();
        stop(&app, &runtime, "m".into()).await.unwrap();
        assert_eq!(
            load_link(&runtime, "m".into()).await.unwrap().revision,
            before.revision
        );
    }

    #[test]
    fn owner_terminal_and_all_questions_survive_saturation_and_reopen() {
        let (root, store, mut saved) = saturated_store(false);
        let project = saved.value.project.clone();
        drop(store);
        let mut store = OperatorStore::open(root.path(), &project).unwrap();
        let mut progress = saved.value.clone();
        progress.dispatch_attempted = true;
        assert!(persist_link(&mut store, progress, Some(saved.revision)).is_err());
        for n in 0..authority::MAX_QUESTION_DELIVERIES {
            if let Some(previous) = saved.value.question_delivery.take() {
                saved.value.question_history.push(previous);
            }
            saved.value.question_delivery = Some(QuestionDelivery {
                tool_call_id: format!("q{n}"),
                asked_at_ms: n as u64,
                answers: vec![],
                state: "dispatch_outcome_unknown".into(),
                resolution_id: None,
            });
            saved = persist_link(&mut store, saved.value, Some(saved.revision)).unwrap();
            saved.value.question_delivery.as_mut().unwrap().state = "enqueued".into();
            saved = persist_link(&mut store, saved.value, Some(saved.revision)).unwrap();
            let duplicate =
                persist_link(&mut store, saved.value.clone(), Some(saved.revision)).unwrap();
            assert_eq!(duplicate.revision, saved.revision);
            saved.value.question_delivery.as_mut().unwrap().state = "resolved".into();
            saved = persist_link(&mut store, saved.value, Some(saved.revision)).unwrap();
        }
        saved.value.worker_agent_id = Some("worker".into());
        saved = persist_link(&mut store, saved.value, Some(saved.revision)).unwrap();
        saved.value.delivery_proof = Some(GoalSnapshot::default());
        saved = persist_link(&mut store, saved.value, Some(saved.revision)).unwrap();
        saved.value.delivery_run_id = Some("run".into());
        saved = persist_link(&mut store, saved.value, Some(saved.revision)).unwrap();
        saved.value.revoked = true;
        saved = persist_link(&mut store, saved.value, Some(saved.revision)).unwrap();
        authority::ensure_terminal_reservation(&mut store, "m").unwrap();
        let repeated = persist_link(&mut store, saved.value.clone(), Some(saved.revision)).unwrap();
        assert_eq!(repeated.revision, saved.revision);
        let mut extra = saved.value.clone();
        extra.escalation = Some("must not replenish".into());
        assert!(store
            .put_reserved_document("supervision", "m", LINKS, "m", &extra, Some(saved.revision))
            .is_err());
    }

    #[test]
    fn legacy_owner_stop_is_grandfathered_once_after_saturation() {
        let (root, mut store, mut saved) = saturated_store(true);
        saved.value.revoked = true;
        saved = persist_link(&mut store, saved.value, Some(saved.revision)).unwrap();
        let project = saved.value.project.clone();
        drop(store);
        let mut store = OperatorStore::open(root.path(), &project).unwrap();
        let repeated = persist_link(&mut store, saved.value.clone(), Some(saved.revision)).unwrap();
        assert_eq!(repeated.revision, saved.revision);
        authority::ensure_terminal_reservation(&mut store, "m").unwrap();
        assert!(authority::ensure_terminal_reservation(&mut store, "new-mandate").is_err());
    }

    #[tokio::test]
    async fn admitted_final_report_is_charged_at_saturation_and_survives_stop_reopen() {
        let (root, mut store, mut saved) = saturated_store(false);
        store.require_supervision_completion("m").unwrap();
        saved.value.dispatch_attempted = true;
        saved.value.worker_agent_id = Some("native-worker".into());
        saved.value.child_chat_id = Some("native-child".into());
        saved.value.delivery_proof = Some(GoalSnapshot {
            content: saved.value.intake_content.clone(),
            criteria: saved.value.intake_criteria.clone(),
            version: 1,
            status: GoalStatus::Completed,
            attempts: vec![refact_chat_api::GoalAttempt {
                at_ms: 1_700_000_000_000,
                verdict: "met".into(),
                verifier_reply: "Native verifier checked the explicit analysis criterion".into(),
                criteria_verdicts: vec![refact_chat_api::CriterionVerdict {
                    id: "C1".into(),
                    met: true,
                    note: String::new(),
                    source: refact_chat_api::VerdictSource::Judged,
                    evidence: None,
                }],
                ..Default::default()
            }],
            ..Default::default()
        });
        saved = persist_link(&mut store, saved.value, Some(saved.revision)).unwrap();
        let project = saved.value.project.clone();
        let worker = store
            .get_document::<MandateRecord>(authority::MANDATES, "m")
            .unwrap()
            .unwrap()
            .value
            .worker;
        let before = store.document_policy_charge().unwrap();
        let runtime = OutcomeRuntime::from_store(store, project.clone(), worker.clone());
        let mut stale = saved.value.clone();
        stale.child_goal_version = Some(2);
        assert!(publish_delivery(&runtime, &stale).await.is_err());
        let run = publish_delivery(&runtime, &saved.value).await.unwrap();
        assert_eq!(
            runtime
                .with_store(|store| store.document_policy_charge())
                .await
                .unwrap(),
            before
        );
        let outcome = runtime.get_run_outcome(run.clone()).await.unwrap().unwrap();
        assert_eq!(
            outcome.value.status,
            refact_buddy_core::operator_types::RunStatus::Completed
        );
        let item = runtime
            .get_work_item(outcome.value.work_id.clone())
            .await
            .unwrap()
            .unwrap();
        let artifact = runtime
            .get_artifact_revision(item.value.current_artifact_id.unwrap(), None)
            .await
            .unwrap()
            .unwrap();
        assert!(artifact.value.body.contains("Native verifier checked"));
        assert_eq!(publish_delivery(&runtime, &saved.value).await.unwrap(), run);
        let app = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
        // No real agent exists in this scratch fixture; durable revoke remains tested
        // independently of a native worker cancellation acknowledgement.
        let _ = stop(&app, &runtime, "m".into()).await;
        runtime.shutdown().await;
        let reopened = OutcomeRuntime::open(root.path(), project, worker).unwrap();
        let retained = load_link(&reopened, "m".into()).await.unwrap();
        assert_eq!(
            publish_delivery(&reopened, &retained.value).await.unwrap(),
            run
        );
        assert_eq!(
            reopened.get_run_outcome(run).await.unwrap().unwrap(),
            outcome
        );
        assert!(reopened
            .with_store(|store| store.require_supervision_completion("m"))
            .await
            .is_err());
        reopened.shutdown().await;
    }

    #[test]
    fn worker_completion_without_bound_verifier_never_delivers() {
        let link = link();
        let mut goal = GoalSnapshot {
            content: link.intake_content.clone(),
            criteria: link.intake_criteria.clone(),
            version: 1,
            status: GoalStatus::Completed,
            ..Default::default()
        };
        assert!(!verified_goal(&link, &goal));
        goal.attempts.push(refact_chat_api::GoalAttempt {
            verdict: "met".into(),
            verifier_reply: "verified".into(),
            criteria_verdicts: vec![refact_chat_api::CriterionVerdict {
                id: "C1".into(),
                met: true,
                note: String::new(),
                source: refact_chat_api::VerdictSource::Judged,
                evidence: None,
            }],
            ..Default::default()
        });
        assert!(verified_goal(&link, &goal));
        goal.version = 2;
        assert!(!verified_goal(&link, &goal));
    }

    #[tokio::test]
    async fn engineering_proof_retained_before_stop_reopen_delivers_once_at_saturation() {
        use super::super::supervision_proof as proof_owner;
        use refact_buddy_core::operator_types::{AuthorityClass, Mandate, WorkerIdentity};
        let fx = proof_owner::tests::fixture(&["src/a.rs"]).await;
        std::fs::write(fx.root.join("src/a.rs"), "fn edited() {}\n").unwrap();
        let project = ProjectIdentity {
            project_id: "p".into(),
            canonical_root: fx.source.to_string_lossy().into_owned(),
        };
        let worker = WorkerIdentity {
            worker_id: "w".into(),
            engine_instance_id: "e".into(),
            epoch: 1,
        };
        let mut value = link();
        value.project = project.clone();
        value.mandate_id = fx.mandate_id.clone();
        value.chat_id = format!("parent-of-{}", fx.chat_id);
        value.child_chat_id = Some(fx.chat_id.clone());
        value.worker_agent_id = Some("native-engineering-worker".into());
        value.dispatch_attempted = true;
        let facts = proof_owner::SessionFacts {
            chat_id: fx.chat_id.clone(),
            goal_version: 1,
            goal_content: value.intake_content.clone(),
            goal_criteria: value.intake_criteria.clone(),
        };
        let capture = proof_owner::capture_before_stream(fx.cache_dir.clone(), &facts)
            .await
            .unwrap()
            .unwrap();
        let reply = "GOAL: MET\nCRITERION C1: MET — isolated source observed";
        proof_owner::bind_after_stream(capture, reply)
            .await
            .unwrap();
        let at_ms = Utc::now().timestamp_millis() as u64;
        assert!(proof_owner::finalize_met(
            &fx.chat_id,
            1,
            &facts.goal_content,
            &facts.goal_criteria,
            reply,
            at_ms
        ));
        let proof = proof_owner::get_authorized_proof(&fx.chat_id).unwrap();
        value.delivery_proof = Some(GoalSnapshot {
            content: value.intake_content.clone(),
            criteria: value.intake_criteria.clone(),
            version: 1,
            status: GoalStatus::Completed,
            attempts: vec![refact_chat_api::GoalAttempt {
                at_ms,
                verdict: "met".into(),
                verifier_reply: reply.into(),
                criteria_verdicts: vec![refact_chat_api::CriterionVerdict {
                    id: "C1".into(),
                    met: true,
                    note: "source observed".into(),
                    source: refact_chat_api::VerdictSource::Judged,
                    evidence: None,
                }],
                ..Default::default()
            }],
            ..Default::default()
        });
        value.engineering_proof = Some(proof.clone());
        let owner = NativeSourceIdentity {
            source_kind: NativeSourceKind::External,
            source_id: "scratch-user".into(),
            source_version: "1".into(),
        };
        let record = MandateRecord {
            schema_version: 1,
            project: project.clone(),
            worker: worker.clone(),
            granted_by: owner.clone(),
            max_workers: 1,
            routine_choices: vec![],
            steering_notes: vec![],
            worker_tools: vec!["update_textdoc".into()],
            worker_target_files: fx.targets.clone(),
            mandate: Mandate {
                schema_version: 1,
                mandate_id: fx.mandate_id.clone(),
                project: project.clone(),
                owner,
                owner_epoch: 1,
                resource_scopes: vec![],
                allowed_operations: vec![
                    "read_project".into(),
                    "engineer_project".into(),
                    "spawn_worker".into(),
                ],
                allowed_authority_classes: vec![AuthorityClass::ReadOnly, AuthorityClass::Mutating],
                model_constraints: vec!["test-model".into()],
                budget_limit: None,
                expires_at: Some((Utc::now() + chrono::Duration::hours(1)).to_rfc3339()),
                revoked_at: None,
            },
        };
        proof_owner::validate_retained(
            &proof,
            &record,
            &fx.chat_id,
            value.delivery_proof.as_ref().unwrap(),
        )
        .unwrap();
        let mut store = OperatorStore::open(&fx.source, &project).unwrap();
        store
            .reserve_native_documents(
                "supervision",
                &fx.mandate_id,
                &authority::reservation_slots(&fx.mandate_id),
            )
            .unwrap();
        store
            .put_native_ordinary_document(
                "supervision",
                &fx.mandate_id,
                authority::MANDATES,
                &fx.mandate_id,
                &record,
                None,
            )
            .unwrap();
        persist_link(&mut store, value, None).unwrap();
        store
            .set_document_growth_policy(Some(
                super::super::operator_observability::DocumentGrowthPolicy {
                    max_rows_per_namespace: 2,
                    replace_editorial_aggregate: false,
                },
            ))
            .unwrap();
        // Install the new-work admission policy before filling document
        // capacity: the policy record itself remains charged and must not be
        // granted a post-saturation write exception.
        store
            .set_capacity_policy(Some(super::super::operator_observability::CapacityPolicy {
                max_rows_per_table: 1,
                max_database_bytes: 1,
            }))
            .unwrap();
        for n in 0..10_000 {
            if store
                .put_document(&format!("fill_{n}"), "row", &record, None)
                .is_err()
            {
                break;
            }
        }
        assert!(store.put_document("full", "row", &record, None).is_err());
        let charge = store.document_policy_charge().unwrap();
        let runtime = OutcomeRuntime::from_store(store, project.clone(), worker.clone());
        proof_owner::acknowledge_authorized(&fx.chat_id);
        // Stop is the actual terminal control path: it first invalidates the
        // live native scope, durably revokes the mandate, and retains the
        // already-admitted historical proof under saturation.
        let app = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
        let stopped = stop(&app, &runtime, fx.mandate_id.clone()).await.unwrap();
        assert!(stopped.revoked);
        runtime.shutdown().await;
        assert!(proof_owner::get_authorized_proof(&fx.chat_id).is_none());
        assert!(authority::native_edit_scope_for_chat(&fx.chat_id).is_none());
        let runtime = OutcomeRuntime::open(
            &fx.source,
            project,
            WorkerIdentity {
                epoch: 2,
                ..worker
            },
        )
        .unwrap();
        // The replacement epoch cannot load current authority or edit scope,
        // but reconciliation owns the retained one-way publication.
        assert!(load_link(&runtime, fx.mandate_id.clone()).await.is_err());
        assert!(authority::native_edit_scope_for_chat(&fx.chat_id).is_none());
        let reconciled = reconcile(&app, &runtime, fx.mandate_id.clone())
            .await
            .unwrap();
        let run_id = reconciled.delivery_run_id.clone().unwrap();
        let observed = status(&app, &runtime, fx.mandate_id.clone()).await.unwrap();
        assert!(observed.allowed_operations.is_empty());
        assert!(observed.steering_notes.is_empty());
        assert!(observed.native_worker.is_none());
        assert!(observed.native_pause.is_none());
        assert!(observed.coverage.contains("historical"));
        assert!(authority::load(&runtime, fx.mandate_id.clone())
            .await
            .is_err());
        assert!(resume_worker(
            &app,
            &runtime,
            fx.mandate_id.clone(),
            record.granted_by.clone(),
            ControlRequest {
                control_id: "historical-resume".into()
            },
        )
        .await
        .is_err());
        let sequence = runtime
            .with_store(|store| Ok(store.retention_preview()?.sequence_high_watermark))
            .await
            .unwrap();
        let repeated = reconcile(&app, &runtime, fx.mandate_id.clone())
            .await
            .unwrap();
        assert_eq!(repeated.delivery_run_id.as_ref(), Some(&run_id));
        assert_eq!(
            runtime
                .with_store(|store| Ok(store.retention_preview()?.sequence_high_watermark))
                .await
                .unwrap(),
            sequence
        );
        assert!(authority::native_edit_scope_for_chat(&fx.chat_id).is_none());
        let retained = load_retained_completion(&runtime, fx.mandate_id.clone())
            .await
            .unwrap()
            .0;
        let mut forged = retained.value.clone();
        forged
            .engineering_proof
            .as_mut()
            .unwrap()
            .source
            .content_digest = "0".repeat(64);
        assert!(publish_delivery(&runtime, &forged).await.is_err());
        assert_eq!(
            publish_delivery(&runtime, &retained.value).await.unwrap(),
            run_id
        );
        let outcome = runtime.get_run_outcome(run_id).await.unwrap().unwrap();
        assert_eq!(outcome.value.worker.epoch, 1);
        assert_eq!(outcome.value.evidence.len(), 2);
        assert_eq!(
            runtime
                .with_store(|store| store.document_policy_charge())
                .await
                .unwrap(),
            charge
        );
        let item = runtime
            .get_work_item(outcome.value.work_id)
            .await
            .unwrap()
            .unwrap();
        let artifact = runtime
            .get_artifact_revision(item.value.current_artifact_id.unwrap(), None)
            .await
            .unwrap()
            .unwrap();
        assert!(artifact
            .value
            .body
            .contains("retained historical verification"));
        assert!(artifact.value.body.contains(&proof.source.content_digest));
        assert!(!artifact
            .value
            .body
            .contains("bounded read-only analysis only"));
        runtime.shutdown().await;
    }

    #[test]
    fn question_resolution_requires_exact_native_generation_and_answer() {
        let answers = vec![refact_chat_api::QuestionAnswer {
            id: "q".into(),
            answer: serde_json::json!("a"),
        }];
        let mut delivery = QuestionDelivery {
            tool_call_id: "t".into(),
            asked_at_ms: 4,
            answers: answers.clone(),
            state: "enqueued".into(),
            resolution_id: None,
        };
        let mut set = refact_chat_api::PendingQuestionSet {
            tool_call_id: "t".into(),
            asked_at_ms: 5,
            questions: vec![],
            answers: answers.clone(),
            status: refact_chat_api::PendingQuestionStatus::Answered,
            resolution_id: Some("r".into()),
            response_protocol_version: refact_chat_api::QUESTION_RESPONSE_PROTOCOL_VERSION,
            response: None,
        };
        observe_question(&mut delivery, &[set.clone()]);
        assert_eq!(delivery.state, "source_unavailable");
        set.asked_at_ms = 4;
        observe_question(&mut delivery, &[set.clone()]);
        assert_eq!(delivery.state, "resolved");
        set.answers[0].answer = serde_json::json!("b");
        observe_question(&mut delivery, &[set.clone()]);
        assert_eq!(delivery.state, "resolved");
        set.status = refact_chat_api::PendingQuestionStatus::Pending;
        observe_question(&mut delivery, &[set]);
        assert_eq!(delivery.state, "resolved");
    }
}
