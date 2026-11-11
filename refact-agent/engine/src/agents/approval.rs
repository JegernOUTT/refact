use std::sync::Arc;
use tokio::sync::Mutex;
use crate::app_state::AppState;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatMessage, ChatToolCall};
use crate::chat::types::ThreadParams;
use refact_work_core::{ApprovalDecision, PendingApproval};

#[path = "approval_user.rs"]
mod user;
pub(crate) use user::{answer_from_user, apply_user_answer, is_user_approval};

#[cfg(test)]
#[path = "approval_routing_tests.rs"]
mod routing_tests;

struct PendingApprovalGuard {
    registry: refact_work_core::ApprovalRegistry,
    questions: refact_chat_api::QuestionRegistry,
    asker: String,
    parent: String,
    generation: u64,
    request_id: String,
    question_id: String,
    execution: Option<Arc<super::execution::RunExecution>>,
    abort: Option<Arc<std::sync::atomic::AtomicBool>>,
}

impl Drop for PendingApprovalGuard {
    fn drop(&mut self) {
        let _ = self
            .registry
            .cancel(&self.parent, self.generation, &self.request_id, now_ms());
        let _ = self
            .questions
            .cancel(&self.question_id, &self.asker, chrono::Utc::now());
        if let Some(execution) = &self.execution {
            execution.cancel();
        }
        if let Some(abort) = &self.abort {
            abort.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

/// Mirror an approval decision onto its tracked `PermissionEscalation`
/// question. The decision itself still belongs to the approval registry; this
/// only settles the question that tracks it, exactly once.
fn settle_escalation_question(
    app: &AppState,
    request_id: &str,
    asker: &str,
    answerer: &str,
    decision: ApprovalDecision,
) {
    let now = chrono::Utc::now();
    let questions = app.questions();
    let _ = match decision {
        ApprovalDecision::Approved => questions.answer(request_id, answerer, "approve".into(), now),
        ApprovalDecision::Rejected => questions.reject(request_id, answerer, now),
        ApprovalDecision::Cancelled => questions.cancel(request_id, asker, now),
        ApprovalDecision::Expired => {
            questions.expire(now);
            return;
        }
    };
}

struct ApprovalNoticeAudit<'a>(&'a ChatMessage);

impl refact_privacy::PrivacyAudited for ApprovalNoticeAudit<'_> {
    fn privacy_records(
        &self,
    ) -> Result<Vec<(usize, refact_privacy::FileRecord)>, refact_privacy::PrivacyAuditError> {
        refact_privacy::records_from_messages(std::slice::from_ref(self.0))
    }
}

pub fn now_ms() -> u64 {
    chrono::Utc::now().timestamp_millis().max(0) as u64
}

pub(crate) async fn dangerous_confirmation_enabled(app: &AppState, chat_id: &str) -> bool {
    let mut current = chat_id.to_string();
    let mut visited = std::collections::HashSet::new();
    let max_depth = crate::runtime_settings::current()
        .coordination
        .max_subchat_depth;
    while visited.len() < max_depth && visited.insert(current.clone()) {
        let Some(session) = app.chat.sessions.read().await.get(&current).cloned() else {
            return false;
        };
        let thread = {
            let session = session.lock().await;
            if session.closed || !session.thread.auto_approve_dangerous_commands {
                return false;
            }
            session.thread.clone()
        };
        if let Some(agent_id) = app.agents.find_agent_id_by_child_chat_id(&current).await {
            let Ok(target) = super::approval_target::ApprovalTarget::resolve(app, &agent_id).await
            else {
                return false;
            };
            let Some(config) = app.agent_executions.saved_config(&agent_id) else {
                return false;
            };
            if app.agent_executions.get(&current).is_none_or(|run| {
                run.is_cancelled() || run.state() == super::execution::ExecutionState::Gone
            }) || app
                .agents
                .abort_flag(&agent_id)
                .await
                .is_none_or(|abort| abort.load(std::sync::atomic::Ordering::Relaxed))
                || !config.stateful
                || !config.auto_approve_dangerous_commands
                || matches!(
                    config.tools,
                    crate::subchat::ToolsPolicy::All
                        | crate::subchat::ToolsPolicy::Supervised { .. }
                )
                || config.chat_id.as_deref() != Some(current.as_str())
                || config.parent_id.as_deref() != Some(target.parent_id.as_str())
                || thread.parent_id != config.parent_id
                || thread.root_chat_id != config.root_chat_id
            {
                return false;
            }
            current = target.parent_id;
        } else if thread
            .task_meta
            .as_ref()
            .is_some_and(|meta| matches!(meta.role.as_str(), "agent" | "agents"))
        {
            let Ok(target) =
                super::approval_target::ApprovalTarget::resolve(app, &format!("board:{current}"))
                    .await
            else {
                return false;
            };
            if thread.parent_id.as_deref() != Some(target.parent_id.as_str()) {
                return false;
            }
            current = target.parent_id;
        } else {
            return !matches!(
                thread.link_type.as_deref(),
                Some("subagent" | "delegate" | "task_agent")
            );
        }
    }
    false
}

pub async fn request(
    app: &AppState,
    ccx: &Arc<Mutex<AtCommandsContext>>,
    call: &ChatToolCall,
    reason: &str,
) -> Result<bool, String> {
    let target = super::approval_target::ApprovalTarget::from_context(app, ccx).await?;
    let (abort, child_worktree) = {
        let context = ccx.lock().await;
        (
            context.abort_flag.clone(),
            context.execution_scope_worktree(),
        )
    };
    let agent_id = target.work_id.clone();
    let child_chat_id = target.chat_id.clone();
    let parent_session = app
        .chat
        .sessions
        .read()
        .await
        .get(&target.parent_id)
        .cloned()
        .ok_or("parent session unavailable for approval")?;
    let (parent_thread, escalate) = {
        let parent = parent_session.lock().await;
        (
            parent.thread.clone(),
            crate::chat::tools::requires_actual_human_approval(reason)
                || !user::parent_can_answer(&parent),
        )
    };
    let request_id = uuid::Uuid::new_v4().to_string();
    let args = serde_json::from_str(&call.function.arguments)
        .map_err(|e| format!("invalid tool arguments: {e}"))?;
    let created = now_ms();
    let execution = super::command_approval::prepare(ccx, call, reason).await?;
    if ccx.lock().await.approved_executions.contains_key(&call.id) {
        return Err("command approval already exists for this tool call".into());
    }
    let pending = PendingApproval {
        id: request_id.clone(),
        child_work_id: agent_id.clone(),
        parent_chat_id: target.parent_id.clone(),
        run_generation: target.generation,
        tool_call_id: call.id.clone(),
        tool_name: call.function.name.clone(),
        args,
        requires_human: crate::chat::tools::requires_actual_human_approval(reason),
        digest: child_worktree
            .as_ref()
            .map(|worktree| worktree.root.to_string_lossy().into_owned()),
        created_at: created,
        expires_at: created.saturating_add(
            refact_work_core::coordination_settings()
                .approval_timeout_secs
                .saturating_mul(1000),
        ),
        execution,
    };
    let instructions = if pending.requires_human {
        "This operation requires exact human approval. Parent model policy or automatic approval cannot authorize it. Waiting for the user; nothing has been approved.".to_string()
    } else if escalate {
        "Escalated to the user because the parent session cannot answer: agent_approval is absent from its effective tool list. Waiting for user approval; nothing has been approved.".to_string()
    } else {
        format!("Use agent_approval with request_id {request_id} and action approve or reject.")
    };
    let mut notice = crate::chat::internal_roles::event(
        crate::chat::internal_roles::EventSubkind::SystemNotice, crate::agents::push::NOTICE_APPROVAL,
        serde_json::json!({"agent_id":agent_id,"request_id":request_id,"tool_call_id":call.id}),
        format!("Child {} needs command approval (request {}): {} {}\nExecution binding: {}\nReason: {}\n{}", target.title, request_id, pending.tool_name, pending.args, serde_json::to_string(&pending.execution).map_err(|_| "approval binding serialization failed")?, reason, instructions),
    );
    {
        let context = ccx.lock().await;
        crate::privacy::records::carry_records_into(&mut notice, &context.messages)
            .map_err(|e| e.to_string())?;
    }
    if let Some(execution) = &pending.execution {
        let records = crate::privacy::records::declared_file_records(
            &app.gcx,
            [std::path::PathBuf::from(&execution.cwd)],
        )
        .map_err(|_| "approval working-directory provenance unavailable")?;
        crate::privacy::records::merge_records(&mut notice, records);
    }
    let caps = crate::global_context::try_load_caps_quickly_if_not_present(app.gcx.clone(), 0)
        .await
        .map_err(|e| e.message)?;
    let model = crate::caps::resolve_chat_model(caps, &parent_thread.model)?;
    crate::privacy::destinations::clear_for_model(
        &app.gcx,
        ApprovalNoticeAudit(&notice),
        &model.base,
    )
    .map_err(|e| e.to_string())?;
    app.agent_approvals
        .register(pending.clone(), created)
        .map_err(|e| format!("approval registration: {e:?}"))?;
    let execution = app.agent_executions.get(&child_chat_id);
    let mut pending_guard = PendingApprovalGuard {
        registry: app.agent_approvals.clone(),
        questions: app.questions().clone(),
        asker: agent_id.clone(),
        parent: target.parent_id.clone(),
        generation: pending.run_generation,
        request_id: request_id.clone(),
        question_id: request_id.clone(),
        execution: execution.clone(),
        abort: Some(abort.clone()),
    };
    if !escalate {
        app.questions()
            .ask(
                refact_chat_api::AskQuestion::new(
                    refact_chat_api::QuestionKind::PermissionEscalation,
                    &agent_id,
                    &target.parent_id,
                    format!("{}:{}", agent_id, call.id),
                    format!("{} {}", pending.tool_name, pending.args),
                )
                .with_id(&request_id)
                .with_options(Some(vec!["approve".into(), "reject".into()]))
                .with_expiry_secs(Some(
                    refact_work_core::coordination_settings().approval_timeout_secs,
                )),
                chrono::Utc::now(),
            )
            .map_err(|error| format!("approval question registration: {error}"))?;
    }
    if let Some(execution) = &execution {
        execution.park().await.map_err(|e| e.to_string())?;
    }
    target.set_waiting(app, true).await?;
    user::show_waiting(app, &target, escalate).await?;
    if escalate {
        pending_guard.question_id = user::ask(app, &pending, &agent_id, &notice).await?;
    }
    let delivered = crate::chat::deliver_to_chat(
        app.clone(),
        &target.parent_id,
        refact_core::chat_types::PendingDelivery::new(
            vec![notice],
            refact_core::chat_types::PushMode::Append,
            crate::agents::push::NOTICE_APPROVAL.to_string(),
            !escalate,
        ),
    )
    .await;
    if let Err(error) = delivered {
        let _ = app.agent_approvals.cancel(
            &target.parent_id,
            pending.run_generation,
            &request_id,
            now_ms(),
        );
        return Err(error);
    }
    let result = match crate::questions::park_and_wait(app, &child_chat_id, &abort, async {
        if escalate {
            user::wait(app, &pending, &pending_guard.question_id).await?;
        }
        app.agent_approvals
            .wait(&request_id, now_ms())
            .await
            .map_err(|e| format!("approval wait: {e:?}"))
    })
    .await
    {
        Ok(decision) => decision,
        Err(_) => {
            let _ = app.agent_approvals.cancel(
                &target.parent_id,
                pending.run_generation,
                &request_id,
                now_ms(),
            );
            Err("approval cancelled".to_string())
        }
    };
    if let Ok(decision) = &result {
        if !escalate {
            settle_escalation_question(app, &request_id, &agent_id, &target.parent_id, *decision);
        }
    }
    if abort.load(std::sync::atomic::Ordering::Relaxed)
        || app
            .runtime
            .shutdown_flag
            .load(std::sync::atomic::Ordering::Relaxed)
    {
        if let Some(execution) = &execution {
            execution.cancel();
        }
        return Err("approval cancelled".into());
    }
    if let Some(execution) = &execution {
        let resume = execution.resume();
        tokio::pin!(resume);
        loop {
            if abort.load(std::sync::atomic::Ordering::Relaxed)
                || app
                    .runtime
                    .shutdown_flag
                    .load(std::sync::atomic::Ordering::Relaxed)
            {
                execution.cancel();
                return Err("approval cancelled".into());
            }
            tokio::select! {
                result = &mut resume => { result.map_err(|e| e.to_string())?; break; },
                _ = tokio::time::sleep(refact_work_core::coordination_settings().lock_poll_interval()) => {}
            }
        }
    }
    let current = super::approval_target::ApprovalTarget::resolve(app, &agent_id).await?;
    let current_scope = ccx
        .lock()
        .await
        .execution_scope_worktree()
        .map(|worktree| worktree.root.to_string_lossy().into_owned());
    if current_scope != pending.digest {
        return Err("approval execution scope changed while awaiting decision".into());
    }
    current.check(&pending)?;
    current.set_waiting(app, false).await?;
    if result? == ApprovalDecision::Approved {
        app.agent_approvals
            .consume(&request_id, &call.id, &pending.args, now_ms())
            .map_err(|e| format!("approval binding: {e:?}"))?;
        if crate::chat::tools::requires_actual_human_approval(reason) {
            let session = app
                .chat
                .sessions
                .read()
                .await
                .get(&child_chat_id)
                .cloned()
                .ok_or("child chat unavailable")?;
            let mut session = session.lock().await;
            crate::scheduler::automation_approval::accept_automation_approval(
                &mut session,
                &call.id,
            )?;
        }
        if let Some(execution) = pending.execution {
            ccx.lock().await.approved_executions.insert(
                call.id.clone(),
                super::command_approval::ApprovedExecution {
                    agent_id,
                    generation: pending.run_generation,
                    expires_at: pending.expires_at,
                    tool_name: pending.tool_name,
                    args: pending.args,
                    execution,
                    consumed: false,
                },
            );
        }
        pending_guard.execution = None;
        pending_guard.abort = None;
        Ok(true)
    } else {
        let denial = app.agent_approvals.denial_reason(&request_id);
        pending_guard.execution = None;
        pending_guard.abort = None;
        if let Some(denial) = denial {
            Err(denial)
        } else {
            Ok(false)
        }
    }
}

pub async fn parent_command_policy(
    app: &AppState,
    parent: &ThreadParams,
    pending: &PendingApproval,
) -> Result<
    (
        Vec<crate::chat::types::PauseReason>,
        Vec<crate::chat::types::PauseReason>,
    ),
    String,
> {
    if now_ms() >= pending.expires_at {
        return Err("approval request expired".into());
    }
    match app.agent_approvals.wait_now(&pending.id, now_ms()) {
        Err(refact_work_core::ApprovalError::NotApproved) => {}
        Ok(ApprovalDecision::Expired) => return Err("approval request expired".into()),
        _ => return Err("approval request is no longer pending".into()),
    }
    let session = app
        .chat
        .sessions
        .read()
        .await
        .get(&pending.parent_chat_id)
        .cloned()
        .ok_or("parent session unavailable")?;
    let allowed_tools = {
        let session = session.lock().await;
        if session.closed || session.thread.id != parent.id || parent.id != pending.parent_chat_id {
            return Err("parent approval authority unavailable".into());
        }
        session.active_command.allowed_tools.clone()
    };
    let target =
        super::approval_target::ApprovalTarget::resolve(app, &pending.child_work_id).await?;
    target.check(pending)?;
    let child_chat_id = target.chat_id;
    let child_session = app
        .chat
        .sessions
        .read()
        .await
        .get(&child_chat_id)
        .cloned()
        .ok_or("child execution scope unavailable")?;
    let child_thread = {
        let child = child_session.lock().await;
        if child.closed || child.abort_flag.load(std::sync::atomic::Ordering::Relaxed) {
            return Err("child approval execution was cancelled".into());
        }
        child.thread.clone()
    };
    if app
        .agent_executions
        .get(&child_chat_id)
        .is_some_and(|run| run.is_cancelled())
    {
        return Err("child approval execution was cancelled".into());
    }
    let child_worktree = child_thread.worktree.clone();
    if child_worktree
        .as_ref()
        .map(|worktree| worktree.root.to_string_lossy().into_owned())
        != pending.digest
    {
        return Err("child approval execution scope no longer matches the request".into());
    }
    let mode = crate::yaml_configs::customization_registry::get_mode_config(
        app.gcx.clone(),
        &parent.mode,
        Some(&parent.model),
    )
    .await
    .ok_or("parent approval mode unavailable")?;
    if crate::yaml_configs::customization_registry::match_tool_confirm_action(
        &mode.tool_confirm.rules,
        "agent_approval",
    )
    .as_deref()
        == Some("deny")
    {
        return Err("parent mode denies command approval".into());
    }
    let ceiling = super::tool_authority::current_ceiling(
        app,
        &pending.parent_chat_id,
        &parent.model,
        &parent.mode,
    )
    .await?;
    if !ceiling.allows_tool("agent_approval") || !ceiling.allows_tool(&pending.tool_name) {
        return Err("parent is not authorized to approve the requested child tool".into());
    }
    let mut effective = parent.clone();
    effective.worktree = child_worktree;
    let catalog = app
        .tool_registry
        .acquire_tool_catalog(
            &parent.mode,
            Some(&parent.model),
            parent
                .worktree
                .as_ref()
                .map(|w| w.root.to_string_lossy().into_owned())
                .as_deref(),
        )
        .await;
    if !catalog
        .index
        .tools
        .iter()
        .any(|tool| tool.name == pending.tool_name)
        || (!allowed_tools.is_empty()
            && !allowed_tools.iter().any(|tool| tool == &pending.tool_name))
    {
        return Err("parent is not authorized to execute the requested child tool".into());
    }
    let call: ChatToolCall = serde_json::from_value(serde_json::json!({
        "id":pending.tool_call_id,"type":"function","function":{"name":pending.tool_name,"arguments":serde_json::to_string(&pending.args).map_err(|e| e.to_string())?}
    })).map_err(|e| e.to_string())?;
    let child_context = Arc::new(Mutex::new(
        AtCommandsContext::new_from_app(
            app.clone(),
            1000,
            1,
            false,
            vec![],
            child_chat_id,
            child_thread.root_chat_id,
            child_thread.model,
            child_thread.task_meta,
            child_thread.worktree,
        )
        .await,
    ));
    super::tool_authority::ensure_context_tool(&child_context, &pending.tool_name).await?;
    let binding = super::command_approval::validate_pending(&child_context, &call, pending).await?;
    if binding != pending.execution {
        return Err("child approval execution binding changed".into());
    }
    Ok(crate::chat::tools::check_tools_confirmation(
        app.clone(),
        &[call],
        &[],
        &effective,
        Some(&catalog),
        &parent.mode,
        Some(&parent.model),
        &allowed_tools,
        "agent_approval",
    )
    .await)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use refact_privacy::{Attribution, FileRecord, PrivacyAudited};

    #[test]
    fn notice_audit_preserves_source_records() {
        let record = FileRecord {
            path: "/project/private.rs".into(),
            zone: "private".into(),
            attribution: Attribution::Declared,
        };
        let mut notice = ChatMessage::new("event".into(), "approval".into());
        notice.extra.insert(
            "privacy".into(),
            serde_json::json!({"files": [record.clone()]}),
        );
        assert_eq!(
            ApprovalNoticeAudit(&notice).privacy_records().unwrap(),
            vec![(0, record)]
        );
    }

    #[test]
    fn notice_audit_rejects_malformed_provenance() {
        let mut notice = ChatMessage::new("event".into(), "approval".into());
        notice
            .extra
            .insert("privacy".into(), serde_json::json!({"files": "invalid"}));
        assert!(ApprovalNoticeAudit(&notice).privacy_records().is_err());
    }

    #[test]
    fn dropping_pending_approval_cancels_it_without_changing_an_existing_decision() {
        for approve in [false, true] {
            let registry = refact_work_core::ApprovalRegistry::new(4);
            let now = now_ms();
            registry
                .register(
                    PendingApproval {
                        id: "request".into(),
                        child_work_id: "child".into(),
                        parent_chat_id: "parent".into(),
                        run_generation: 1,
                        tool_call_id: "call".into(),
                        tool_name: "shell".into(),
                        args: serde_json::json!({}),
                        requires_human: false,
                        digest: None,
                        created_at: now,
                        expires_at: now + 60_000,
                        execution: None,
                    },
                    now,
                )
                .unwrap();
            if approve {
                registry.decide("parent", 1, "request", true, now).unwrap();
            }
            drop(PendingApprovalGuard {
                registry: registry.clone(),
                questions: refact_chat_api::QuestionRegistry::new(Default::default()).unwrap(),
                asker: "child".into(),
                parent: "parent".into(),
                generation: 1,
                request_id: "request".into(),
                question_id: "request".into(),
                execution: None,
                abort: None,
            });
            assert_eq!(
                registry.wait_now("request", now).unwrap(),
                if approve {
                    ApprovalDecision::Approved
                } else {
                    ApprovalDecision::Cancelled
                }
            );
        }
    }

    pub(crate) struct Fixture {
        pub(crate) dir: tempfile::TempDir,
        pub(crate) app: AppState,
        pub(crate) ccx: Arc<Mutex<AtCommandsContext>>,
        pub(crate) agent_id: String,
        pub(crate) generation: u64,
        execution: Arc<crate::agents::execution::RunExecution>,
    }

    pub(crate) async fn fixture() -> Fixture {
        fixture_with_hooks(true).await
    }

    pub(crate) async fn fixture_with_hooks(enabled: bool) -> Fixture {
        use crate::agents::types::{CreateAgentRequest, BgAgentKind};
        use refact_work_core::{AdmissionRequest, WorkKey, WorkScope, ProjectKey, CheckoutKey};
        let dir = tempfile::tempdir().unwrap();
        let mut gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().join("cache"),
            dir.path().join("config"),
        )
        .await;
        Arc::get_mut(&mut gcx).unwrap().hooks_config.enabled = enabled;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().into()];
        let mut shell_policy = crate::tools::shell_gate::ShellGatePolicy::default();
        shell_policy.execution.foreground_timeout_secs = 20;
        crate::tools::shell_gate::save_policy(gcx.clone(), &shell_policy)
            .await
            .unwrap();
        let (root, _) = crate::caps::project_model_defaults_state(gcx.clone()).await;
        let mut caps = crate::caps::CodeAssistantCaps::default();
        caps.chat_models.insert(
            "test/model".into(),
            Arc::new(crate::caps::ChatModelRecord {
                base: crate::caps::BaseModelRecord {
                    id: "test/model".into(),
                    ..Default::default()
                },
                ..Default::default()
            }),
        );
        {
            let mut state = gcx.caps_state.write().await;
            state.caps = Some(Arc::new(caps));
            state.loaded_project_root = Some(root);
            state.last_attempted_ts = now_ms() / 1000;
        }
        let app = AppState::from_gcx(gcx).await;
        let mut parent = crate::chat::types::ChatSession::new("parent".into());
        parent.thread.model = "test/model".into();
        parent.work_project_root = Some(dir.path().into());
        parent.set_runtime_state(crate::chat::types::SessionState::Generating, None);
        app.chat
            .sessions
            .write()
            .await
            .insert("parent".into(), Arc::new(Mutex::new(parent)));
        let (record, _, _) = app
            .agents
            .create(CreateAgentRequest {
                parent_chat_id: "parent".into(),
                parent_root_chat_id: Some("parent".into()),
                parent_tool_call_id: None,
                kind: BgAgentKind::Subagent,
                config_name: "test".into(),
                title: "child".into(),
                prompt: "test".into(),
                target_files: vec![],
                model: "test/model".into(),
                model_type: None,
                goal_summary: None,
                plan_present: false,
                worktree_id: None,
                worktree_branch: None,
            })
            .await
            .unwrap();
        let child_id = "subchat-approval-test";
        app.agents
            .mark_running(&record.agent_id, child_id.into())
            .await
            .unwrap();
        app.agent_executions.remember_config(
            &record.agent_id,
            crate::subchat::SubchatConfig {
                runner_lease_token: None,
                native_job_fence: None,
                unlimited_configured_steps: false,
                history_source_model: None,
                tool_name: "subagent".into(),
                stateful: true,
                autonomous_no_confirm: false,
                auto_approve_editing_tools: false,
                auto_approve_dangerous_commands: false,
                chat_id: Some(child_id.into()),
                title: Some("child".into()),
                parent_id: Some("parent".into()),
                link_type: Some("subagent".into()),
                root_chat_id: Some("parent".into()),
                tools: crate::subchat::ToolsPolicy::Only(vec![
                    "shell".into(),
                    "process_start".into(),
                ]),
                max_steps: 10,
                prepend_system_prompt: false,
                wrap_up: None,
                task_meta: None,
                worktree: None,
                model: "test/model".into(),
                mode: "agent".into(),
                n_ctx: 4096,
                implicit_context_window: false,
                max_new_tokens: 512,
                temperature: None,
                reasoning_effort: None,
                cache_control: crate::llm::params::CacheControl::Ephemeral,
                parent_tool_call_id: None,
                parent_subchat_tx: None,
                abort_flag: None,
                soft_abort: false,
                background_agent_id: Some(record.agent_id.clone()),
                subchat_depth: 1,
                final_step_force_answer: false,
                buddy_meta: None,
                step_progress: None,
                activity_stamp: None,
                trace_parent: crate::subchat::TraceParent::rooted("parent", "parent"),
            },
        );
        let mut child = crate::chat::types::ChatSession::new(child_id.into());
        child.thread.model = "test/model".into();
        child.work_project_root = Some(dir.path().into());
        app.chat
            .sessions
            .write()
            .await
            .insert(child_id.into(), Arc::new(Mutex::new(child)));
        let permit = app
            .work_scheduler
            .reserve(AdmissionRequest::new(
                WorkKey::background_agent(&record.agent_id).run(record.run_generation),
                WorkScope::new(ProjectKey::new("project"), CheckoutKey::new("root")),
            ))
            .unwrap()
            .acquire()
            .await
            .unwrap();
        let execution = app.agent_executions.install(child_id, permit).unwrap();
        let ccx = Arc::new(Mutex::new(
            AtCommandsContext::new_from_app(
                app.clone(),
                4096,
                10,
                false,
                vec![],
                child_id.into(),
                Some("parent".into()),
                "test/model".into(),
                None,
                None,
            )
            .await,
        ));
        ccx.lock().await.background_agent_id = Some(record.agent_id.clone());
        Fixture {
            dir,
            app,
            ccx,
            agent_id: record.agent_id,
            generation: record.run_generation,
            execution,
        }
    }

    impl Fixture {
        pub(crate) fn call(&self, tool: &str) -> ChatToolCall {
            let command = if cfg!(windows) {
                r#"[System.IO.File]::AppendAllText((Join-Path (Get-Location) 'approval.txt'), "approved`n", [System.Text.Encoding]::ASCII)"#
            } else {
                "echo approved >> approval.txt"
            };
            serde_json::from_value(serde_json::json!({"id":"command","type":"function","function":{
                "name":tool, "arguments":serde_json::json!({"command":command, "description":"Record approval", "workdir":self.dir.path(), "tty":false, "needs_confirmation":true}).to_string()
            }})).unwrap()
        }

        pub(crate) async fn waiting<F: std::future::Future<Output = Result<bool, String>>>(
            &self,
            mut future: std::pin::Pin<&mut F>,
        ) -> PendingApproval {
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                tokio::select! {
                    result = future.as_mut() => panic!("approval ended before decision: {result:?}"),
                    pending = async {
                        loop {
                            if let Some(pending) = self
                                .app
                                .agent_approvals
                                .list_for_parent("parent", now_ms())
                                .first()
                                .cloned()
                            {
                                let waiting = if self.agent_id.starts_with("board:") {
                                    let id = self.ccx.lock().await.chat_id.clone();
                                    let child = self
                                        .app
                                        .chat
                                        .sessions
                                        .read()
                                        .await
                                        .get(&id)
                                        .cloned()
                                        .unwrap();
                                    let waiting = child.lock().await.runtime.state
                                        == crate::chat::types::SessionState::WaitingUserInput;
                                    waiting
                                } else {
                                    self.app
                                        .agents
                                        .get_any(&self.agent_id)
                                        .await
                                        .unwrap()
                                        .status
                                        == crate::agents::types::BgAgentStatus::WaitingForApproval
                                };
                                if waiting {
                                    if !self.agent_id.starts_with("board:") {
                                        self.app
                                            .agents
                                            .flush_pending_writes_for_test()
                                            .await
                                            .unwrap();
                                    }
                                    break pending;
                                }
                            }
                            tokio::task::yield_now().await;
                        }
                    } => pending,
                }
            })
            .await
            .unwrap()
        }

        async fn execute(
            &self,
            call: &ChatToolCall,
        ) -> Result<refact_runtime_api::ToolExecutionResult, String> {
            self.app
                .tool_registry
                .execute_tool(
                    &self.ccx,
                    "agent",
                    Some("test/model"),
                    &call.id,
                    &call.function.name,
                    serde_json::from_str(&call.function.arguments).unwrap(),
                )
                .await?
                .ok_or("tool not registered".into())
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            self.execution.cancel();
            self.app.agent_executions.finish("subchat-approval-test");
        }
    }

    #[tokio::test]
    async fn child_command_approval_parks_without_side_effects_then_resumes_with_exact_binding() {
        let fixture = fixture().await;
        let call = fixture.call("shell");
        let mut request_future =
            Box::pin(request(&fixture.app, &fixture.ccx, &call, "needs decision"));
        let pending = fixture.waiting(request_future.as_mut()).await;
        assert_eq!(
            fixture.execution.state(),
            crate::agents::execution::ExecutionState::Parked
        );
        assert!(pending.execution.is_some());
        assert!(!fixture.dir.path().join("approval.txt").exists());
        fixture
            .app
            .agent_approvals
            .decide("parent", fixture.generation, &pending.id, true, now_ms())
            .unwrap();
        assert!(request_future.await.unwrap());
        assert_eq!(
            fixture.execution.state(),
            crate::agents::execution::ExecutionState::Running
        );
        assert!(fixture
            .ccx
            .lock()
            .await
            .approved_executions
            .contains_key("command"));
        let output = fixture.execute(&call).await.unwrap();
        assert!(
            output
                .messages
                .iter()
                .all(|message| message.tool_failed != Some(true)),
            "child command failed: {:?}",
            output
                .messages
                .iter()
                .filter(|message| message.tool_failed == Some(true))
                .map(|message| message.content.content_text_only())
                .collect::<Vec<_>>()
        );
        let bytes = std::fs::read(fixture.dir.path().join("approval.txt")).unwrap();
        assert!(!bytes.is_empty());
        assert!(fixture
            .execute(&call)
            .await
            .err()
            .unwrap()
            .contains("already been used"));
        assert_eq!(
            bytes,
            std::fs::read(fixture.dir.path().join("approval.txt")).unwrap()
        );
        assert!(request(&fixture.app, &fixture.ccx, &call, "retry")
            .await
            .err()
            .unwrap()
            .contains("already exists"));
    }

    #[tokio::test]
    async fn child_app_write_needs_no_human_approval() {
        let fixture = fixture().await;
        let mut config = fixture
            .app
            .agent_executions
            .saved_config(&fixture.agent_id)
            .unwrap();
        config.tools = crate::subchat::ToolsPolicy::Only(vec!["app".into()]);
        fixture
            .app
            .agent_executions
            .remember_config(&fixture.agent_id, config);
        // The context captured the original shell-only ceiling at construction.
        fixture.ccx.lock().await.child_tool_ceiling =
            Some(crate::subchat::ToolsPolicy::Only(vec!["app".into()]));
        // Parent policy must independently allow the App; user-facing catalog
        // augmentation does not grant authority to a child.
        let gcx = fixture.app.gcx.clone();
        let mut modes =
            crate::yaml_configs::customization_registry::get_project_registry(gcx.clone())
                .await
                .unwrap();
        modes.modes.insert(
            "agent".into(),
            serde_yaml::from_str("schema_version: 1\nid: agent\ntools: [shell, process_start, app, agent_approval]\n").unwrap(),
        );
        gcx.project_registry_cache
            .write()
            .unwrap()
            .insert(fixture.dir.path().to_path_buf(), modes);
        let created = crate::apps::execute(
            &fixture.app,
            crate::apps::Caller::Host,
            crate::apps::Command::Create {
                slug: "human-child".into(),
                name: "Human child".into(),
            },
        )
        .await
        .unwrap();
        let app_id: uuid::Uuid =
            serde_json::from_value(created["manifest"]["app_id"].clone()).unwrap();
        let parent = fixture.app.chat.sessions.read().await["parent"].clone();
        let mut workspace = refact_chat_api::AppWorkspace::empty();
        workspace.bindings.push(refact_chat_api::AppBinding {
            binding_id: uuid::Uuid::new_v4(),
            app_id,
            params: serde_json::Map::new(),
            state: serde_json::Map::new(),
        });
        {
            let mut parent = parent.lock().await;
            parent.thread.auto_approve_dangerous_commands = true;
            parent.thread.app_workspace =
                Some(refact_chat_api::StoredAppWorkspace::Supported(workspace));
        }
        let call: ChatToolCall = serde_json::from_value(serde_json::json!({
            "id":"app-write", "type":"function", "function":{"name":"app", "arguments":serde_json::json!({
                "action":"write_file", "app_id":app_id, "path":"ui/index.html", "content":"human approved",
                "expected_revision":created["revision"]
            }).to_string()}
        })).unwrap();
        let result = fixture.execute(&call).await.unwrap();
        assert!(
            result
                .messages
                .iter()
                .all(|message| message.tool_failed != Some(true)),
            "child App write failed"
        );
        fixture.app.gcx.apps.shutdown().await;
    }

    #[tokio::test]
    async fn dropped_approval_cancels_its_parked_execution_and_pending_request() {
        let fixture = fixture().await;
        let call = fixture.call("process_start");
        let mut future = Box::pin(request(&fixture.app, &fixture.ccx, &call, "needs decision"));
        let pending = fixture.waiting(future.as_mut()).await;
        drop(future);
        assert_eq!(
            fixture.execution.state(),
            crate::agents::execution::ExecutionState::Gone
        );
        assert_eq!(fixture.app.work_scheduler.snapshot().outstanding, 0);
        assert_eq!(
            fixture.app.agent_approvals.wait_now(&pending.id, now_ms()),
            Ok(ApprovalDecision::Cancelled)
        );
        assert!(fixture.ccx.lock().await.approved_executions.is_empty());
        assert!(!fixture.dir.path().join("approval.txt").exists());
    }

    #[tokio::test]
    async fn changed_command_arguments_or_tool_cannot_reuse_an_approved_call() {
        for change_tool in [false, true] {
            let fixture = fixture().await;
            let call = fixture.call("shell");
            let mut future = Box::pin(request(&fixture.app, &fixture.ccx, &call, "needs decision"));
            let pending = fixture.waiting(future.as_mut()).await;
            fixture
                .app
                .agent_approvals
                .decide("parent", fixture.generation, &pending.id, true, now_ms())
                .unwrap();
            assert!(future.await.unwrap());
            let mut changed = call.clone();
            if change_tool {
                changed.function.name = "process_start".into();
            } else {
                let mut args: serde_json::Value =
                    serde_json::from_str(&call.function.arguments).unwrap();
                args["run_in_background"] = true.into();
                changed.function.arguments = args.to_string();
            }
            assert!(fixture
                .execute(&changed)
                .await
                .err()
                .unwrap()
                .contains("approved tool or arguments changed"));
            assert!(fixture
                .execute(&call)
                .await
                .err()
                .unwrap()
                .contains("already been used"));
            assert!(!fixture.dir.path().join("approval.txt").exists());
        }
    }

    pub(crate) async fn board_fixture() -> Fixture {
        let mut fixture = fixture().await;
        fixture.app.agent_executions.finish("subchat-approval-test");
        let child_id = format!("board-child-{}", uuid::Uuid::new_v4());
        let task_id = "approval-task";
        let parent = fixture
            .app
            .chat
            .sessions
            .read()
            .await
            .get("parent")
            .cloned()
            .unwrap();
        parent.lock().await.thread.task_meta = Some(crate::chat::types::TaskMeta {
            task_id: task_id.into(),
            role: "planner".into(),
            planner_chat_id: Some("parent".into()),
            ..Default::default()
        });
        let mut child = crate::chat::types::ChatSession::new(child_id.clone());
        child.thread.model = "test/model".into();
        child.thread.mode = "task_agent".into();
        child.work_project_root = Some(fixture.dir.path().into());
        child.thread.task_meta = Some(crate::chat::types::TaskMeta {
            task_id: task_id.into(),
            role: "agents".into(),
            card_id: Some("card".into()),
            agent_id: Some("worker".into()),
            planner_chat_id: Some("parent".into()),
        });
        let meta = child.thread.task_meta.clone();
        fixture
            .app
            .chat
            .sessions
            .write()
            .await
            .insert(child_id.clone(), Arc::new(Mutex::new(child)));
        std::fs::create_dir_all(fixture.dir.path().join(".refact/tasks").join(task_id)).unwrap();
        let mut board = crate::tasks::types::TaskBoard::default();
        board.owner_chat_id = Some("parent".into());
        let card: crate::tasks::types::BoardCard = serde_json::from_value(serde_json::json!({
            "id":"card", "title":"Approval card", "column":"doing", "created_at":"2026-01-01T00:00:00Z", "agent_chat_id":child_id, "assignee":"worker"
        })).unwrap();
        board.cards.push(card);
        crate::tasks::storage::save_board(fixture.app.gcx.clone(), task_id, &board)
            .await
            .unwrap();
        crate::tasks::admission::reserve(&fixture.app, &child_id, "project", "parent", 0)
            .await
            .unwrap();
        crate::tasks::admission::wait_for_permit(&child_id)
            .await
            .unwrap();
        fixture.execution = fixture.app.agent_executions.get(&child_id).unwrap();
        fixture.generation = fixture.execution.admission_generation();
        fixture.agent_id = format!("board:{child_id}");
        fixture.ccx = Arc::new(Mutex::new(
            AtCommandsContext::new_from_app(
                fixture.app.clone(),
                4096,
                10,
                false,
                vec![],
                child_id,
                Some("parent".into()),
                "test/model".into(),
                meta,
                None,
            )
            .await,
        ));
        fixture
    }

    #[tokio::test]
    async fn board_tool_loop_relays_exact_command_and_executes_only_after_parent_decision() {
        let fixture = board_fixture().await;
        let child_id = fixture.ccx.lock().await.chat_id.clone();
        let session = fixture
            .app
            .chat
            .sessions
            .read()
            .await
            .get(&child_id)
            .cloned()
            .unwrap();
        let call = fixture.call("shell");
        let parent = fixture
            .app
            .chat
            .sessions
            .read()
            .await
            .get("parent")
            .cloned()
            .unwrap();
        parent.lock().await.thread.auto_approve_dangerous_commands = true;
        {
            let mut child = session.lock().await;
            child.thread.parent_id = Some("parent".into());
            child.thread.auto_approve_dangerous_commands = true;
        }
        let thread = session.lock().await.thread.clone();
        let (asks, denials) = crate::chat::tools::check_tools_confirmation(
            fixture.app.clone(),
            &[call.clone()],
            &[],
            &thread,
            None,
            "task_agent",
            Some("test/model"),
            &[],
            "test",
        )
        .await;
        assert!(asks.is_empty(), "{asks:?}");
        assert!(denials.is_empty(), "{denials:?}");
        parent.lock().await.thread.auto_approve_dangerous_commands = false;
        let (asks, denials) = crate::chat::tools::check_tools_confirmation(
            fixture.app.clone(),
            &[call.clone()],
            &[],
            &thread,
            None,
            "task_agent",
            Some("test/model"),
            &[],
            "test",
        )
        .await;
        assert_eq!(asks.len(), 1);
        assert!(denials.is_empty(), "{denials:?}");
        session.lock().await.add_message(ChatMessage {
            role: "assistant".into(),
            tool_calls: Some(vec![call.clone()]),
            ..Default::default()
        });
        let mut step = Box::pin(crate::chat::tools::process_tool_calls_once(
            fixture.app.clone(),
            session.clone(),
            "task_agent",
            Some("test/model"),
        ));
        let pending = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                assert!(futures::poll!(step.as_mut()).is_pending());
                if fixture.execution.state() == super::super::execution::ExecutionState::Parked {
                    if let Some(pending) = fixture
                        .app
                        .agent_approvals
                        .list_for_parent("parent", now_ms())
                        .first()
                        .cloned()
                    {
                        break pending;
                    }
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(pending.child_work_id, fixture.agent_id);
        assert_eq!(pending.run_generation, fixture.generation);
        assert!(pending.execution.is_some());
        assert!(!fixture.dir.path().join("approval.txt").exists());
        fixture
            .app
            .agent_approvals
            .grant_user_once(
                "parent",
                &pending.id,
                pending.run_generation,
                "approve-board",
                now_ms(),
            )
            .unwrap();
        let parent_ccx = Arc::new(Mutex::new(
            AtCommandsContext::new_from_app(
                fixture.app.clone(),
                4096,
                10,
                false,
                vec![],
                "parent".into(),
                None,
                "test/model".into(),
                None,
                None,
            )
            .await,
        ));
        let output = fixture
            .app
            .tool_registry
            .execute_tool(
                &parent_ccx,
                "task_planner",
                Some("test/model"),
                "approve-board",
                "agent_approval",
                serde_json::json!({"action":"approve", "request_id":pending.id})
                    .as_object()
                    .unwrap()
                    .clone(),
            )
            .await
            .unwrap()
            .expect("public approval tool unavailable");
        assert!(output
            .messages
            .iter()
            .all(|message| message.tool_failed != Some(true)));
        assert!(matches!(
            tokio::time::timeout(std::time::Duration::from_secs(30), step)
                .await
                .unwrap(),
            crate::chat::tools::ToolStepOutcome::Continue
        ));
        let child_tool_results = session
            .lock()
            .await
            .messages
            .iter()
            .filter(|message| message.role == "tool" && message.tool_call_id == call.id)
            .map(|message| (message.tool_failed, message.content.content_text_only()))
            .collect::<Vec<_>>();
        assert_eq!(
            std::fs::read_to_string(fixture.dir.path().join("approval.txt"))
                .unwrap_or_else(|error| {
                    panic!(
                        "reading approval.txt after child tool step failed: {error}; child tool results: {child_tool_results:?}"
                    )
                })
                .lines()
                .count(),
            1
        );
        assert_eq!(
            fixture
                .app
                .agent_approvals
                .consume(&pending.id, &call.id, &pending.args, now_ms()),
            Err(refact_work_core::ApprovalError::Consumed)
        );
        crate::tasks::admission::end(&child_id).await;
    }

    #[tokio::test]
    async fn user_decision_queue_grants_the_exact_wrapper_once_without_running_a_model() {
        let fixture = fixture().await;
        let call = fixture.call("shell");
        let mut waiting = Box::pin(request(&fixture.app, &fixture.ccx, &call, "user decision"));
        let pending = fixture.waiting(waiting.as_mut()).await;
        let parent = fixture
            .app
            .chat
            .sessions
            .read()
            .await
            .get("parent")
            .cloned()
            .unwrap();
        let wrapper: ChatToolCall = serde_json::from_value(serde_json::json!({
            "id":"user-approved-wrapper", "type":"function", "function":{
                "name":"agent_approval", "arguments":serde_json::json!({"action":"approve", "request_id":pending.id}).to_string()
            }
        })).unwrap();
        {
            let mut session = parent.lock().await;
            let index = session.messages.len();
            session.add_message(ChatMessage {
                role: "assistant".into(),
                tool_calls: Some(vec![wrapper.clone()]),
                ..Default::default()
            });
            session.set_paused_with_reasons_and_auto_approved(
                vec![crate::chat::types::PauseReason {
                    reason_type: "confirmation".into(),
                    tool_name: "agent_approval".into(),
                    command: "Approve exact child call".into(),
                    rule: "explicit user approval".into(),
                    tool_call_id: wrapper.id.clone(),
                    integr_config_path: None,
                }],
                vec![],
                Some(index),
            );
            session
                .user_interrupt_flag
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
        let decision = crate::chat::types::ToolDecisionItem {
            tool_call_id: wrapper.id.clone(),
            accepted: true,
        };
        crate::chat::handle_tool_decisions_guarded(
            fixture.app.clone(),
            parent.clone(),
            std::slice::from_ref(&decision),
            None,
            "approval-test",
        )
        .await;
        assert!(waiting.await.unwrap());
        crate::chat::handle_tool_decisions_guarded(
            fixture.app.clone(),
            parent.clone(),
            &[decision],
            None,
            "approval-test",
        )
        .await;
        let session = parent.lock().await;
        let results: Vec<_> = session
            .messages
            .iter()
            .filter(|message| message.role == "tool" && message.tool_call_id == wrapper.id)
            .collect();
        assert_eq!(results.len(), 1);
        assert_ne!(results[0].tool_failed, Some(true));
        assert!(!fixture.dir.path().join("approval.txt").exists());
    }

    #[tokio::test]
    async fn user_rejection_releases_child_and_changed_wrapper_cannot_acquire_authority() {
        for changed in [false, true] {
            let fixture = fixture().await;
            let call = fixture.call("shell");
            let mut waiting = Box::pin(request(&fixture.app, &fixture.ccx, &call, "user decision"));
            let pending = fixture.waiting(waiting.as_mut()).await;
            let parent = fixture
                .app
                .chat
                .sessions
                .read()
                .await
                .get("parent")
                .cloned()
                .unwrap();
            let wrapper: ChatToolCall = serde_json::from_value(serde_json::json!({
                "id":"wrapper", "type":"function", "function":{
                    "name":"agent_approval", "arguments":serde_json::json!({"action":"approve", "request_id":pending.id}).to_string()
                }
            })).unwrap();
            {
                let mut session = parent.lock().await;
                let index = session.messages.len();
                session.add_message(ChatMessage {
                    role: "assistant".into(),
                    tool_calls: Some(vec![wrapper]),
                    ..Default::default()
                });
                session.set_paused_with_reasons_and_auto_approved(
                    vec![crate::chat::types::PauseReason {
                        reason_type: "confirmation".into(),
                        tool_name: "agent_approval".into(),
                        command: "Approve exact child call".into(),
                        rule: "explicit user approval".into(),
                        tool_call_id: "wrapper".into(),
                        integr_config_path: None,
                    }],
                    vec![],
                    Some(index),
                );
                if changed {
                    session.messages[index].tool_calls.as_mut().unwrap()[0]
                        .function
                        .arguments =
                        serde_json::json!({"action":"reject", "request_id":pending.id}).to_string();
                }
                session
                    .user_interrupt_flag
                    .store(true, std::sync::atomic::Ordering::SeqCst);
            }
            crate::chat::handle_tool_decisions_guarded(
                fixture.app.clone(),
                parent.clone(),
                &[crate::chat::types::ToolDecisionItem {
                    tool_call_id: "wrapper".into(),
                    accepted: changed,
                }],
                None,
                "approval-test",
            )
            .await;
            if changed {
                assert_eq!(
                    fixture.app.agent_approvals.wait_now(&pending.id, now_ms()),
                    Ok(ApprovalDecision::Cancelled)
                );
                assert!(parent
                    .lock()
                    .await
                    .messages
                    .iter()
                    .any(|message| message.tool_call_id == "wrapper"
                        && message.tool_failed == Some(true)));
                drop(waiting);
            } else {
                assert_eq!(
                    fixture.app.agent_approvals.wait_now(&pending.id, now_ms()),
                    Ok(ApprovalDecision::Rejected)
                );
                assert!(!waiting.await.unwrap());
            }
            assert!(!fixture.dir.path().join("approval.txt").exists());
        }
    }

    #[tokio::test]
    async fn both_runtimes_reject_changed_environment_and_cancelled_execution() {
        for board in [false, true] {
            for tool in ["shell", "process_start"] {
                for change in ["environment", "abort", "execution", "expiry", "shutdown"] {
                    let fixture = if board {
                        board_fixture().await
                    } else {
                        fixture().await
                    };
                    let call = fixture.call(tool);
                    let mut waiting =
                        Box::pin(request(&fixture.app, &fixture.ccx, &call, "exact binding"));
                    let pending = fixture.waiting(waiting.as_mut()).await;
                    fixture
                        .app
                        .agent_approvals
                        .decide("parent", fixture.generation, &pending.id, true, now_ms())
                        .unwrap();
                    assert!(waiting.await.unwrap());
                    match change {
                        "environment" => {
                            std::fs::write(
                                fixture.app.gcx.config_dir.join("variables.yaml"),
                                "APPROVAL_BINDING_TEST: changed\n",
                            )
                            .unwrap();
                        }
                        "abort" => fixture
                            .ccx
                            .lock()
                            .await
                            .abort_flag
                            .store(true, std::sync::atomic::Ordering::SeqCst),
                        "execution" => fixture.execution.cancel(),
                        "expiry" => {
                            fixture
                                .ccx
                                .lock()
                                .await
                                .approved_executions
                                .get_mut(&call.id)
                                .unwrap()
                                .expires_at = 0
                        }
                        _ => fixture
                            .app
                            .runtime
                            .shutdown_flag
                            .store(true, std::sync::atomic::Ordering::SeqCst),
                    }
                    let error = fixture
                        .execute(&call)
                        .await
                        .err()
                        .expect("changed approval executed");
                    let expected = match change {
                        "environment" => "environment or privilege changed",
                        "expiry" => "expired before execution",
                        _ => "cancelled before execution",
                    };
                    assert!(error.contains(expected), "{board} {tool} {change}: {error}");
                    assert!(!fixture.dir.path().join("approval.txt").exists());
                    if board {
                        let child_id = fixture.ccx.lock().await.chat_id.clone();
                        crate::tasks::admission::end(&child_id).await;
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn approvals_wait_for_mutations_to_drain_before_releasing_capacity() {
        for board in [false, true] {
            let fixture = if board {
                board_fixture().await
            } else {
                fixture().await
            };
            let operation = fixture.execution.begin_operation().await.unwrap();
            let call = fixture.call("shell");
            let mut waiting = Box::pin(request(&fixture.app, &fixture.ccx, &call, "drain"));
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                loop {
                    assert!(futures::poll!(waiting.as_mut()).is_pending());
                    if fixture.execution.pending_waits() > 0 {
                        break;
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            assert!(fixture.execution.holds_permit());
            assert_eq!(fixture.execution.active_operations(), 1);
            drop(operation);
            let pending = fixture.waiting(waiting.as_mut()).await;
            assert!(!fixture.execution.holds_permit());
            fixture
                .app
                .agent_approvals
                .decide("parent", fixture.generation, &pending.id, false, now_ms())
                .unwrap();
            assert!(!waiting.await.unwrap());
            assert!(fixture.execution.holds_permit());
            assert!(!fixture.dir.path().join("approval.txt").exists());
            if board {
                let child_id = fixture.ccx.lock().await.chat_id.clone();
                crate::tasks::admission::end(&child_id).await;
            }
        }
    }

    async fn pause_wrappers(
        fixture: &Fixture,
        requests: &[&PendingApproval],
    ) -> (
        Arc<Mutex<crate::chat::types::ChatSession>>,
        usize,
        Vec<ChatToolCall>,
    ) {
        let parent = fixture
            .app
            .chat
            .sessions
            .read()
            .await
            .get("parent")
            .cloned()
            .unwrap();
        let wrappers: Vec<ChatToolCall> = requests.iter().enumerate().map(|(index, request)| {
            serde_json::from_value(serde_json::json!({
                "id":format!("wrapper-{index}"), "type":"function", "function":{
                    "name":"agent_approval", "arguments":serde_json::json!({"action":"approve", "request_id":request.id}).to_string()
                }
            })).unwrap()
        }).collect();
        let mut session = parent.lock().await;
        let index = session.messages.len();
        session.add_message(ChatMessage {
            role: "assistant".into(),
            tool_calls: Some(wrappers.clone()),
            ..Default::default()
        });
        session.set_paused_with_reasons_and_auto_approved(
            wrappers
                .iter()
                .map(|call| crate::chat::types::PauseReason {
                    reason_type: "confirmation".into(),
                    tool_name: "agent_approval".into(),
                    command: "Approve exact child call".into(),
                    rule: "explicit user approval".into(),
                    tool_call_id: call.id.clone(),
                    integr_config_path: None,
                })
                .collect(),
            vec![],
            Some(index),
        );
        session
            .user_interrupt_flag
            .store(true, std::sync::atomic::Ordering::SeqCst);
        drop(session);
        (parent, index, wrappers)
    }

    #[tokio::test]
    async fn partial_user_decisions_bind_original_wrappers_and_never_grant_early() {
        for mutation in ["unchanged", "arguments", "tool", "removed"] {
            let fixture = fixture().await;
            let call = fixture.call("shell");
            let mut waiting = Box::pin(request(&fixture.app, &fixture.ccx, &call, "decision"));
            let first = fixture.waiting(waiting.as_mut()).await;
            let mut second = first.clone();
            second.id = uuid::Uuid::new_v4().to_string();
            second.tool_call_id = "second-command".into();
            fixture
                .app
                .agent_approvals
                .register(second.clone(), now_ms())
                .unwrap();
            let (parent, index, wrappers) = pause_wrappers(&fixture, &[&first, &second]).await;
            let decision = crate::chat::types::ToolDecisionItem {
                tool_call_id: wrappers[0].id.clone(),
                accepted: true,
            };
            crate::chat::handle_tool_decisions_guarded(
                fixture.app.clone(),
                parent.clone(),
                &[decision.clone(), decision.clone()],
                None,
                "approval-test",
            )
            .await;
            assert_eq!(
                fixture.app.agent_approvals.consume_user_once(
                    "parent",
                    &first.id,
                    first.run_generation,
                    &wrappers[0].id,
                    now_ms(),
                ),
                Err(refact_work_core::ApprovalError::NotApproved)
            );
            assert_eq!(parent.lock().await.runtime.pause_reasons.len(), 1);
            assert_eq!(
                fixture.execution.state(),
                super::super::execution::ExecutionState::Parked
            );
            {
                let mut session = parent.lock().await;
                let calls = session.messages[index].tool_calls.as_mut().unwrap();
                match mutation {
                    "arguments" => {
                        calls[0].function.arguments =
                            serde_json::json!({"action":"approve", "request_id":second.id})
                                .to_string()
                    }
                    "tool" => calls[0].function = call.function.clone(),
                    "removed" => {
                        calls.remove(0);
                    }
                    _ => {}
                }
            }
            crate::chat::handle_tool_decisions_guarded(
                fixture.app.clone(),
                parent.clone(),
                &[crate::chat::types::ToolDecisionItem {
                    tool_call_id: wrappers[1].id.clone(),
                    accepted: true,
                }],
                None,
                "approval-test",
            )
            .await;
            assert_eq!(
                fixture.app.agent_approvals.wait_now(&first.id, now_ms()),
                Ok(if mutation == "unchanged" {
                    ApprovalDecision::Approved
                } else {
                    ApprovalDecision::Cancelled
                })
            );
            assert_eq!(
                fixture.app.agent_approvals.wait_now(&second.id, now_ms()),
                Ok(ApprovalDecision::Approved)
            );
            assert_eq!(waiting.await.unwrap(), mutation == "unchanged");
            crate::chat::handle_tool_decisions_guarded(
                fixture.app.clone(),
                parent.clone(),
                &[decision],
                None,
                "approval-test",
            )
            .await;
            let session = parent.lock().await;
            for wrapper in &wrappers {
                let count = session
                    .messages
                    .iter()
                    .filter(|message| message.role == "tool" && message.tool_call_id == wrapper.id)
                    .count();
                assert_eq!(
                    count,
                    usize::from(mutation != "removed" || wrapper.id != wrappers[0].id),
                    "{mutation}"
                );
            }
            assert!(session.approval_wrapper_calls.is_empty());
            assert!(!fixture.dir.path().join("approval.txt").exists());
        }
    }

    #[tokio::test]
    async fn conflicting_user_decisions_deny_once_in_either_order() {
        for reject_first in [false, true] {
            let fixture = fixture().await;
            let call = fixture.call("shell");
            let mut waiting = Box::pin(request(&fixture.app, &fixture.ccx, &call, "decision"));
            let pending = fixture.waiting(waiting.as_mut()).await;
            let (parent, _, wrappers) = pause_wrappers(&fixture, &[&pending]).await;
            let decisions: Vec<_> = [!reject_first, reject_first, true]
                .into_iter()
                .map(|accepted| crate::chat::types::ToolDecisionItem {
                    tool_call_id: wrappers[0].id.clone(),
                    accepted,
                })
                .collect();
            crate::chat::handle_tool_decisions_guarded(
                fixture.app.clone(),
                parent.clone(),
                &decisions,
                None,
                "approval-test",
            )
            .await;
            assert_eq!(
                fixture.app.agent_approvals.wait_now(&pending.id, now_ms()),
                Ok(ApprovalDecision::Rejected)
            );
            assert!(!waiting.await.unwrap());
            assert_eq!(
                parent
                    .lock()
                    .await
                    .messages
                    .iter()
                    .filter(
                        |message| message.role == "tool" && message.tool_call_id == wrappers[0].id
                    )
                    .count(),
                1
            );
            assert!(!fixture.dir.path().join("approval.txt").exists());
        }
    }

    #[tokio::test]
    async fn restored_wrapper_has_no_implicit_user_authority() {
        let fixture = fixture().await;
        let call = fixture.call("shell");
        let mut waiting = Box::pin(request(&fixture.app, &fixture.ccx, &call, "decision"));
        let pending = fixture.waiting(waiting.as_mut()).await;
        let (parent, _, wrappers) = pause_wrappers(&fixture, &[&pending]).await;
        {
            let mut session = parent.lock().await;
            let mut restored = crate::chat::types::ChatSession::new_with_trajectory(
                "parent".into(),
                session.messages.clone(),
                session.thread.clone(),
                session.created_at.clone(),
                None,
                vec![],
                None,
            );
            assert!(restored.approval_wrapper_calls.is_empty());
            restored.runtime = session.runtime.clone();
            restored.work_project_root = Some(fixture.dir.path().into());
            restored
                .user_interrupt_flag
                .store(true, std::sync::atomic::Ordering::SeqCst);
            *session = restored;
        }
        crate::chat::handle_tool_decisions_guarded(
            fixture.app.clone(),
            parent.clone(),
            &[crate::chat::types::ToolDecisionItem {
                tool_call_id: wrappers[0].id.clone(),
                accepted: true,
            }],
            None,
            "approval-test",
        )
        .await;
        assert_eq!(
            fixture.app.agent_approvals.wait_now(&pending.id, now_ms()),
            Err(refact_work_core::ApprovalError::NotApproved)
        );
        assert!(parent
            .lock()
            .await
            .messages
            .iter()
            .any(|message| message.tool_call_id == wrappers[0].id
                && message.tool_failed == Some(true)));
        drop(waiting);
        assert_eq!(fixture.app.work_scheduler.snapshot().outstanding, 0);
        assert!(!fixture.dir.path().join("approval.txt").exists());
    }

    #[tokio::test]
    async fn board_approval_cannot_survive_reassignment_or_cancelled_ownership() {
        for reassign in [false, true] {
            let fixture = board_fixture().await;
            let child_id = fixture.ccx.lock().await.chat_id.clone();
            let call = fixture.call("shell");
            let mut request = Box::pin(super::request(&fixture.app, &fixture.ccx, &call, "test"));
            let pending = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                loop {
                    assert!(futures::poll!(request.as_mut()).is_pending());
                    if fixture.execution.state() == super::super::execution::ExecutionState::Parked
                    {
                        if let Some(pending) = fixture
                            .app
                            .agent_approvals
                            .list_for_parent("parent", now_ms())
                            .first()
                            .cloned()
                        {
                            break pending;
                        }
                    }
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            if reassign {
                crate::tasks::storage::update_board_atomic(
                    fixture.app.gcx.clone(),
                    "approval-task",
                    |board| {
                        board.get_card_mut("card").unwrap().agent_chat_id =
                            Some("replacement".into());
                        Ok(())
                    },
                )
                .await
                .unwrap();
            } else {
                fixture.execution.cancel();
            }
            assert!(super::super::approval_target::ApprovalTarget::resolve(
                &fixture.app,
                &pending.child_work_id
            )
            .await
            .is_err());
            let parent = fixture
                .app
                .chat
                .sessions
                .read()
                .await
                .get("parent")
                .cloned()
                .unwrap();
            parent.lock().await.thread.auto_approve_dangerous_commands = true;
            let context = Arc::new(Mutex::new(
                AtCommandsContext::new_from_app(
                    fixture.app.clone(),
                    1000,
                    1,
                    false,
                    vec![],
                    "parent".into(),
                    None,
                    "test/model".into(),
                    None,
                    None,
                )
                .await,
            ));
            assert!(fixture
                .app
                .tool_registry
                .execute_tool(
                    &context,
                    "agent",
                    Some("test/model"),
                    "stale-approve",
                    "agent_approval",
                    serde_json::json!({"action":"approve", "request_id":pending.id})
                        .as_object()
                        .unwrap()
                        .clone(),
                )
                .await
                .is_err());
            assert_eq!(
                fixture.app.agent_approvals.wait_now(&pending.id, now_ms()),
                Err(refact_work_core::ApprovalError::NotApproved)
            );
            drop(request);
            assert!(!fixture.dir.path().join("approval.txt").exists());
            crate::tasks::admission::end(&child_id).await;
        }
    }

    #[tokio::test]
    async fn child_control_tools_are_reachable_in_parent_modes_without_widening_buddy() {
        let fixture = fixture().await;
        for mode in ["agent", "task_planner", "task_agent"] {
            let catalog = fixture
                .app
                .tool_registry
                .acquire_tool_catalog(mode, Some("test/model"), None)
                .await;
            for name in [
                "subagent",
                "agent_approval",
                "agent_lifecycle",
                "work_overview",
                "work_inspect",
            ] {
                assert!(
                    catalog.index.tools.iter().any(|tool| tool.name == name),
                    "{mode} cannot reach {name}"
                );
            }
        }
        let catalog = fixture
            .app
            .tool_registry
            .acquire_tool_catalog("buddy", Some("test/model"), None)
            .await;
        for name in ["subagent", "agent_approval", "agent_lifecycle"] {
            assert!(
                !catalog.index.tools.iter().any(|tool| tool.name == name),
                "unexpected {name} authority in buddy"
            );
        }
    }

    #[tokio::test]
    async fn auto_approve_wrapper_requires_current_consent_and_cannot_expand_tool_authority() {
        let fixture = fixture().await;
        let call = fixture.call("shell");
        let mut future = Box::pin(request(&fixture.app, &fixture.ccx, &call, "needs decision"));
        let pending = fixture.waiting(future.as_mut()).await;
        let parent_ccx = Arc::new(Mutex::new(
            AtCommandsContext::new_from_app(
                fixture.app.clone(),
                4096,
                10,
                false,
                vec![],
                "parent".into(),
                None,
                "test/model".into(),
                None,
                None,
            )
            .await,
        ));
        let args = serde_json::json!({"action":"approve", "request_id":pending.id})
            .as_object()
            .unwrap()
            .clone();
        let wrapper: ChatToolCall = serde_json::from_value(serde_json::json!({
            "id":"wrapper", "type":"function", "function":{
                "name":"agent_approval", "arguments":serde_json::to_string(&args).unwrap()
            }
        }))
        .unwrap();
        let parent_session = fixture
            .app
            .chat
            .sessions
            .read()
            .await
            .get("parent")
            .cloned()
            .unwrap();
        let thread = parent_session.lock().await.thread.clone();
        let (asks, denials) = crate::chat::tools::check_tools_confirmation(
            fixture.app.clone(),
            &[wrapper.clone()],
            &[],
            &thread,
            None,
            "agent",
            Some("test/model"),
            &[],
            "test",
        )
        .await;
        assert!(denials.is_empty(), "{denials:?}");
        assert_eq!(asks.len(), 1);
        assert_eq!(asks[0].tool_call_id, "wrapper");
        let error = match fixture
            .app
            .tool_registry
            .execute_tool(
                &parent_ccx,
                "agent",
                Some("test/model"),
                "wrapper",
                "agent_approval",
                args.clone(),
            )
            .await
        {
            Err(error) => error,
            Ok(None) => panic!("agent_approval is absent from the public agent catalog"),
            Ok(Some(result)) => panic!(
                "agent_approval executed without a user grant: {:?}",
                result
                    .messages
                    .iter()
                    .map(|message| message.content.content_text_only())
                    .collect::<Vec<_>>()
            ),
        };
        assert!(error.contains("explicit user approval"), "{error}");
        assert_eq!(
            fixture.app.agent_approvals.wait_now(&pending.id, now_ms()),
            Err(refact_work_core::ApprovalError::NotApproved)
        );
        parent_session
            .lock()
            .await
            .thread
            .auto_approve_dangerous_commands = true;
        let thread = parent_session.lock().await.thread.clone();
        let mut protected = vec![wrapper];
        for (name, args) in [
            (
                "agent_lifecycle",
                serde_json::json!({"agent_id": fixture.agent_id, "action": "close"}),
            ),
            (
                "pause",
                serde_json::json!({"target":{"kind":"child", "agent_id":fixture.agent_id}, "expected_run_generation":fixture.generation}),
            ),
            (
                "resume",
                serde_json::json!({"target":{"kind":"child", "agent_id":fixture.agent_id}, "expected_run_generation":fixture.generation}),
            ),
        ] {
            protected.push(serde_json::from_value(serde_json::json!({
                "id":name, "type":"function", "function":{"name":name, "arguments":args.to_string()}
            })).unwrap());
        }
        let (asks, denials) = crate::chat::tools::check_tools_confirmation(
            fixture.app.clone(),
            &protected,
            &[],
            &thread,
            None,
            "agent",
            Some("test/model"),
            &[],
            "test",
        )
        .await;
        assert!(asks.is_empty(), "{asks:?}");
        assert!(denials.is_empty(), "{denials:?}");
        let mut foreign = crate::chat::types::ChatSession::new("foreign".into());
        foreign.thread.auto_approve_dangerous_commands = true;
        fixture
            .app
            .chat
            .sessions
            .write()
            .await
            .insert("foreign".into(), Arc::new(Mutex::new(foreign)));
        parent_ccx.lock().await.chat_id = "foreign".into();
        assert!(fixture
            .app
            .tool_registry
            .execute_tool(
                &parent_ccx,
                "agent",
                Some("test/model"),
                "wrapper",
                "agent_approval",
                args.clone(),
            )
            .await
            .err()
            .unwrap()
            .contains("NotOwner"));
        parent_ccx.lock().await.chat_id = "parent".into();
        parent_session
            .lock()
            .await
            .thread
            .auto_approve_dangerous_commands = false;
        assert!(fixture
            .app
            .tool_registry
            .execute_tool(
                &parent_ccx,
                "agent",
                Some("test/model"),
                "wrapper",
                "agent_approval",
                args.clone(),
            )
            .await
            .err()
            .unwrap()
            .contains("explicit user approval"));
        parent_session
            .lock()
            .await
            .thread
            .auto_approve_dangerous_commands = true;
        let parent = fixture
            .app
            .chat
            .sessions
            .read()
            .await
            .get("parent")
            .cloned()
            .unwrap();
        parent.lock().await.active_command.allowed_tools = vec!["cat".into()];
        assert!(fixture
            .app
            .tool_registry
            .execute_tool(
                &parent_ccx,
                "agent",
                Some("test/model"),
                "wrapper",
                "agent_approval",
                args.clone()
            )
            .await
            .err()
            .unwrap()
            .contains("not authorized"));
        parent.lock().await.active_command.allowed_tools.clear();
        let approved = fixture
            .app
            .tool_registry
            .execute_tool(
                &parent_ccx,
                "agent",
                Some("test/model"),
                "wrapper",
                "agent_approval",
                args.clone(),
            )
            .await
            .unwrap()
            .unwrap();
        let response: serde_json::Value =
            serde_json::from_str(&approved.messages[0].content.content_text_only()).unwrap();
        assert_eq!(response["decision"], "approved");
        assert_eq!(response["approval_source"], "parent_policy");
        assert!(future.await.unwrap());
        let repeated = fixture
            .app
            .tool_registry
            .execute_tool(
                &parent_ccx,
                "agent",
                Some("test/model"),
                "wrapper",
                "agent_approval",
                args,
            )
            .await
            .unwrap()
            .unwrap();
        assert!(repeated.messages[0]
            .content
            .content_text_only()
            .contains("already_decided"));
        for (state, expected) in [
            (ApprovalDecision::Rejected, "no longer pending"),
            (ApprovalDecision::Expired, "expired"),
        ] {
            let mut terminal = pending.clone();
            terminal.id = format!("terminal-{state:?}");
            terminal.tool_call_id = terminal.id.clone();
            terminal.created_at = now_ms();
            terminal.expires_at = terminal.created_at + 60_000;
            fixture
                .app
                .agent_approvals
                .register(terminal.clone(), now_ms())
                .unwrap();
            if state == ApprovalDecision::Rejected {
                fixture
                    .app
                    .agent_approvals
                    .decide("parent", fixture.generation, &terminal.id, false, now_ms())
                    .unwrap();
            } else {
                fixture.app.agent_approvals.expire(terminal.expires_at);
            }
            assert_eq!(
                fixture.app.agent_approvals.wait_now(&terminal.id, now_ms()),
                Ok(state)
            );
            assert!(fixture
                .app
                .tool_registry
                .execute_tool(
                    &parent_ccx,
                    "agent",
                    Some("test/model"),
                    "terminal-wrapper",
                    "agent_approval",
                    serde_json::json!({"action":"approve", "request_id":terminal.id})
                        .as_object()
                        .unwrap()
                        .clone(),
                )
                .await
                .err()
                .unwrap()
                .contains(expected));
            assert_eq!(
                fixture.app.agent_approvals.wait_now(&terminal.id, now_ms()),
                Ok(state)
            );
        }
        assert!(!fixture.dir.path().join("approval.txt").exists());
    }
}
