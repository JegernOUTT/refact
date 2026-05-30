use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::exec::types::normalize_workspace_path;
use crate::exec::{ExecMode, ExecProcessFilter, ExecProcessSnapshot, ExecStatusKind};
use crate::files_correction::get_active_project_path;
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::tools::file_edit::auxiliary::active_execution_scope;
use crate::integrations::integr_abstract::IntegrationConfirmation;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};
use crate::worktrees::scope::ExecutionScope;

pub struct ToolCleanBackgroundProcesses {
    pub config_path: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CleanScope {
    Chat,
    Workspace,
    All,
}

#[async_trait]
impl Tool for ToolCleanBackgroundProcesses {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let scope = parse_scope(args)?;
        let include_services = parse_include_services(args)?;
        let (gcx, exec_registry, execution_scope, chat_id, task_role) = {
            let ccx = ccx.lock().await;
            (
                ccx.app.gcx.clone(),
                ccx.app.runtime.exec_registry.clone(),
                ccx.execution_scope.clone(),
                ccx.chat_id.clone(),
                ccx.task_meta.as_ref().map(|meta| meta.role.clone()),
            )
        };
        reject_unauthorized_sensitive_cleanup(scope, include_services, task_role.as_deref())?;
        let workspace = if scope == CleanScope::Workspace {
            Some(current_workspace(gcx, execution_scope.as_ref()).await?)
        } else {
            None
        };
        let base_filter = scoped_filter(scope, &chat_id, tool_call_id, workspace);
        let mut killed = Vec::new();
        for mode in target_modes(include_services) {
            for status in [ExecStatusKind::Starting, ExecStatusKind::Running] {
                let mut filter = base_filter.clone();
                filter.mode = Some(mode.clone());
                filter.status = Some(status);
                killed.extend(exec_registry.remove_by_owner(filter).await?);
            }
        }
        killed.sort_by(|a, b| a.meta.process_id.as_str().cmp(b.meta.process_id.as_str()));
        let body = json!({
            "killed_count": killed.len(),
            "killed": killed.iter().map(killed_value).collect::<Vec<_>>(),
        });
        let mut extra = serde_json::Map::new();
        extra.insert("clean_background_processes".to_string(), body.clone());

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(body.to_string()),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                tool_failed: Some(false),
                output_filter: Some(OutputFilter::no_limits()),
                extra,
                ..Default::default()
            })],
        ))
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "clean_background_processes".to_string(),
            display_name: "Clean Background Processes".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Kill and reap all non-terminal background processes owned by the current chat. Use to clean up after experiments. `scope=chat` (default) is available in normal chats. `scope=workspace`, `scope=all`, and `include_services=true` can execute only from planner/admin context and cannot be used by normal agents.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "scope": {
                        "type": "string",
                        "enum": ["chat", "owner", "workspace", "all"],
                        "default": "chat",
                        "description": "Which set of processes to target. `chat` (default) kills processes owned by the current chat — available in normal chats. `owner` is a compatibility alias for `chat`. `workspace` kills processes in the active workspace and can execute only from planner/admin context. `all` kills every process globally and can execute only from planner/admin context."
                    },
                    "include_services": {
                        "type": "boolean",
                        "default": false,
                        "description": "Also kill Service-mode processes. Can execute only from planner/admin context; normal agents must leave this false."
                    }
                }
            }),
            output_schema: None,
            annotations: None,
        }
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }

    async fn command_to_match_against_confirm_deny(
        &self,
        _ccx: Arc<AMutex<AtCommandsContext>>,
        args: &HashMap<String, Value>,
    ) -> Result<String, String> {
        let scope = parse_scope(args)?;
        let include_services = parse_include_services(args)?;
        if scope == CleanScope::All || include_services {
            Ok(format!(
                "clean_background_processes scope={} include_services={}",
                scope.as_str(),
                include_services
            ))
        } else if scope == CleanScope::Workspace {
            Ok("clean_background_processes scope=workspace".to_string())
        } else {
            Ok(String::new())
        }
    }

    fn confirm_deny_rules(&self) -> Option<IntegrationConfirmation> {
        Some(IntegrationConfirmation {
            ask_user: vec!["*".to_string()],
            deny: Vec::new(),
        })
    }

    fn has_config_path(&self) -> Option<String> {
        Some(self.config_path.clone())
    }
}

fn parse_scope(args: &HashMap<String, Value>) -> Result<CleanScope, String> {
    match args.get("scope") {
        Some(Value::String(scope)) if scope.trim().is_empty() => Ok(CleanScope::Chat),
        Some(Value::String(scope)) => match scope.trim() {
            "chat" | "owner" => Ok(CleanScope::Chat),
            "workspace" => Ok(CleanScope::Workspace),
            "all" => Ok(CleanScope::All),
            other => Err(format!(
                "Invalid scope `{other}`. Must be one of: chat, owner, workspace, all"
            )),
        },
        Some(value) => Err(format!("argument `scope` is not a string: {value:?}")),
        None => Ok(CleanScope::Chat),
    }
}

fn parse_include_services(args: &HashMap<String, Value>) -> Result<bool, String> {
    match args.get("include_services") {
        Some(Value::Bool(value)) => Ok(*value),
        Some(value) => Err(format!(
            "argument `include_services` is not a boolean: {value:?}"
        )),
        None => Ok(false),
    }
}

async fn current_workspace(
    gcx: Arc<crate::global_context::GlobalContext>,
    execution_scope: Option<&ExecutionScope>,
) -> Result<PathBuf, String> {
    if let Some(scope) = active_execution_scope(execution_scope) {
        return Ok(normalize_workspace_path(scope.effective_root()));
    }
    get_active_project_path(gcx)
        .await
        .map(|path| normalize_workspace_path(&path))
        .ok_or_else(|| "No active project for background process cleanup".to_string())
}

fn scoped_filter(
    scope: CleanScope,
    chat_id: &str,
    _tool_call_id: &str,
    workspace: Option<PathBuf>,
) -> ExecProcessFilter {
    match scope {
        CleanScope::Chat => ExecProcessFilter {
            chat_id: Some(chat_id.to_string()),
            ..ExecProcessFilter::default()
        },
        CleanScope::Workspace => ExecProcessFilter {
            workspace,
            ..ExecProcessFilter::default()
        },
        CleanScope::All => ExecProcessFilter::default(),
    }
}

impl CleanScope {
    fn as_str(self) -> &'static str {
        match self {
            CleanScope::Chat => "chat",
            CleanScope::Workspace => "workspace",
            CleanScope::All => "all",
        }
    }
}

fn reject_unauthorized_sensitive_cleanup(
    scope: CleanScope,
    include_services: bool,
    task_role: Option<&str>,
) -> Result<(), String> {
    if is_sensitive_cleanup(scope, include_services) && !is_cleanup_admin_role(task_role) {
        return Err(
            "Workspace cleanup, global cleanup, and service cleanup require planner/admin context"
                .to_string(),
        );
    }
    Ok(())
}

fn is_sensitive_cleanup(scope: CleanScope, include_services: bool) -> bool {
    scope == CleanScope::Workspace || scope == CleanScope::All || include_services
}

fn is_cleanup_admin_role(task_role: Option<&str>) -> bool {
    matches!(task_role, Some("planner") | Some("admin"))
}

fn target_modes(include_services: bool) -> Vec<ExecMode> {
    if include_services {
        vec![ExecMode::Background, ExecMode::Service]
    } else {
        vec![ExecMode::Background]
    }
}

fn killed_value(snapshot: &ExecProcessSnapshot) -> Value {
    json!({
        "process_id": snapshot.meta.process_id.as_str(),
        "short_description": snapshot.meta.short_description.as_str(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::AppState;
    use crate::exec::types::DEFAULT_EXEC_OUTPUT_LIMIT_BYTES;
    use crate::chat::types::TaskMeta;
    use crate::exec::{ExecOwnerMeta, ExecProcessId, ExecProcessMeta};
    use crate::tools::tool_shell::ToolShell;
    use crate::tools::tools_description::{MatchConfirmDenyResult, Tool};

    async fn test_ccx(
        chat_id: &str,
    ) -> (
        Arc<crate::global_context::GlobalContext>,
        Arc<AMutex<AtCommandsContext>>,
    ) {
        test_ccx_with_task_role(chat_id, None).await
    }

    async fn test_ccx_with_task_role(
        chat_id: &str,
        task_role: Option<&str>,
    ) -> (
        Arc<crate::global_context::GlobalContext>,
        Arc<AMutex<AtCommandsContext>>,
    ) {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let ccx = AtCommandsContext::new_with_abort(
            AppState::from_gcx(gcx.clone()).await,
            4096,
            20,
            false,
            Vec::new(),
            chat_id.to_string(),
            None,
            "model".to_string(),
            task_role.map(task_meta),
            None,
            None,
        )
        .await;
        (gcx, Arc::new(AMutex::new(ccx)))
    }

    fn task_meta(role: &str) -> TaskMeta {
        TaskMeta {
            task_id: "task".to_string(),
            role: role.to_string(),
            agent_id: None,
            card_id: None,
            planner_chat_id: None,
        }
    }

    async fn register_running(
        gcx: &crate::global_context::GlobalContext,
        process_id: &str,
        mode: ExecMode,
        chat_id: &str,
        short_description: &str,
    ) -> ExecProcessId {
        let snapshot = gcx
            .exec_registry
            .register(
                ExecProcessMeta::new(mode, "test command".to_string())
                    .with_process_id(ExecProcessId(process_id.to_string()))
                    .with_owner(ExecOwnerMeta {
                        chat_id: Some(chat_id.to_string()),
                        tool_call_id: Some("owner-call".to_string()),
                        ..ExecOwnerMeta::default()
                    })
                    .with_short_description(short_description.to_string()),
                DEFAULT_EXEC_OUTPUT_LIMIT_BYTES,
            )
            .await;
        gcx.exec_registry
            .mark_started(&snapshot.meta.process_id)
            .await
            .unwrap();
        snapshot.meta.process_id
    }

    async fn register_running_in_workspace(
        gcx: &crate::global_context::GlobalContext,
        process_id: &str,
        mode: ExecMode,
        chat_id: &str,
        workspace: &std::path::Path,
        short_description: &str,
    ) -> ExecProcessId {
        let snapshot = gcx
            .exec_registry
            .register(
                ExecProcessMeta::new(mode, "test command".to_string())
                    .with_process_id(ExecProcessId(process_id.to_string()))
                    .with_owner(ExecOwnerMeta {
                        chat_id: Some(chat_id.to_string()),
                        tool_call_id: Some("owner-call".to_string()),
                        workspace: Some(workspace.to_path_buf()),
                        ..ExecOwnerMeta::default()
                    })
                    .with_short_description(short_description.to_string()),
                DEFAULT_EXEC_OUTPUT_LIMIT_BYTES,
            )
            .await;
        gcx.exec_registry
            .mark_started(&snapshot.meta.process_id)
            .await
            .unwrap();
        snapshot.meta.process_id
    }

    async fn run_tool(
        ccx: Arc<AMutex<AtCommandsContext>>,
        args: HashMap<String, Value>,
    ) -> Result<ChatMessage, String> {
        let mut tool = ToolCleanBackgroundProcesses {
            config_path: String::new(),
        };
        let (_, messages) = tool
            .tool_execute(ccx, &"cleanup-call".to_string(), &args)
            .await?;
        match messages.into_iter().next().unwrap() {
            ContextEnum::ChatMessage(message) => Ok(message),
            ContextEnum::ContextFile(_) => panic!("expected chat message"),
        }
    }

    fn args(entries: Vec<(&str, Value)>) -> HashMap<String, Value> {
        entries
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect()
    }

    fn body(message: &ChatMessage) -> Value {
        match &message.content {
            ChatContent::SimpleText(text) => serde_json::from_str(text).unwrap(),
            _ => panic!("expected text body"),
        }
    }

    fn killed_ids(body: &Value) -> Vec<String> {
        body["killed"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["process_id"].as_str().unwrap().to_string())
            .collect()
    }

    fn process_id_from_message(message: &ChatMessage) -> ExecProcessId {
        ExecProcessId(
            message.extra["exec"]["process_id"]
                .as_str()
                .unwrap()
                .to_string(),
        )
    }

    fn background_sleep_command() -> String {
        if cfg!(target_os = "windows") {
            "Start-Sleep -Seconds 30".to_string()
        } else {
            "sleep 30".to_string()
        }
    }

    #[tokio::test]
    async fn chat_scope_kills_only_this_chat() {
        let (gcx, ccx) = test_ccx("chat-a").await;
        let killed = register_running(
            &gcx,
            "exec_chat_a_background",
            ExecMode::Background,
            "chat-a",
            "chat a process",
        )
        .await;
        let kept = register_running(
            &gcx,
            "exec_chat_b_background",
            ExecMode::Background,
            "chat-b",
            "chat b process",
        )
        .await;

        let message = run_tool(ccx, HashMap::new()).await.unwrap();
        let body = body(&message);

        assert_eq!(body["killed_count"], json!(1));
        assert_eq!(killed_ids(&body), vec![killed.as_str().to_string()]);
        assert!(gcx.exec_registry.get(&killed).await.is_none());
        assert!(gcx.exec_registry.get(&kept).await.is_some());
    }

    #[tokio::test]
    async fn services_excluded_by_default() {
        let (gcx, ccx) = test_ccx("chat").await;
        let background = register_running(
            &gcx,
            "exec_background_default",
            ExecMode::Background,
            "chat",
            "background process",
        )
        .await;
        let service = register_running(
            &gcx,
            "exec_service_default",
            ExecMode::Service,
            "chat",
            "service process",
        )
        .await;

        let message = run_tool(ccx, HashMap::new()).await.unwrap();
        let body = body(&message);

        assert_eq!(body["killed_count"], json!(1));
        assert_eq!(killed_ids(&body), vec![background.as_str().to_string()]);
        assert!(gcx.exec_registry.get(&background).await.is_none());
        assert!(gcx.exec_registry.get(&service).await.is_some());
    }

    #[tokio::test]
    async fn owner_scope_aliases_chat_scope() {
        let (gcx, ccx) = test_ccx("chat-a").await;
        let killed = register_running(
            &gcx,
            "exec_owner_alias_chat_a",
            ExecMode::Background,
            "chat-a",
            "chat a process",
        )
        .await;
        let kept = register_running(
            &gcx,
            "exec_owner_alias_chat_b",
            ExecMode::Background,
            "chat-b",
            "chat b process",
        )
        .await;

        let message = run_tool(ccx, args(vec![("scope", json!("owner"))]))
            .await
            .unwrap();
        let body = body(&message);

        assert_eq!(body["killed_count"], json!(1));
        assert_eq!(killed_ids(&body), vec![killed.as_str().to_string()]);
        assert!(gcx.exec_registry.get(&killed).await.is_none());
        assert!(gcx.exec_registry.get(&kept).await.is_some());
    }

    #[tokio::test]
    async fn shell_background_workspace_cleanup_kills_process() {
        let workspace = tempfile::tempdir().unwrap();
        let (gcx, ccx) = test_ccx_with_task_role("chat", Some("planner")).await;
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![workspace.path().to_path_buf()];
        let mut shell = ToolShell::default();
        let (_, messages) = shell
            .tool_execute(
                ccx.clone(),
                &"shell-call".to_string(),
                &args(vec![
                    ("command", json!(background_sleep_command())),
                    ("description", json!("Run background sleep for cleanup")),
                    ("run_in_background", json!(true)),
                ]),
            )
            .await
            .unwrap();
        let message = match messages.into_iter().next().unwrap() {
            ContextEnum::ChatMessage(message) => message,
            ContextEnum::ContextFile(_) => panic!("expected chat message"),
        };
        let process_id = process_id_from_message(&message);
        assert_eq!(
            gcx.exec_registry
                .get(&process_id)
                .await
                .unwrap()
                .meta
                .owner
                .workspace,
            Some(normalize_workspace_path(workspace.path()))
        );

        let cleanup = run_tool(ccx, args(vec![("scope", json!("workspace"))]))
            .await
            .unwrap();
        let body = body(&cleanup);

        assert_eq!(body["killed_count"], json!(1));
        assert_eq!(killed_ids(&body), vec![process_id.as_str().to_string()]);
        assert!(gcx.exec_registry.get(&process_id).await.is_none());
    }

    #[tokio::test]
    async fn workspace_scope_requires_confirmation() {
        let (_gcx, ccx) = test_ccx("chat").await;
        let tool = ToolCleanBackgroundProcesses {
            config_path: String::new(),
        };

        let workspace_result = tool
            .match_against_confirm_deny(ccx.clone(), &args(vec![("scope", json!("workspace"))]))
            .await
            .unwrap();
        let chat_result = tool
            .match_against_confirm_deny(ccx, &HashMap::new())
            .await
            .unwrap();

        assert_eq!(
            workspace_result.result,
            MatchConfirmDenyResult::CONFIRMATION
        );
        assert_eq!(chat_result.result, MatchConfirmDenyResult::PASS);
    }

    #[tokio::test]
    async fn global_and_service_cleanup_require_confirmation() {
        let (_gcx, ccx) = test_ccx("chat").await;
        let tool = ToolCleanBackgroundProcesses {
            config_path: String::new(),
        };

        let include_services = tool
            .match_against_confirm_deny(ccx.clone(), &args(vec![("include_services", json!(true))]))
            .await
            .unwrap();
        let all_scope = tool
            .match_against_confirm_deny(ccx.clone(), &args(vec![("scope", json!("all"))]))
            .await
            .unwrap();
        let chat_scope = tool
            .match_against_confirm_deny(ccx, &HashMap::new())
            .await
            .unwrap();

        assert_eq!(
            include_services.result,
            MatchConfirmDenyResult::CONFIRMATION
        );
        assert_eq!(all_scope.result, MatchConfirmDenyResult::CONFIRMATION);
        assert_eq!(chat_scope.result, MatchConfirmDenyResult::PASS);
    }

    #[tokio::test]
    async fn global_and_service_cleanup_rejected_without_planner_context() {
        let (_gcx, ccx) = test_ccx("chat").await;
        let include_services_err =
            run_tool(ccx.clone(), args(vec![("include_services", json!(true))]))
                .await
                .unwrap_err();
        let all_scope_err = run_tool(ccx, args(vec![("scope", json!("all"))]))
            .await
            .unwrap_err();

        assert!(include_services_err.contains("planner/admin context"));
        assert!(all_scope_err.contains("planner/admin context"));
    }

    #[tokio::test]
    async fn workspace_cleanup_rejected_without_planner_context() {
        let (_gcx, ccx) = test_ccx("chat").await;
        let err = run_tool(ccx, args(vec![("scope", json!("workspace"))]))
            .await
            .unwrap_err();

        assert!(err.contains("planner/admin context"));
    }

    #[tokio::test]
    async fn chat_cleanup_allowed_without_planner_context() {
        let (gcx, ccx) = test_ccx("chat-normal").await;
        let killed = register_running(
            &gcx,
            "exec_chat_cleanup_allowed",
            ExecMode::Background,
            "chat-normal",
            "normal chat process",
        )
        .await;

        let message = run_tool(ccx, HashMap::new()).await.unwrap();
        let body = body(&message);

        assert_eq!(body["killed_count"], json!(1));
        assert_eq!(killed_ids(&body), vec![killed.as_str().to_string()]);
        assert!(gcx.exec_registry.get(&killed).await.is_none());
    }

    #[tokio::test]
    async fn planner_workspace_cleanup_allowed() {
        let workspace = tempfile::tempdir().unwrap();
        let (gcx, ccx) = test_ccx_with_task_role("planner-chat", Some("planner")).await;
        *gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![workspace.path().to_path_buf()];
        let killed = register_running_in_workspace(
            &gcx,
            "exec_planner_workspace_cleanup_allowed",
            ExecMode::Background,
            "other-chat",
            workspace.path(),
            "other chat process",
        )
        .await;

        let message = run_tool(ccx, args(vec![("scope", json!("workspace"))]))
            .await
            .unwrap();
        let body = body(&message);

        assert_eq!(body["killed_count"], json!(1));
        assert_eq!(killed_ids(&body), vec![killed.as_str().to_string()]);
        assert!(gcx.exec_registry.get(&killed).await.is_none());
    }

    #[tokio::test]
    async fn include_services_true_kills_them() {
        let (gcx, ccx) = test_ccx_with_task_role("chat", Some("planner")).await;
        let background = register_running(
            &gcx,
            "exec_background_included",
            ExecMode::Background,
            "chat",
            "background process",
        )
        .await;
        let service = register_running(
            &gcx,
            "exec_service_included",
            ExecMode::Service,
            "chat",
            "service process",
        )
        .await;

        let message = run_tool(ccx, args(vec![("include_services", json!(true))]))
            .await
            .unwrap();
        let body = body(&message);

        assert_eq!(body["killed_count"], json!(2));
        assert_eq!(
            killed_ids(&body),
            vec![
                background.as_str().to_string(),
                service.as_str().to_string()
            ]
        );
        assert!(gcx.exec_registry.get(&background).await.is_none());
        assert!(gcx.exec_registry.get(&service).await.is_none());
    }

    #[test]
    fn clean_background_processes_description_mentions_planner_requirement() {
        let tool = ToolCleanBackgroundProcesses {
            config_path: String::new(),
        };
        let desc = tool.tool_description();
        assert!(
            desc.description.contains("planner/admin"),
            "tool description must mention planner/admin restriction: {}",
            desc.description
        );
        assert!(!desc.description.contains("confirmation"));
        let scope_desc = desc.input_schema["properties"]["scope"]["description"]
            .as_str()
            .unwrap();
        assert!(
            scope_desc.contains("planner/admin"),
            "scope description must mention planner/admin restriction: {scope_desc}"
        );
        assert!(!scope_desc.contains("confirmation"));
        let include_services_desc = desc.input_schema["properties"]["include_services"]
            ["description"]
            .as_str()
            .unwrap();
        assert!(
            include_services_desc.contains("planner/admin"),
            "include_services description must mention planner/admin restriction: {include_services_desc}"
        );
    }

    #[tokio::test]
    async fn foreground_unaffected() {
        let (gcx, ccx) = test_ccx_with_task_role("chat", Some("planner")).await;
        let background = register_running(
            &gcx,
            "exec_background_foreground_test",
            ExecMode::Background,
            "chat",
            "background process",
        )
        .await;
        let foreground = register_running(
            &gcx,
            "exec_foreground_unaffected",
            ExecMode::Foreground,
            "chat",
            "foreground process",
        )
        .await;

        let message = run_tool(ccx, args(vec![("include_services", json!(true))]))
            .await
            .unwrap();
        let body = body(&message);

        assert_eq!(body["killed_count"], json!(1));
        assert_eq!(killed_ids(&body), vec![background.as_str().to_string()]);
        assert!(gcx.exec_registry.get(&background).await.is_none());
        assert!(gcx.exec_registry.get(&foreground).await.is_some());
    }
}
