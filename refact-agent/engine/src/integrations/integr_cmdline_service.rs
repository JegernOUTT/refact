use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::exec::{
    generate_short_description, sanitize_short_description, ExecMode, ExecOutputChunk,
    ExecOutputStream, ExecOwnerMeta, ExecReadResult, ExecReadinessProbe, ExecServiceLookup,
    ExecSpawnRequest, ExecStatus,
};
use crate::global_context::GlobalContext;
use crate::integrations::integr_abstract::{
    IntegrationCommon, IntegrationConfirmation, IntegrationTrait,
};
use crate::integrations::integr_cmdline::{format_output, replace_args, CmdlineToolConfig};
use crate::postprocessing::pp_command_output::{output_mini_postprocessing, OutputFilter};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

const SERVICE_TRANSCRIPT_MAX_BYTES: usize = 2 * 1024 * 1024;

#[derive(Default)]
pub struct ToolService {
    pub common: IntegrationCommon,
    pub name: String,
    pub cfg: CmdlineToolConfig,
    pub config_path: String,
}

#[async_trait]
impl IntegrationTrait for ToolService {
    async fn integr_settings_apply(
        &mut self,
        _gcx: Arc<GlobalContext>,
        config_path: String,
        value: &serde_json::Value,
    ) -> Result<(), serde_json::Error> {
        self.cfg = serde_json::from_value(value.clone())?;
        self.common = serde_json::from_value(value.clone())?;
        self.config_path = config_path;
        Ok(())
    }

    fn integr_settings_as_json(&self) -> serde_json::Value {
        serde_json::to_value(&self.cfg).unwrap()
    }

    fn integr_common(&self) -> IntegrationCommon {
        self.common.clone()
    }

    async fn integr_tools(&self, integr_name: &str) -> Vec<Box<dyn Tool + Send>> {
        vec![Box::new(ToolService {
            common: self.common.clone(),
            name: integr_name.to_string(),
            cfg: self.cfg.clone(),
            config_path: self.config_path.clone(),
        })]
    }

    fn integr_schema(&self) -> &str {
        CMDLINE_SERVICE_INTEGRATION_SCHEMA
    }
}

#[async_trait]
impl Tool for ToolService {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let args_str = parse_service_tool_args(args, &self.cfg)?;
        let command = replace_args(self.cfg.command.as_str(), &args_str);
        let workdir = replace_args(self.cfg.command_workdir.as_str(), &args_str);
        let action = args_str
            .get("action")
            .cloned()
            .unwrap_or_else(|| "start".to_string());
        if !["start", "restart", "stop", "status"].contains(&action.as_str()) {
            return Err("Tool call is invalid. Param 'action' must be one of 'start', 'restart', 'stop', 'status'. Try again".to_string());
        }
        let (gcx, exec_registry, chat_id) = {
            let ccx_lock = ccx.lock().await;
            (
                ccx_lock.app.gcx.clone(),
                ccx_lock.app.runtime.exec_registry.clone(),
                ccx_lock.chat_id.clone(),
            )
        };
        let workspace = crate::files_correction::get_active_project_path(gcx.clone()).await;
        let output = match action.as_str() {
            "start" => {
                start_service(
                    gcx,
                    &exec_registry,
                    &self.name,
                    tool_call_id,
                    &chat_id,
                    workspace,
                    &command,
                    &workdir,
                    &self.cfg,
                )
                .await?
            }
            "restart" => {
                if let Some(snapshot) =
                    find_active_service(&exec_registry, &self.name, &chat_id, workspace.as_ref())
                        .await
                {
                    let _ = exec_registry.kill(&snapshot.meta.process_id).await?;
                }
                start_service(
                    gcx,
                    &exec_registry,
                    &self.name,
                    tool_call_id,
                    &chat_id,
                    workspace,
                    &command,
                    &workdir,
                    &self.cfg,
                )
                .await?
            }
            "stop" => {
                let snapshot =
                    find_active_service(&exec_registry, &self.name, &chat_id, workspace.as_ref())
                        .await
                        .ok_or_else(|| format!("Service '{}' is not running", self.name))?;
                let snapshot = exec_registry.kill(&snapshot.meta.process_id).await?;
                format_service_snapshot("Service stopped", &snapshot)
            }
            "status" => {
                let snapshot =
                    find_service(&exec_registry, &self.name, &chat_id, workspace.as_ref())
                        .await
                        .ok_or_else(|| format!("Service '{}' is not running", self.name))?;
                let read = exec_registry.read(&snapshot.meta.process_id, 0, None).await;
                let mut output = format_service_snapshot("Service status", &snapshot);
                output.push_str(&format_service_logs(&read, &self.cfg.output_filter));
                output
            }
            _ => return Err(format!("Unknown action: {action}")),
        };
        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(output),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                output_filter: Some(OutputFilter::no_limits()),
                ..Default::default()
            })],
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }

    fn tool_description(&self) -> ToolDesc {
        let required: Vec<String> = self
            .cfg
            .parameters_required
            .clone()
            .unwrap_or_else(|| self.cfg.parameters.iter().map(|p| p.name.clone()).collect());
        let mut properties = serde_json::Map::new();
        for p in &self.cfg.parameters {
            properties.insert(
                p.name.clone(),
                json!({
                    "type": p.param_type,
                    "description": p.description
                }),
            );
        }
        properties.insert(
            "action".to_string(),
            json!({
                "type": "string",
                "description": "Action to perform: start, restart, stop, status"
            }),
        );
        let input_schema = json!({
            "type": "object",
            "properties": properties,
            "required": required
        });
        ToolDesc {
            name: self.name.clone(),
            display_name: self.name.clone(),
            source: ToolSource {
                source_type: ToolSourceType::Integration,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: self.cfg.description.clone(),
            input_schema,
            output_schema: None,
            annotations: None,
        }
    }

    async fn command_to_match_against_confirm_deny(
        &self,
        _ccx: Arc<AMutex<AtCommandsContext>>,
        args: &HashMap<String, Value>,
    ) -> Result<String, String> {
        let args_str = parse_service_tool_args(args, &self.cfg)?;
        Ok(replace_args(self.cfg.command.as_str(), &args_str))
    }

    fn confirm_deny_rules(&self) -> Option<IntegrationConfirmation> {
        Some(self.integr_common().confirmation)
    }

    fn has_config_path(&self) -> Option<String> {
        Some(self.config_path.clone())
    }
}

fn parse_service_tool_args(
    args: &HashMap<String, Value>,
    cfg: &CmdlineToolConfig,
) -> Result<HashMap<String, String>, String> {
    let mut args_str = HashMap::new();
    for (key, value) in args {
        match value {
            Value::String(s) => {
                args_str.insert(key.clone(), s.clone());
            }
            _ => return Err(format!("argument `{key}` is not a string: {value:?}")),
        }
    }
    for param in &cfg.parameters {
        if cfg
            .parameters_required
            .as_ref()
            .map_or(false, |required| required.contains(&param.name))
            && !args_str.contains_key(&param.name)
        {
            return Err(format!("Missing required argument `{}`", param.name));
        }
    }
    Ok(args_str)
}

async fn start_service(
    gcx: Arc<GlobalContext>,
    exec_registry: &crate::exec::ExecRegistry,
    service_name: &str,
    tool_call_id: &str,
    chat_id: &str,
    workspace: Option<PathBuf>,
    command: &str,
    workdir: &str,
    cfg: &CmdlineToolConfig,
) -> Result<String, String> {
    if let Some(snapshot) =
        find_active_service(exec_registry, service_name, chat_id, workspace.as_ref()).await
    {
        return Err(format!(
            "Service '{}' is already running as {}. Use stop or process_kill first.",
            service_name, snapshot.meta.process_id
        ));
    }
    let mut error_log = Vec::new();
    let env_variables = crate::integrations::setting_up_integrations::get_vars_for_replacements(
        gcx.clone(),
        &mut error_log,
    )
    .await;
    let resolved_workdir = resolve_workdir(gcx, workdir).await?;
    let owner = ExecOwnerMeta {
        chat_id: Some(chat_id.to_string()),
        tool_call_id: Some(tool_call_id.to_string()),
        service_name: Some(service_name.to_string()),
        workspace,
    };
    let short_description = sanitize_short_description(&format!(
        "service {}: {}",
        service_name,
        generate_short_description(command, &ExecMode::Service)
    ));
    let mut request = ExecSpawnRequest::service(command.to_string())
        .with_owner(owner)
        .with_env_map(env_variables)
        .with_startup_wait(Duration::from_secs(cfg.startup_wait))
        .with_transcript_limit(SERVICE_TRANSCRIPT_MAX_BYTES)
        .with_short_description(short_description);
    if let Some(workdir) = resolved_workdir {
        request = request.with_cwd(workdir);
    }
    if !cfg.startup_wait_keyword.is_empty() || cfg.startup_wait_port.is_some() {
        request = request.with_readiness(ExecReadinessProbe {
            wait_keyword: if cfg.startup_wait_keyword.is_empty() {
                None
            } else {
                Some(cfg.startup_wait_keyword.clone())
            },
            wait_port: cfg.startup_wait_port,
        });
    }
    let result = exec_registry.spawn(request).await?;
    let read = exec_registry
        .read(&result.snapshot.meta.process_id, 0, None)
        .await;
    let mut output = format_service_snapshot("Service started", &result.snapshot);
    output.push_str(&format_service_logs(&read, &cfg.output_filter));
    Ok(output)
}

async fn resolve_workdir(
    gcx: Arc<GlobalContext>,
    workdir: &str,
) -> Result<Option<PathBuf>, String> {
    if workdir.trim().is_empty() {
        return Ok(crate::files_correction::get_active_project_path(gcx).await);
    }
    let path = PathBuf::from(workdir);
    let resolved = if path.is_absolute() {
        path
    } else {
        let project_dirs = crate::files_correction::get_project_dirs(gcx).await;
        let first_dir = project_dirs
            .first()
            .ok_or_else(|| "No project directory found".to_string())?;
        first_dir.join(path)
    };
    if resolved.exists() {
        Ok(Some(resolved))
    } else {
        Err(format!("Workdir '{}' does not exist", resolved.display()))
    }
}

async fn find_service(
    exec_registry: &crate::exec::ExecRegistry,
    service_name: &str,
    chat_id: &str,
    workspace: Option<&PathBuf>,
) -> Option<crate::exec::ExecProcessSnapshot> {
    let mut lookup = ExecServiceLookup::new(service_name.to_string());
    if !chat_id.is_empty() {
        lookup = lookup.with_chat_id(chat_id.to_string());
    }
    if let Some(workspace) = workspace.cloned() {
        lookup = lookup.with_workspace(workspace);
    }
    exec_registry.find_service(lookup).await
}

async fn find_active_service(
    exec_registry: &crate::exec::ExecRegistry,
    service_name: &str,
    chat_id: &str,
    workspace: Option<&PathBuf>,
) -> Option<crate::exec::ExecProcessSnapshot> {
    find_service(exec_registry, service_name, chat_id, workspace)
        .await
        .filter(|snapshot| !snapshot.status.is_terminal())
}

fn status_label(status: &ExecStatus) -> &'static str {
    match status {
        ExecStatus::Starting => "starting",
        ExecStatus::Running => "running",
        ExecStatus::Exited { .. } => "exited",
        ExecStatus::Failed { .. } => "failed",
        ExecStatus::Killed => "killed",
        ExecStatus::TimedOut => "timed_out",
    }
}

fn exit_code(status: &ExecStatus) -> Option<i32> {
    match status {
        ExecStatus::Exited { exit_code } => *exit_code,
        ExecStatus::Starting
        | ExecStatus::Running
        | ExecStatus::Failed { .. }
        | ExecStatus::Killed
        | ExecStatus::TimedOut => None,
    }
}

fn format_service_snapshot(title: &str, snapshot: &crate::exec::ExecProcessSnapshot) -> String {
    let cwd = snapshot
        .meta
        .cwd
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "<default>".to_string());
    let service_name = snapshot
        .meta
        .owner
        .service_name
        .as_deref()
        .unwrap_or("<none>");
    let exit_code = exit_code(&snapshot.status)
        .map(|code| code.to_string())
        .unwrap_or_else(|| "<none>".to_string());
    format!(
        "{title}\nprocess_id: {}\nshort_description: {}\nstatus: {}\nmode: {}\nservice_name: {}\ncommand: {}\ncwd: {}\nexit_code: {}\n",
        snapshot.meta.process_id,
        snapshot.meta.short_description,
        status_label(&snapshot.status),
        snapshot.meta.mode,
        service_name,
        snapshot.meta.command,
        cwd,
        exit_code
    )
}

fn format_service_logs(read: &ExecReadResult, output_filter: &OutputFilter) -> String {
    let stdout = collect_stream(&read.chunks, ExecOutputStream::Stdout);
    let stderr = collect_stream(&read.chunks, ExecOutputStream::Stderr);
    let filtered_stdout = output_mini_postprocessing(output_filter, &stdout);
    let filtered_stderr = output_mini_postprocessing(output_filter, &stderr);
    let mut output = String::new();
    output.push_str("\nRecent output:\n");
    output.push_str(&format_output(&filtered_stdout, &filtered_stderr));
    output.push_str(&format!(
        "transcript: next_seq={}, latest_seq={}, current_bytes={}, dropped_bytes={}, truncated_chunks={}, is_truncated={}\n",
        read.next_seq,
        read.latest_seq,
        read.current_bytes,
        read.dropped_bytes,
        read.truncated_chunks,
        read.is_truncated
    ));
    output
}

fn collect_stream(chunks: &[ExecOutputChunk], stream: ExecOutputStream) -> String {
    chunks
        .iter()
        .filter(|chunk| chunk.stream == stream)
        .map(|chunk| chunk.text.as_str())
        .collect::<String>()
}

pub const CMDLINE_SERVICE_INTEGRATION_SCHEMA: &str = r#"
fields:
  command:
    f_type: string_long
    f_desc: "The command to execute."
    f_placeholder: "echo Hello World"
  command_workdir:
    f_type: string_long
    f_desc: "The working directory for the command."
    f_placeholder: "/path/to/workdir"
  description:
    f_type: string_long
    f_desc: "The model will see this description, why the model should call this?"
  parameters:
    f_type: "tool_parameters"
    f_desc: "The model will fill in those parameters."
  startup_wait_port:
    f_type: string_short
    f_desc: "Wait for TCP to become occupied during startup."
    f_placeholder: "8080"
  startup_wait:
    f_type: string_short
    f_desc: "Max time to wait for service to start."
    f_default: "10"
  startup_wait_keyword:
    f_type: string
    f_desc: "Wait until a keyword appears in stdout or stderr at startup."
    f_placeholder: "Ready"
description: |
  As opposed to command line argumenets

  There you can adapt any command line tool for use by AI model. You can give the model instructions why to call it, which parameters to provide,
  set a timeout and restrict the output. If you want a tool that runs in the background such as a web server, use service_* instead.
available:
  on_your_laptop_possible: true
  when_isolated_possible: true
confirmation:
  ask_user_default: ["*"]
  deny_default: ["sudo*"]
smartlinks:
  - sl_label: "Test"
    sl_chat:
      - role: "user"
        content: |
          🔧 Test the tool that corresponds to %CURRENT_CONFIG%
          If the tool isn't available or doesn't work, go through the usual plan in the system prompt. If it works express happiness, and change nothing.
    sl_enable_only_with_tool: true
  - sl_label: "Auto Configure"
    sl_chat:
      - role: "user"
        content: |
          🔧 Please write %CURRENT_CONFIG% based on what you see in the project. Follow the plan in the system prompt. Remember that service_ tools
          are only suitable for blocking command line commands that run until you hit Ctrl+C, like web servers or `tail -f`.
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::AppState;
    use crate::integrations::integr_cmdline::CmdlineParam;

    fn args(entries: Vec<(&str, Value)>) -> HashMap<String, Value> {
        entries
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect()
    }

    async fn test_ccx() -> (Arc<GlobalContext>, Arc<AMutex<AtCommandsContext>>) {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let ccx = AtCommandsContext::new_with_abort(
            AppState::from_gcx(gcx.clone()).await,
            4096,
            20,
            false,
            Vec::new(),
            "chat".to_string(),
            None,
            "model".to_string(),
            None,
            None,
            None,
        )
        .await;
        (gcx, Arc::new(AMutex::new(ccx)))
    }

    fn long_running_command(output: &str) -> String {
        if cfg!(target_os = "windows") {
            format!("[Console]::Out.Write('{output}'); Start-Sleep -Seconds 30")
        } else {
            format!("printf {output:?}; sleep 30")
        }
    }

    fn only_message(messages: Vec<ContextEnum>) -> ChatMessage {
        match messages.into_iter().next().unwrap() {
            ContextEnum::ChatMessage(message) => message,
            ContextEnum::ContextFile(_) => panic!("expected chat message"),
        }
    }

    fn text(message: &ChatMessage) -> String {
        match &message.content {
            ChatContent::SimpleText(text) => text.clone(),
            _ => String::new(),
        }
    }

    #[tokio::test]
    async fn custom_service_uses_exec_registry_not_integration_sessions() {
        let (gcx, ccx) = test_ccx().await;
        let mut tool = ToolService {
            name: "service_api".to_string(),
            cfg: CmdlineToolConfig {
                command: long_running_command("custom-ready"),
                command_workdir: String::new(),
                startup_wait: 1,
                parameters: Vec::new(),
                ..CmdlineToolConfig::default()
            },
            ..ToolService::default()
        };
        let (_, messages) = tool
            .tool_execute(ccx, &"tool_call".to_string(), &args(vec![]))
            .await
            .unwrap();
        let message = only_message(messages);
        assert!(text(&message).contains("exec_service_service_api"));
        assert!(gcx.integration_sessions.lock().await.is_empty());
        let snapshot = gcx
            .exec_registry
            .find_service(ExecServiceLookup::new("service_api").with_chat_id("chat"))
            .await
            .unwrap();
        let snapshot = gcx
            .exec_registry
            .kill(&snapshot.meta.process_id)
            .await
            .unwrap();
        assert_eq!(snapshot.status, ExecStatus::Killed);
    }

    #[test]
    fn custom_service_args_still_validate_required_params() {
        let cfg = CmdlineToolConfig {
            parameters: vec![CmdlineParam {
                name: "target".to_string(),
                param_type: "string".to_string(),
                description: String::new(),
            }],
            parameters_required: Some(vec!["target".to_string()]),
            ..CmdlineToolConfig::default()
        };
        let err = parse_service_tool_args(&args(vec![]), &cfg).unwrap_err();
        assert_eq!(err, "Missing required argument `target`");
    }
}
