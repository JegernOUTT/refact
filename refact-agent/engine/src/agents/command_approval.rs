use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::ChatToolCall;
use crate::exec::{ExecSpawnRequest, ExecEnvPolicy};
use refact_work_core::ExecutionApproval;

pub struct ApprovedExecution {
    pub agent_id: String,
    pub generation: u64,
    pub expires_at: u64,
    pub tool_name: String,
    pub args: serde_json::Value,
    pub execution: ExecutionApproval,
    pub consumed: bool,
}

pub fn freeze(request: &mut ExecSpawnRequest) -> Result<(), String> {
    request.env = refact_exec::build_child_env(&request.env_policy, &request.env)
        .into_iter()
        .collect();
    request.env_policy = ExecEnvPolicy::Exact;
    let cwd = request
        .cwd
        .clone()
        .unwrap_or(std::env::current_dir().map_err(|error| error.to_string())?);
    request.cwd = Some(dunce::canonicalize(cwd).map_err(|error| error.to_string())?);
    Ok(())
}

pub fn descriptor(request: &ExecSpawnRequest, reason: &str) -> Result<ExecutionApproval, String> {
    let env: std::collections::BTreeMap<_, _> = request.env.iter().collect();
    let argv = request.argv.clone().unwrap_or_else(|| {
        #[cfg(not(windows))]
        {
            vec!["sh".into(), "-c".into(), request.command.clone()]
        }
        #[cfg(windows)]
        {
            vec![
                "powershell.exe".into(),
                "-Command".into(),
                request.command.clone(),
            ]
        }
    });
    let privilege = request
        .sandbox
        .as_ref()
        .map(|sandbox| format!("{:?}", sandbox.mode))
        .unwrap_or_else(|| "unconfined".into());
    let cwd = request
        .cwd
        .as_ref()
        .ok_or("approval requires resolved working directory")?
        .to_string_lossy()
        .into_owned();
    let binding = serde_json::to_string(&serde_json::json!({
        "command":request.command, "argv":argv, "cwd":cwd, "env":env,
        "sandbox":format!("{:?}", request.sandbox), "env_policy":format!("{:?}", request.env_policy),
    })).map_err(|error| error.to_string())?;
    Ok(ExecutionApproval {
        command: request.command.clone(),
        argv,
        cwd,
        environment_keys: env.keys().map(|key| (*key).clone()).collect(),
        fingerprint: crate::file_activity::digest_of(&binding),
        privilege,
        reason: reason.into(),
    })
}

pub async fn prepare(
    ccx: &Arc<Mutex<AtCommandsContext>>,
    call: &ChatToolCall,
    reason: &str,
) -> Result<Option<ExecutionApproval>, String> {
    if !matches!(call.function.name.as_str(), "shell" | "process_start") {
        return Ok(None);
    }
    let args: HashMap<String, serde_json::Value> =
        serde_json::from_str(&call.function.arguments).map_err(|_| "invalid command arguments")?;
    let (mut request, checkout, cache) = match call.function.name.as_str() {
        "shell" => crate::tools::shell_approval_request(ccx, &args).await?,
        _ => crate::tools::process_approval_request(ccx, &args).await?,
    };
    freeze(&mut request)?;
    let mut approval = descriptor(&request, reason)?;
    if let Some(checkout) = &checkout {
        checkout.revalidate(ccx, &args, &request).await?;
        checkout.bind_approval(&mut approval, &request)?;
    }
    if let Some(cache) = &cache {
        cache.revalidate(ccx, &request).await?;
        cache.bind_approval(&mut approval);
    }
    Ok(Some(approval))
}

pub(crate) struct PendingCommandValidation {
    pending: refact_work_core::PendingApproval,
    chat_id: String,
    execution: Arc<super::execution::RunExecution>,
}

impl PendingCommandValidation {
    pub(crate) async fn check(&self, ccx: &Arc<Mutex<AtCommandsContext>>) -> Result<(), String> {
        let (app, chat_id, model, task, worktree, root) = {
            let context = ccx.lock().await;
            (
                context.app.clone(),
                context.chat_id.clone(),
                context.current_model.clone(),
                context.task_meta.clone(),
                context.execution_scope_worktree(),
                context.root_chat_id.clone(),
            )
        };
        let now = super::approval::now_ms();
        if chat_id != self.chat_id
            || now >= self.pending.expires_at
            || app
                .agent_approvals
                .get(&self.pending.parent_chat_id, &self.pending.id, now)
                .map_err(|_| "pending command approval unavailable")?
                != self.pending
            || app.agent_approvals.wait_now(&self.pending.id, now)
                != Err(refact_work_core::ApprovalError::NotApproved)
        {
            return Err("pending command approval changed or expired".into());
        }
        let target = super::approval_target::ApprovalTarget::from_context(&app, ccx).await?;
        target.check(&self.pending)?;
        let current = crate::tasks::admission::current_execution(&app, &chat_id)
            .map_err(|_| "pending command execution unavailable")?;
        if !Arc::ptr_eq(&current, &self.execution)
            || current.admission_generation() != self.pending.run_generation
            || current.is_cancelled()
            || current.holds_permit()
            || current.state() != super::execution::ExecutionState::Parked
        {
            return Err("pending command execution is not the current parked owner".into());
        }
        let session = app
            .chat
            .sessions
            .read()
            .await
            .get(&chat_id)
            .cloned()
            .ok_or("pending command session unavailable")?;
        let session = session.lock().await;
        if session.closed
            || session
                .abort_flag
                .load(std::sync::atomic::Ordering::Relaxed)
            || app
                .runtime
                .shutdown_flag
                .load(std::sync::atomic::Ordering::Relaxed)
            || session.runtime.state != crate::chat::types::SessionState::WaitingUserInput
            || session.thread.model != model
            || session.thread.task_meta != task
            || session.thread.worktree != worktree
            || session.thread.root_chat_id.as_deref().unwrap_or(&chat_id) != root
            || session.thread.parent_id.as_deref() != Some(self.pending.parent_chat_id.as_str())
        {
            return Err("pending command session is no longer waiting".into());
        }
        Ok(())
    }
}

pub(crate) async fn validate_pending(
    ccx: &Arc<Mutex<AtCommandsContext>>,
    call: &ChatToolCall,
    pending: &refact_work_core::PendingApproval,
) -> Result<Option<ExecutionApproval>, String> {
    let reason = pending
        .execution
        .as_ref()
        .map(|execution| execution.reason.as_str())
        .unwrap_or("");
    let managed_board = pending.child_work_id.starts_with("board:")
        && ccx
            .lock()
            .await
            .execution_scope
            .as_ref()
            .is_some_and(|scope| scope.is_enforced() && scope.worktree().kind == "task_agent");
    if !managed_board {
        return prepare(ccx, call, reason).await;
    }
    if !matches!(pending.tool_name.as_str(), "shell" | "process_start") {
        return Ok(None);
    }
    let (app, chat_id) = {
        let context = ccx.lock().await;
        (context.app.clone(), context.chat_id.clone())
    };
    let proof = PendingCommandValidation {
        pending: pending.clone(),
        execution: app
            .agent_executions
            .get(&chat_id)
            .ok_or("pending command execution unavailable")?,
        chat_id,
    };
    proof.check(ccx).await?;
    let args: HashMap<String, serde_json::Value> =
        serde_json::from_value(pending.args.clone()).map_err(|_| "invalid command arguments")?;
    let (mut request, checkout) = match pending.tool_name.as_str() {
        "shell" => crate::tools::shell_approval_request_without_cache(ccx, &args).await?,
        _ => crate::tools::process_approval_request_without_cache(ccx, &args).await?,
    };
    freeze(&mut request)?;
    if let Some(checkout) = &checkout {
        checkout.revalidate(ccx, &args, &request).await?;
    }
    let approval = crate::exec::command_policy::validate_pending_managed_cache(
        ccx,
        checkout.as_ref(),
        request,
        reason,
        &proof,
    )
    .await?;
    proof.check(ccx).await?;
    Ok(Some(approval))
}

pub(crate) async fn enforce(
    ccx: &Arc<Mutex<AtCommandsContext>>,
    call_id: &str,
    tool_name: &str,
    args: &HashMap<String, serde_json::Value>,
    request: &mut ExecSpawnRequest,
    checkout: Option<&crate::tools::command_checkout::ValidatedCommandCheckout>,
    cache: Option<&crate::exec::command_policy::ManagedCacheDecision>,
) -> Result<(), String> {
    let managed_task = {
        let context = ccx.lock().await;
        context
            .execution_scope
            .as_ref()
            .filter(|scope| scope.is_enforced() && scope.worktree().kind == "task_agent")
            .map(|scope| scope.worktree().clone())
    };
    if let Some(meta) = managed_task {
        crate::exec::command_policy::ensure_managed_task_execution(ccx, &meta).await?;
    }
    let (approval, app) = {
        let mut context = ccx.lock().await;
        let approval = match context.approved_executions.get_mut(call_id) {
            Some(approval) if approval.consumed => {
                return Err("command approval has already been used".into())
            }
            Some(approval) => {
                approval.consumed = true;
                if approval.tool_name != tool_name
                    || approval.args
                        != serde_json::to_value(args).map_err(|_| "invalid command arguments")?
                {
                    return Err("approved tool or arguments changed; request approval again".into());
                }
                Some((
                    approval.agent_id.clone(),
                    approval.generation,
                    approval.expires_at,
                    approval.execution.clone(),
                ))
            }
            None => None,
        };
        if approval.is_some()
            && (context
                .abort_flag
                .load(std::sync::atomic::Ordering::Relaxed)
                || context
                    .app
                    .runtime
                    .shutdown_flag
                    .load(std::sync::atomic::Ordering::Relaxed)
                || context
                    .app
                    .agent_executions
                    .get(&context.chat_id)
                    .is_some_and(|execution| execution.is_cancelled()))
        {
            return Err("command approval cancelled before execution".into());
        }
        (approval, context.app.clone())
    };
    let Some((agent_id, generation, expires_at, execution)) = approval else {
        return Ok(());
    };
    if super::approval::now_ms() >= expires_at {
        return Err("command approval expired before execution".into());
    }
    let target = super::approval_target::ApprovalTarget::resolve(&app, &agent_id).await?;
    if target.generation != generation || target.chat_id != ccx.lock().await.chat_id {
        return Err("command approval belongs to a stale child attempt".into());
    }
    freeze(request)?;
    let mut current = descriptor(request, &execution.reason)?;
    if let Some(checkout) = checkout {
        checkout.bind_approval(&mut current, request)?;
    }
    if let Some(cache) = cache {
        cache.bind_approval(&mut current);
    }
    if current != execution {
        return Err("approved command, working directory, environment or privilege changed; request approval again".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::ExecMode;
    use crate::exec::types::{ExecSandboxSpec, ExecSandboxMode};

    #[test]
    fn exact_descriptor_binds_all_execution_inputs_without_disclosing_environment_values() {
        let dir = tempfile::tempdir().unwrap();
        let mut request =
            ExecSpawnRequest::new(ExecMode::Foreground, "echo hello").with_cwd(dir.path());
        request
            .env
            .insert("TEST_SECRET".into(), "never-print-this".into());
        freeze(&mut request).unwrap();
        let accepted = descriptor(&request, "required by child").unwrap();
        let serialized = serde_json::to_string(&accepted).unwrap();
        assert!(!serialized.contains("never-print-this"));
        assert!(serialized.contains("TEST_SECRET"));
        for change in 0..4 {
            let mut changed = request.clone();
            match change {
                0 => changed.command = "echo different".into(),
                1 => {
                    changed.env.insert("TEST_SECRET".into(), "new-value".into());
                }
                2 => changed.cwd = Some(dir.path().join("different")),
                _ => {
                    changed.sandbox = Some(ExecSandboxSpec {
                        mode: ExecSandboxMode::ReadOnly,
                        ro_paths: vec![],
                        rw_paths: vec![],
                        allow_network: false,
                    })
                }
            }
            assert_ne!(accepted, descriptor(&changed, "required by child").unwrap());
        }
        assert_eq!(
            refact_exec::build_child_env(&request.env_policy, &request.env).len(),
            request.env.len()
        );
    }

    #[tokio::test]
    async fn approved_execution_rejects_mutation_and_can_never_be_consumed_twice() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            dir.path().join("cache"),
            dir.path().join("config"),
        )
        .await;
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        let (record, _, _) = app
            .agents
            .create(crate::agents::types::CreateAgentRequest {
                parent_chat_id: "parent".into(),
                parent_root_chat_id: Some("parent".into()),
                parent_tool_call_id: None,
                kind: crate::agents::types::BgAgentKind::Subagent,
                config_name: "test".into(),
                title: "test".into(),
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
        app.agents
            .mark_running(&record.agent_id, "child".into())
            .await
            .unwrap();
        let ccx = Arc::new(Mutex::new(
            AtCommandsContext::new_from_app(
                app.clone(),
                4096,
                10,
                false,
                vec![],
                "child".into(),
                None,
                "test/model".into(),
                None,
                None,
            )
            .await,
        ));
        let mut request =
            ExecSpawnRequest::new(ExecMode::Foreground, "echo hello").with_cwd(dir.path());
        request.env_policy = ExecEnvPolicy::Scrubbed {
            passthrough: vec![],
        };
        freeze(&mut request).unwrap();
        for (call, change) in [("good", false), ("changed", true)] {
            ccx.lock().await.approved_executions.insert(
                call.into(),
                ApprovedExecution {
                    agent_id: record.agent_id.clone(),
                    generation: record.run_generation,
                    expires_at: super::super::approval::now_ms() + 60_000,
                    tool_name: "shell".into(),
                    args: serde_json::json!({}),
                    execution: descriptor(&request, "reason").unwrap(),
                    consumed: false,
                },
            );
            let mut attempted = request.clone();
            if change {
                attempted.command = "echo changed".into();
            }
            assert_eq!(
                enforce(
                    &ccx,
                    call,
                    "shell",
                    &HashMap::new(),
                    &mut attempted,
                    None,
                    None
                )
                .await
                .is_ok(),
                !change
            );
            assert!(enforce(
                &ccx,
                call,
                "shell",
                &HashMap::new(),
                &mut request.clone(),
                None,
                None
            )
            .await
            .unwrap_err()
            .contains("already been used"));
        }
        #[cfg(target_os = "linux")]
        {
            let source = dir.path().join("repo");
            std::fs::create_dir_all(source.join("refact-agent/engine")).unwrap();
            std::fs::write(
                source.join("refact-agent/engine/Cargo.toml"),
                "[workspace]\n",
            )
            .unwrap();
            for args in [
                vec!["init"],
                vec!["checkout", "-b", "main"],
                vec!["add", "."],
                vec![
                    "-c",
                    "user.name=Fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                    "commit",
                    "-m",
                    "base",
                ],
            ] {
                crate::worktrees::git::run_git(&source, &args).unwrap();
            }
            let service =
                crate::worktrees::service::WorktreeService::new(app.gcx.cache_dir.clone(), source)
                    .unwrap();
            let meta = service
                .create_worktree(crate::worktrees::types::CreateWorktreeRequest {
                    branch: Some("refact/subagent/cache-approval".into()),
                    chat_id: Some("child".into()),
                    kind: Some("subagent".into()),
                    ..Default::default()
                })
                .await
                .unwrap()
                .worktree
                .meta;
            app.agents
                .set_worktree_metadata(&record.agent_id, Some(meta.id.clone()), meta.branch.clone())
                .await
                .unwrap();
            let mut config = crate::subchat::test_support::minimal_config("child", "parent");
            config.worktree = Some(meta.clone());
            app.agent_executions
                .remember_config(&record.agent_id, config);
            ccx.lock().await.execution_scope = Some(
                crate::worktrees::scope::ExecutionScope::from_worktree(&meta),
            );
            let mut session = crate::chat::types::ChatSession::new("child".into());
            session.thread.mode = "agent".into();
            session.thread.worktree = Some(meta.clone());
            app.chat
                .sessions
                .write()
                .await
                .insert("child".into(), Arc::new(Mutex::new(session)));
            let raw = ExecSpawnRequest::foreground("printf ok").with_cwd(meta.root.clone());
            let mut prepared = raw.clone();
            let cache =
                crate::exec::command_policy::prepare_managed_cache(&ccx, None, &mut prepared)
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(
                prepared.env["CARGO_TARGET_DIR"],
                meta.root
                    .join("refact-agent/engine/target")
                    .to_str()
                    .unwrap()
            );
            let mut approved = descriptor(&prepared, "managed cache").unwrap();
            cache.bind_approval(&mut approved);
            for call in ["managed-good", "managed-stale"] {
                ccx.lock().await.approved_executions.insert(
                    call.into(),
                    ApprovedExecution {
                        agent_id: record.agent_id.clone(),
                        generation: record.run_generation,
                        expires_at: super::super::approval::now_ms() + 60_000,
                        tool_name: "shell".into(),
                        args: serde_json::json!({}),
                        execution: approved.clone(),
                        consumed: false,
                    },
                );
            }
            let mut actual = raw.clone();
            let current =
                crate::exec::command_policy::prepare_managed_cache(&ccx, None, &mut actual)
                    .await
                    .unwrap()
                    .unwrap();
            enforce(
                &ccx,
                "managed-good",
                "shell",
                &HashMap::new(),
                &mut actual,
                None,
                Some(&current),
            )
            .await
            .unwrap();
            current.revalidate(&ccx, &actual).await.unwrap();
            assert_eq!(actual.env, prepared.env);
            std::fs::write(
                meta.root.join("refact-agent/engine/Cargo.toml"),
                "[workspace]\nmembers = []\n",
            )
            .unwrap();
            let mut stale = raw;
            let changed =
                crate::exec::command_policy::prepare_managed_cache(&ccx, None, &mut stale)
                    .await
                    .unwrap()
                    .unwrap();
            assert!(enforce(
                &ccx,
                "managed-stale",
                "shell",
                &HashMap::new(),
                &mut stale,
                None,
                Some(&changed)
            )
            .await
            .unwrap_err()
            .contains("changed"));
            assert!(ccx.lock().await.approved_executions["managed-stale"].consumed);
        }
    }
}
