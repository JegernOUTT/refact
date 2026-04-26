use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;
use tokio::time::{timeout, Duration};

use crate::ext::config_dirs::CommandSource;
use crate::ext::slash_commands::SlashCommand;
use crate::global_context::GlobalContext;
use crate::integrations::mcp::session_mcp::SessionMCP;

pub const MCP_PROMPT_PREFIX: &str = "mcp_";

pub fn sanitize_name(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub fn server_name_from_session(session: &SessionMCP) -> String {
    if let Some(info) = &session.server_info {
        let name = sanitize_name(&info.server_info.name);
        if !name.is_empty() {
            return name;
        }
    }
    let path = std::path::Path::new(&session.config_path);
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("mcp");
    sanitize_name(stem)
}

pub fn mcp_prompt_command_name(server_name: &str, prompt_name: &str) -> String {
    format!(
        "{}{}_{}",
        MCP_PROMPT_PREFIX,
        server_name,
        sanitize_name(prompt_name)
    )
}

pub async fn mcp_prompts_as_slash_commands(gcx: Arc<ARwLock<GlobalContext>>) -> Vec<SlashCommand> {
    let sessions: Vec<
        Arc<tokio::sync::Mutex<Box<dyn crate::integrations::sessions::IntegrationSession>>>,
    > = {
        let gcx_locked = gcx.read().await;
        gcx_locked.integration_sessions.values().cloned().collect()
    };

    let mut result = Vec::new();
    for session_arc in sessions {
        let mut session_locked = session_arc.lock().await;
        let mcp_session = match session_locked.as_any_mut().downcast_mut::<SessionMCP>() {
            Some(s) => s,
            None => continue,
        };
        if mcp_session.mcp_prompts.is_empty() {
            continue;
        }
        let server_name = server_name_from_session(mcp_session);
        for prompt in &mcp_session.mcp_prompts {
            let cmd_name = mcp_prompt_command_name(&server_name, &prompt.name);
            let description = prompt.description.clone().unwrap_or_default();
            let argument_hint = build_argument_hint(prompt);
            result.push(SlashCommand {
                name: cmd_name,
                description,
                argument_hint,
                allowed_tools: vec![],
                model: None,
                body: String::new(),
                source: CommandSource::GlobalRefact,
                file_path: PathBuf::new(),
            });
        }
    }
    result
}

fn build_argument_hint(prompt: &rmcp::model::Prompt) -> String {
    let args = match &prompt.arguments {
        Some(a) if !a.is_empty() => a,
        _ => return String::new(),
    };
    let parts: Vec<String> = args
        .iter()
        .map(|a| {
            if a.required.unwrap_or(false) {
                format!("<{}>", a.name)
            } else {
                format!("[{}]", a.name)
            }
        })
        .collect();
    parts.join(" ")
}

pub struct McpPromptParsed {
    pub server_config_path: String,
    pub prompt_name: String,
    pub args_map: HashMap<String, String>,
}

pub async fn parse_mcp_prompt_command(
    gcx: Arc<ARwLock<GlobalContext>>,
    cmd_name: &str,
    args_str: &str,
) -> Option<McpPromptParsed> {
    if !cmd_name.starts_with(MCP_PROMPT_PREFIX) {
        return None;
    }
    let sessions: Vec<(
        String,
        Arc<tokio::sync::Mutex<Box<dyn crate::integrations::sessions::IntegrationSession>>>,
    )> = {
        let gcx_locked = gcx.read().await;
        gcx_locked
            .integration_sessions
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    };

    for (config_path, session_arc) in sessions {
        let mut session_locked = session_arc.lock().await;
        let mcp_session = match session_locked.as_any_mut().downcast_mut::<SessionMCP>() {
            Some(s) => s,
            None => continue,
        };
        let server_name = server_name_from_session(mcp_session);
        for prompt in &mcp_session.mcp_prompts {
            let expected_name = mcp_prompt_command_name(&server_name, &prompt.name);
            if expected_name == cmd_name {
                let args_map = build_args_map(prompt, args_str);
                return Some(McpPromptParsed {
                    server_config_path: config_path,
                    prompt_name: prompt.name.clone(),
                    args_map,
                });
            }
        }
    }
    None
}

fn build_args_map(prompt: &rmcp::model::Prompt, args_str: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let positional: Vec<&str> = args_str.split_whitespace().collect();
    if let Some(arguments) = &prompt.arguments {
        for (i, arg) in arguments.iter().enumerate() {
            if let Some(val) = positional.get(i) {
                map.insert(arg.name.clone(), val.to_string());
            }
        }
    }
    map
}

pub async fn execute_mcp_prompt(
    gcx: Arc<ARwLock<GlobalContext>>,
    cmd_name: &str,
    args_str: &str,
    request_timeout: u64,
) -> Result<String, String> {
    let parsed = match parse_mcp_prompt_command(gcx.clone(), cmd_name, args_str).await {
        Some(p) => p,
        None => return Err(format!("MCP prompt not found: {}", cmd_name)),
    };

    let session_arc = {
        let gcx_locked = gcx.read().await;
        gcx_locked
            .integration_sessions
            .get(&parsed.server_config_path)
            .cloned()
    };

    let session_arc = match session_arc {
        Some(s) => s,
        None => {
            return Err(format!(
                "MCP session not found: {}",
                parsed.server_config_path
            ))
        }
    };

    let client_arc = {
        let mut session_locked = session_arc.lock().await;
        let mcp_session = session_locked
            .as_any_mut()
            .downcast_mut::<SessionMCP>()
            .ok_or("not an MCP session")?;
        mcp_session.mcp_client.clone()
    };

    let client_arc = match client_arc {
        Some(c) => c,
        None => {
            return Err(format!(
                "MCP client not connected: {}",
                parsed.server_config_path
            ))
        }
    };

    let args_obj: Option<serde_json::Map<String, serde_json::Value>> = if parsed.args_map.is_empty()
    {
        None
    } else {
        Some(
            parsed
                .args_map
                .into_iter()
                .map(|(k, v)| (k, serde_json::Value::String(v)))
                .collect(),
        )
    };

    let params = if let Some(args) = args_obj {
        rmcp::model::GetPromptRequestParams::new(parsed.prompt_name).with_arguments(args)
    } else {
        rmcp::model::GetPromptRequestParams::new(parsed.prompt_name)
    };

    let peer = {
        let client_locked = client_arc.lock().await;
        match &*client_locked {
            Some(c) => c.peer().clone(),
            None => return Err("MCP client disconnected".to_string()),
        }
    }; // lock released before the network call

    let result = match timeout(
        Duration::from_secs(request_timeout),
        peer.get_prompt(params),
    )
    .await
    {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => return Err(format!("get_prompt failed: {:?}", e)),
        Err(_) => return Err(format!("get_prompt timed out after {}s", request_timeout)),
    };

    Ok(format_prompt_result(result))
}

fn format_prompt_result(result: rmcp::model::GetPromptResult) -> String {
    let mut parts = Vec::new();
    for msg in result.messages {
        let text = match &msg.content {
            rmcp::model::PromptMessageContent::Text { text } => text.clone(),
            rmcp::model::PromptMessageContent::Image { .. } => "[image]".to_string(),
            rmcp::model::PromptMessageContent::Resource { resource } => match &resource.resource {
                rmcp::model::ResourceContents::TextResourceContents { text, .. } => text.clone(),
                rmcp::model::ResourceContents::BlobResourceContents { .. } => {
                    "[blob resource]".to_string()
                }
            },
            rmcp::model::PromptMessageContent::ResourceLink { .. } => "[resource link]".to_string(),
        };
        match msg.role {
            rmcp::model::PromptMessageRole::User => parts.push(text),
            rmcp::model::PromptMessageRole::Assistant => {
                parts.push(format!("[assistant]: {}", text));
            }
        }
    }
    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_name() {
        assert_eq!(sanitize_name("hello-world"), "hello_world");
        assert_eq!(sanitize_name("my server"), "my_server");
        assert_eq!(sanitize_name("valid_name_123"), "valid_name_123");
        assert_eq!(sanitize_name("dots.and.slashes/"), "dots_and_slashes_");
    }

    #[test]
    fn test_mcp_prompt_command_name() {
        assert_eq!(
            mcp_prompt_command_name("myserver", "code_review"),
            "mcp_myserver_code_review"
        );
        assert_eq!(
            mcp_prompt_command_name("my_server", "review-code"),
            "mcp_my_server_review_code"
        );
    }

    #[test]
    fn test_build_args_map_positional() {
        let prompt = rmcp::model::Prompt::new(
            "test",
            None::<String>,
            Some(vec![
                rmcp::model::PromptArgument::new("arg1").with_required(true),
                rmcp::model::PromptArgument::new("arg2").with_required(false),
            ]),
        );
        let map = build_args_map(&prompt, "value1 value2");
        assert_eq!(map.get("arg1"), Some(&"value1".to_string()));
        assert_eq!(map.get("arg2"), Some(&"value2".to_string()));
    }

    #[test]
    fn test_build_argument_hint_required_optional() {
        let prompt = rmcp::model::Prompt::new(
            "test",
            None::<String>,
            Some(vec![
                rmcp::model::PromptArgument::new("req").with_required(true),
                rmcp::model::PromptArgument::new("opt").with_required(false),
            ]),
        );
        assert_eq!(build_argument_hint(&prompt), "<req> [opt]");
    }

    #[test]
    fn test_build_argument_hint_no_args() {
        let prompt = rmcp::model::Prompt::new("test", None::<String>, None);
        assert_eq!(build_argument_hint(&prompt), "");
    }
}
