use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use refact_chat_api::{ChatEvent, PythonApprovalDecisionKind as Decision, PythonApprovalRequest};
use tokio::sync::{oneshot, Mutex as AsyncMutex};
use tokio_util::sync::CancellationToken;

use crate::app_state::AppState;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::ChatToolCall;

#[derive(Clone, Debug)]
pub struct ApprovalAsk {
    pub chat_id: String,
    pub tool_call_id: String,
    pub run_id: String,
    pub call_id: String,
    pub tool_name: String,
    pub args_preview: String,
    pub command: Option<String>,
    pub rule: String,
    pub editing_tool: bool,
    /// Engine change preview; such a request always needs a person.
    pub engine: Option<refact_chat_api::PythonEngineRequest>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApprovalOutcome {
    Allowed { for_chat: bool },
    AllowedForRun,
    Denied,
    Stopped,
    Cancelled,
}

pub(crate) struct Pending {
    chat_id: String,
    request: PythonApprovalRequest,
    pub(crate) full_args: String,
    sender: oneshot::Sender<ApprovalOutcome>,
}

#[derive(Default)]
pub struct ApprovalBroker {
    pub(crate) pending: Mutex<HashMap<String, Pending>>,
    decisions: AsyncMutex<()>,
    child_runs: Mutex<HashMap<(String, String), CancellationToken>>,
    pub(crate) run_permits: Arc<Mutex<HashSet<(String, String)>>>,
}

pub(crate) struct Run {
    id: uuid::Uuid,
    run_id: String,
    token: CancellationToken,
    deadline: Option<tokio::time::Instant>,
}

#[derive(Default)]
pub struct PythonToolRuntime {
    pub registry: super::registry::KernelRegistry,
    pub approvals: ApprovalBroker,
    pub tickets: Mutex<HashMap<(String, String), super::registry::TurnTicket>>,
    pub detached: Mutex<HashMap<(String, String), String>>,
    pub nested_parents: Mutex<HashMap<(String, String), String>>,
    pub(crate) runs: Arc<Mutex<HashMap<(String, String), Run>>>,
}

pub struct RunGuard {
    id: uuid::Uuid,
    run_id: String,
    permits: Arc<Mutex<HashSet<(String, String)>>>,
    runs: Arc<Mutex<HashMap<(String, String), Run>>>,
    key: (String, String),
    token: CancellationToken,
}

impl RunGuard {
    pub fn cancellation_token(&self) -> CancellationToken {
        self.token.clone()
    }
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        self.token.cancel();
        self.permits
            .lock()
            .unwrap()
            .remove(&(self.key.0.clone(), self.run_id.clone()));
        let mut runs = self.runs.lock().unwrap();
        if runs.get(&self.key).is_some_and(|run| run.id == self.id) {
            runs.remove(&self.key);
        }
    }
}

/// A run permit never stands in for a person, e.g. for protected configuration edits.
pub(crate) fn run_permit_covers(request: &PythonApprovalRequest) -> bool {
    request.editing_tool && !needs_person(request)
}

/// Rule of every engine change request.
pub const ENGINE_CHANGE_APPROVAL: &str = "engine change: always asks the user";

/// Only a person may answer: protected configuration edits, automation approvals and every
/// engine change (allow once, deny or stop; no run or chat permit).
pub(crate) fn needs_person(request: &PythonApprovalRequest) -> bool {
    crate::chat::tools::requires_actual_human_approval(&request.rule)
        || request.engine.is_some()
        || request.rule == ENGINE_CHANGE_APPROVAL
}

impl PythonToolRuntime {
    pub fn register_run(&self, chat_id: &str, tool_call_id: &str, run_id: &str) -> RunGuard {
        let key = (chat_id.to_string(), tool_call_id.to_string());
        let token = CancellationToken::new();
        let id = uuid::Uuid::new_v4();
        if let Some(previous) = self.runs.lock().unwrap().insert(
            key.clone(),
            Run {
                id,
                run_id: run_id.into(),
                token: token.clone(),
                deadline: None,
            },
        ) {
            previous.token.cancel();
        }
        RunGuard {
            id,
            run_id: run_id.into(),
            permits: self.approvals.run_permits.clone(),
            runs: self.runs.clone(),
            key,
            token,
        }
    }

    pub fn set_run_deadline(
        &self,
        chat_id: &str,
        tool_call_id: &str,
        deadline: tokio::time::Instant,
    ) {
        if let Some(run) = self
            .runs
            .lock()
            .unwrap()
            .get_mut(&(chat_id.to_string(), tool_call_id.to_string()))
        {
            run.deadline = Some(deadline);
        }
    }
    pub fn run_deadline(&self, chat_id: &str, tool_call_id: &str) -> Option<tokio::time::Instant> {
        self.runs
            .lock()
            .unwrap()
            .get(&(chat_id.to_string(), tool_call_id.to_string()))
            .and_then(|run| run.deadline)
    }

    pub fn stop_run(&self, chat_id: &str, tool_call_id: &str) -> Result<(), String> {
        let runs = self.runs.lock().unwrap();
        let run = runs
            .get(&(chat_id.into(), tool_call_id.into()))
            .ok_or("Unknown python run")?;
        run.token.cancel();
        self.approvals.cancel_run_requests(chat_id, &run.run_id);
        Ok(())
    }

    fn token(&self, ask: &ApprovalAsk) -> Option<CancellationToken> {
        self.runs
            .lock()
            .unwrap()
            .get(&(ask.chat_id.clone(), ask.tool_call_id.clone()))
            .filter(|run| run.run_id == ask.run_id)
            .map(|run| run.token.clone())
    }
    pub fn active_run_ids(&self, chat_id: &str) -> Vec<String> {
        self.runs
            .lock()
            .unwrap()
            .iter()
            .filter(|((chat, _), _)| chat == chat_id)
            .map(|(_, run)| run.run_id.clone())
            .collect()
    }
}

struct RequestGuard {
    app: AppState,
    chat_id: String,
    request_id: String,
}

impl Drop for RequestGuard {
    fn drop(&mut self) {
        self.app
            .python_tool
            .approvals
            .pending
            .lock()
            .unwrap()
            .remove(&self.request_id);
        let app = self.app.clone();
        let chat_id = self.chat_id.clone();
        let request_id = self.request_id.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                remove_request(&app, &chat_id, &request_id, Decision::Stop).await;
            });
        }
    }
}

async fn remove_request(app: &AppState, chat_id: &str, request_id: &str, decision: Decision) {
    if let Some(session) = app.chat.sessions.read().await.get(chat_id).cloned() {
        let mut session = session.lock().await;
        let before = session.runtime.python_approvals.len();
        session
            .runtime
            .python_approvals
            .retain(|request| request.request_id != request_id);
        if before != session.runtime.python_approvals.len() {
            session.emit(ChatEvent::PythonApprovalResolved {
                request_id: request_id.into(),
                decision,
            });
        }
    }
}

impl ApprovalBroker {
    pub async fn request(&self, app: &AppState, ask: ApprovalAsk) -> ApprovalOutcome {
        let Some(session) = app.chat.sessions.read().await.get(&ask.chat_id).cloned() else {
            return ApprovalOutcome::Cancelled;
        };
        let thread = session.lock().await.thread.clone();
        let child = app
            .agents
            .find_agent_id_by_child_chat_id(&ask.chat_id)
            .await
            .is_some()
            || thread
                .task_meta
                .as_ref()
                .is_some_and(|meta| matches!(meta.role.as_str(), "agent" | "agents"))
            || matches!(
                thread.link_type.as_deref(),
                Some("subagent" | "delegate" | "task_agent")
            );
        if child {
            return ApprovalOutcome::Denied;
        }
        let Some(token) = app.python_tool.token(&ask) else {
            return ApprovalOutcome::Cancelled;
        };
        let full_args = ask.args_preview.clone();
        let request_tool_name = ask.tool_name.clone();
        let request = PythonApprovalRequest {
            request_id: uuid::Uuid::new_v4().to_string(),
            tool_call_id: ask.tool_call_id,
            run_id: ask.run_id,
            call_id: ask.call_id,
            tool_name: ask.tool_name,
            args_preview: args_preview(&ask.args_preview, 2000, 400),
            command: ask
                .command
                .filter(|command| !command.is_empty() && command != &request_tool_name)
                .map(|command| bounded_preview(&command, 16000)),
            rule: ask.rule,
            editing_tool: ask.editing_tool,
            created_at_ms: crate::agents::approval::now_ms(),
            engine: ask.engine,
            needs_person: false,
        };
        let request = PythonApprovalRequest {
            needs_person: needs_person(&request),
            ..request
        };
        let (sender, receiver) = oneshot::channel();
        let guard = RequestGuard {
            app: app.clone(),
            chat_id: ask.chat_id.clone(),
            request_id: request.request_id.clone(),
        };
        {
            let mut session = session.lock().await;
            if session.closed || token.is_cancelled() {
                return ApprovalOutcome::Cancelled;
            }
            let mut pending = self.pending.lock().unwrap();
            if run_permit_covers(&request) && self.run_permitted(&ask.chat_id, &request.run_id) {
                return ApprovalOutcome::AllowedForRun;
            }
            pending.insert(
                request.request_id.clone(),
                Pending {
                    chat_id: ask.chat_id.clone(),
                    request: request.clone(),
                    full_args,
                    sender,
                },
            );
            drop(pending);
            session.runtime.python_approvals.push(request.clone());
            session.emit(ChatEvent::PythonApprovalRequested {
                request,
            });
        }
        let outcome = tokio::select! {
            biased;
            result = receiver => result.unwrap_or(ApprovalOutcome::Cancelled),
            _ = token.cancelled() => ApprovalOutcome::Cancelled,
        };
        remove_request(app, &ask.chat_id, &guard.request_id, Decision::Stop).await;
        drop(guard);
        outcome
    }

    pub async fn request_with_context(
        &self,
        app: &AppState,
        ask: ApprovalAsk,
        context: &Arc<AsyncMutex<AtCommandsContext>>,
        call: &ChatToolCall,
    ) -> ApprovalOutcome {
        let (chat_id, child) = {
            let context = context.lock().await;
            (
                context.chat_id.clone(),
                context.background_agent_id.is_some()
                    || context
                        .task_meta
                        .as_ref()
                        .is_some_and(|meta| matches!(meta.role.as_str(), "agent" | "agents")),
            )
        };
        if chat_id != ask.chat_id
            || call.id != ask.call_id
            || call.function.name != ask.tool_name
            || call.function.arguments != ask.args_preview
        {
            return ApprovalOutcome::Denied;
        }
        if child {
            self.request_child(app, ask, context, call).await
        } else {
            self.request(app, ask).await
        }
    }

    async fn request_child(
        &self,
        app: &AppState,
        ask: ApprovalAsk,
        context: &Arc<AsyncMutex<AtCommandsContext>>,
        call: &ChatToolCall,
    ) -> ApprovalOutcome {
        let Some(token) = app.python_tool.token(&ask) else {
            return ApprovalOutcome::Cancelled;
        };
        let key = (ask.chat_id.clone(), ask.run_id.clone());
        self.child_runs
            .lock()
            .unwrap()
            .insert(key.clone(), token.clone());
        let outcome = tokio::select! {
            result = crate::agents::approval::request_with_parking(app, context, call, &ask.rule, false) => match result {
                Ok(true) => ApprovalOutcome::Allowed { for_chat: false },
                Err(error) if error == "approval cancelled" => ApprovalOutcome::Cancelled,
                _ => ApprovalOutcome::Denied,
            },
            _ = token.cancelled() => ApprovalOutcome::Cancelled,
        };
        self.child_runs.lock().unwrap().remove(&key);
        outcome
    }

    pub async fn decide(
        &self,
        app: &AppState,
        chat_id: &str,
        request_id: &str,
        decision: Decision,
    ) -> Result<(), String> {
        let _decision_guard = self.decisions.lock().await;
        let (request, full_args) = self
            .pending
            .lock()
            .unwrap()
            .get(request_id)
            .filter(|pending| pending.chat_id == chat_id)
            .map(|pending| (pending.request.clone(), pending.full_args.clone()))
            .ok_or("Unknown python approval request")?;
        if matches!(decision, Decision::AllowForChat | Decision::AllowForRun)
            && !request.editing_tool
        {
            return Err(format!("{decision:?} requires an editing tool"));
        }
        if matches!(decision, Decision::AllowForChat | Decision::AllowForRun)
            && request.needs_person
        {
            let scope = if decision == Decision::AllowForRun {
                "this run"
            } else {
                "this chat"
            };
            return Err(format!(
                "Allowing for {scope} cannot cover this change: it needs a person's approval each time. Choose Allow once, Deny or Stop."
            ));
        }
        let accepted = matches!(
            decision,
            Decision::AllowOnce | Decision::AllowForChat | Decision::AllowForRun
        );
        let hooked = crate::ext::hooks_runner::apply_permission_decision(
            app.clone(),
            chat_id.into(),
            crate::ext::hooks_runner::get_project_dir_string(app.clone()).await,
            crate::ext::hooks_runner::PermissionDecisionRequest {
                tool_name: request.tool_name.clone(),
                tool_input: serde_json::from_str(&full_args)
                    .unwrap_or_else(|_| serde_json::json!({})),
                accepted,
                source: "user_confirmation".into(),
                request_id: Some(request_id.into()),
                tool_call_id: request.call_id.clone(),
                reason_type: "confirmation".into(),
            },
        )
        .await;
        let decision = if accepted && !hooked.accepted {
            Decision::Deny
        } else {
            decision
        };
        let Some(session) = app.chat.sessions.read().await.get(chat_id).cloned() else {
            return Err("Chat session unavailable".into());
        };
        let (pending, released) = {
            let mut all = self.pending.lock().unwrap();
            let pending = all
                .remove(request_id)
                .ok_or("Python approval request no longer pending")?;
            let mut released = Vec::new();
            let run_alive = app
                .python_tool
                .runs
                .lock()
                .unwrap()
                .get(&(chat_id.to_string(), request.tool_call_id.clone()))
                .is_some_and(|run| run.run_id == request.run_id && !run.token.is_cancelled());
            if decision == Decision::AllowForRun && run_alive {
                self.run_permits
                    .lock()
                    .unwrap()
                    .insert((chat_id.to_string(), request.run_id.clone()));
                let ids: Vec<String> = all
                    .iter()
                    .filter(|(_, item)| {
                        item.chat_id == chat_id
                            && item.request.run_id == request.run_id
                            && run_permit_covers(&item.request)
                    })
                    .map(|(id, _)| id.clone())
                    .collect();
                for id in ids {
                    if let Some(item) = all.remove(&id) {
                        released.push((id, item.sender));
                    }
                }
            }
            (pending, released)
        };
        if decision == Decision::AllowForChat {
            let mut session = session.lock().await;
            let (changed, params) = crate::chat::queue::apply_setparams_patch(
                &mut session.thread,
                &serde_json::json!({"auto_approve_editing_tools":true}),
            );
            session.emit(ChatEvent::ThreadUpdated {
                params,
            });
            if changed {
                session.increment_version();
                session.touch();
            }
            drop(session);
            crate::chat::trajectories::maybe_save_trajectory_with_intent(
                app.clone(),
                session_arc(app, chat_id).await?,
                crate::chat::types::TrajectoryCommitIntent::Required,
            )
            .await;
        }
        let outcome = match decision {
            Decision::AllowOnce => ApprovalOutcome::Allowed {
                for_chat: false,
            },
            Decision::AllowForChat => ApprovalOutcome::Allowed {
                for_chat: true,
            },
            Decision::AllowForRun => ApprovalOutcome::AllowedForRun,
            Decision::Deny => ApprovalOutcome::Denied,
            Decision::Stop => ApprovalOutcome::Stopped,
        };
        remove_request(app, chat_id, request_id, decision).await;
        for (id, sender) in released {
            remove_request(app, chat_id, &id, Decision::AllowForRun).await;
            let _ = sender.send(ApprovalOutcome::AllowedForRun);
        }
        if decision == Decision::Stop {
            let runs = app.python_tool.runs.lock().unwrap();
            let run = runs
                .get(&(chat_id.into(), request.tool_call_id.clone()))
                .filter(|run| run.run_id == request.run_id);
            let _ = pending.sender.send(outcome);
            if let Some(run) = run {
                run.token.cancel();
            }
            drop(runs);
            self.cancel_run_requests(chat_id, &request.run_id);
        } else {
            let _ = pending.sender.send(outcome);
        }
        Ok(())
    }

    pub fn cancel_run_requests(&self, chat_id: &str, run_id: &str) {
        if let Some(token) = self
            .child_runs
            .lock()
            .unwrap()
            .remove(&(chat_id.into(), run_id.into()))
        {
            token.cancel();
        }
        let mut pending = self.pending.lock().unwrap();
        // Removed under the pending lock: a concurrent AllowForRun decision holds it too.
        self.run_permits
            .lock()
            .unwrap()
            .remove(&(chat_id.to_string(), run_id.to_string()));
        let ids: Vec<_> = pending
            .iter()
            .filter(|(_, item)| item.chat_id == chat_id && item.request.run_id == run_id)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            if let Some(item) = pending.remove(&id) {
                let _ = item.sender.send(ApprovalOutcome::Cancelled);
            }
        }
    }

    pub fn run_permitted(&self, chat_id: &str, run_id: &str) -> bool {
        self.run_permits
            .lock()
            .unwrap()
            .contains(&(chat_id.to_string(), run_id.to_string()))
    }
}

async fn session_arc(
    app: &AppState,
    chat_id: &str,
) -> Result<Arc<AsyncMutex<crate::chat::types::ChatSession>>, String> {
    app.chat
        .sessions
        .read()
        .await
        .get(chat_id)
        .cloned()
        .ok_or("Chat session unavailable".into())
}

fn bounded_preview(text: &str, cap: usize) -> String {
    if text.chars().count() <= cap {
        return text.to_owned();
    }
    let marker = "… [truncated]";
    text.chars()
        .take(cap.saturating_sub(marker.chars().count()))
        .collect::<String>()
        + marker
}
#[cfg(test)]
mod preview_tests {
    use super::*;
    #[test]
    fn approval_wire_previews_are_character_bounded() {
        for cap in [2000, 16000] {
            let preview = bounded_preview(&"é".repeat(cap + 1), cap);
            assert_eq!(preview.chars().count(), cap);
            assert!(preview.ends_with("[truncated]"));
        }
    }
}

/// Keep previews valid JSON; full arguments remain private to the broker.
pub(crate) fn args_preview(text: &str, cap: usize, value_cap: usize) -> String {
    let Ok(serde_json::Value::Object(args)) = serde_json::from_str(text) else {
        return "{}".into();
    };
    let mut preview = serde_json::Map::new();
    let total = args.len();
    for (i, (key, value)) in args.into_iter().enumerate() {
        let compact = match &value {
            serde_json::Value::String(s) => s.clone(),
            _ => value.to_string(),
        };
        let value = if compact.chars().count() > value_cap {
            serde_json::Value::String(format!(
                "{}… (+{} chars)",
                compact.chars().take(value_cap).collect::<String>(),
                compact.chars().count() - value_cap
            ))
        } else if value.is_array() || value.is_object() {
            serde_json::Value::String(compact)
        } else {
            value
        };
        preview.insert(key.clone(), value);
        let mut candidate = preview.clone();
        if i + 1 < total {
            candidate.insert(
                "…".into(),
                serde_json::json!(format!("+{} more arguments", total - i - 1)),
            );
        }
        if serde_json::to_string(&candidate).unwrap().chars().count() > cap {
            preview.remove(&key);
            preview.insert(
                "…".into(),
                serde_json::json!(format!("+{} more arguments", total - i)),
            );
            break;
        }
    }
    serde_json::to_string(&preview).unwrap()
}

#[cfg(test)]
mod args_preview_tests {
    use super::*;
    use serde_json::{json, Value};
    #[test]
    fn previews_preserve_json_and_bound_multibyte_values() {
        let input = json!({"long":"é".repeat(450),"short":42,"nested":["x".repeat(500)]});
        let p: Value = serde_json::from_str(&args_preview(&input.to_string(), 2000, 400)).unwrap();
        assert_eq!(p["short"], 42);
        assert_eq!(p["long"], format!("{}… (+50 chars)", "é".repeat(400)));
        assert!(p["nested"].as_str().unwrap().contains("… (+"));
    }
    #[test]
    fn previews_drop_remaining_keys_and_keep_call_order() {
        let mut input = serde_json::Map::new();
        for i in 0..20 {
            input.insert(format!("key{i:02}"), json!("é".repeat(500)));
        }
        for (cap, per_value) in [(2000, 400), (1000, 200)] {
            let p = args_preview(&Value::Object(input.clone()).to_string(), cap, per_value);
            assert!(p.chars().count() <= cap);
            let v: Value = serde_json::from_str(&p).unwrap();
            assert!(v["…"].as_str().unwrap().ends_with("more arguments"));
            assert!(v.get("key00").is_some());
            assert!(v.get("key19").is_none());
        }
    }
}
