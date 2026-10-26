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
    let (mut request, checkout) = match call.function.name.as_str() {
        "shell" => crate::tools::shell_approval_request(ccx, &args).await?,
        _ => crate::tools::process_approval_request(ccx, &args).await?,
    };
    freeze(&mut request)?;
    let mut approval = descriptor(&request, reason)?;
    if let Some(checkout) = &checkout {
        checkout.revalidate(ccx, &args, &request).await?;
        checkout.bind_approval(&mut approval, &request)?;
    }
    Ok(Some(approval))
}

pub(crate) async fn enforce(
    ccx: &Arc<Mutex<AtCommandsContext>>,
    call_id: &str,
    tool_name: &str,
    args: &HashMap<String, serde_json::Value>,
    request: &mut ExecSpawnRequest,
    checkout: Option<&crate::tools::command_checkout::ValidatedCommandCheckout>,
) -> Result<(), String> {
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
        let gcx = crate::global_context::tests::make_test_gcx().await;
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
                enforce(&ccx, call, "shell", &HashMap::new(), &mut attempted, None)
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
                None
            )
            .await
            .unwrap_err()
            .contains("already been used"));
        }
    }
}
