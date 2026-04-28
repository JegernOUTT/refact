use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::RwLock as ARwLock;
use tracing::info;
use tokio::sync::Mutex as AMutex;

use crate::global_context::GlobalContext;
use super::actor::redact_sensitive;
use super::diagnostics::DiagnosticContext;
use super::types::BuddyActivity;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::tools::tools_description::Tool;

fn extract_tool_text(out: Vec<ContextEnum>, fallback: &str) -> String {
    out.into_iter()
        .find_map(|item| match item {
            ContextEnum::ChatMessage(msg) => match msg.content {
                ChatContent::SimpleText(text) => Some(text),
                _ => None,
            },
            _ => None,
        })
        .unwrap_or_else(|| fallback.to_string())
}

const RATE_LIMIT_SECS: u64 = 3600;
const DEDUP_SECS: i64 = 86400;

#[derive(Debug)]
pub struct IssueGate {
    pub has_diagnostics: bool,
    pub has_repro_context: bool,
    pub integration_configured: bool,
    pub auto_creation_enabled: bool,
    pub within_rate_limit: bool,
}

pub fn check_issue_gate(gate: &IssueGate) -> bool {
    gate.has_diagnostics
        && gate.has_repro_context
        && gate.integration_configured
        && gate.auto_creation_enabled
        && gate.within_rate_limit
}

pub fn check_manual_issue_gate(gate: &IssueGate) -> bool {
    gate.has_diagnostics && gate.integration_configured
}

fn gate_error(gate: &IssueGate, manual: bool) -> String {
    if !gate.has_diagnostics {
        return "gate blocked: no diagnostic information (need non-empty error with source file or tool name)".to_string();
    }
    if !manual && !gate.has_repro_context {
        return "gate blocked: no reproduction context (source file or tool name required)"
            .to_string();
    }
    if !gate.integration_configured {
        return "gate blocked: no issue tracker integration configured".to_string();
    }
    if !manual && !gate.auto_creation_enabled {
        return "gate blocked: automatic issue creation is disabled in settings".to_string();
    }
    if !manual && !gate.within_rate_limit {
        return "gate blocked: rate limit active (one issue per hour)".to_string();
    }
    "gate blocked: unknown condition".to_string()
}

#[derive(Debug, Clone)]
enum IssueProvider {
    GitHub { binary: String, token: String },
    GitLab { binary: String, token: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RepoHost {
    GitHub,
    GitLab,
    GitLabSelfHosted(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepoInfo {
    pub owner: String,
    pub repo: String,
    pub host: RepoHost,
}

impl RepoInfo {
    fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.repo)
    }
}

pub struct BuddyIssueCreateResult {
    pub url: String,
    pub provider: String,
    pub repo: String,
}

pub(crate) fn validate_issue_binary(configured: &str) -> Result<&'static str, String> {
    if configured.contains('/') || configured.contains('\\') {
        return Err(format!(
            "issue binary must be bare command, got path: {}",
            configured
        ));
    }
    match configured {
        "gh" => Ok("gh"),
        "glab" => Ok("glab"),
        other => Err(format!("unsupported issue binary: {}", other)),
    }
}

async fn try_read_github(config_dir: &PathBuf) -> Result<Option<IssueProvider>, String> {
    let path = config_dir.join("integrations.d").join("github.yaml");
    let content = match tokio::fs::read_to_string(&path).await {
        Ok(c) => c,
        Err(_) => return Ok(None),
    };
    let val: serde_yaml::Value = match serde_yaml::from_str(&content) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    let token = match val.get("gh_token").and_then(|v| v.as_str()) {
        Some(t) if !t.is_empty() => t.to_string(),
        _ => return Ok(None),
    };
    let configured = val
        .get("gh_binary_path")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("gh");
    let binary = validate_issue_binary(configured)?.to_string();
    Ok(Some(IssueProvider::GitHub { binary, token }))
}

async fn try_read_gitlab(config_dir: &PathBuf) -> Result<Option<IssueProvider>, String> {
    let path = config_dir.join("integrations.d").join("gitlab.yaml");
    let content = match tokio::fs::read_to_string(&path).await {
        Ok(c) => c,
        Err(_) => return Ok(None),
    };
    let val: serde_yaml::Value = match serde_yaml::from_str(&content) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    let token = match val.get("glab_token").and_then(|v| v.as_str()) {
        Some(t) if !t.is_empty() => t.to_string(),
        _ => return Ok(None),
    };
    let configured = val
        .get("glab_binary_path")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("glab");
    let binary = validate_issue_binary(configured)?.to_string();
    Ok(Some(IssueProvider::GitLab { binary, token }))
}

pub(crate) fn parse_remote_url(url: &str) -> Option<RepoInfo> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(rest) = trimmed.strip_prefix("git@") {
        let (host, path) = rest.split_once(':')?;
        return parse_remote_path(host, path);
    }

    if let Ok(parsed) = url::Url::parse(trimmed) {
        let host = parsed.host_str()?;
        let path = parsed.path().trim_start_matches('/');
        return parse_remote_path(host, path);
    }

    None
}

fn parse_remote_path(host: &str, path: &str) -> Option<RepoInfo> {
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if parts.len() < 2 {
        return None;
    }
    let owner = parts[parts.len() - 2].to_string();
    let mut repo = parts[parts.len() - 1].to_string();
    if repo.ends_with(".git") {
        repo.truncate(repo.len() - 4);
    }
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    let host = host.to_ascii_lowercase();
    let host = match host.as_str() {
        "github.com" => RepoHost::GitHub,
        "gitlab.com" => RepoHost::GitLab,
        _ => RepoHost::GitLabSelfHosted(host),
    };
    Some(RepoInfo { owner, repo, host })
}

pub(crate) fn detect_repo_from_git(project_root: &Path) -> Option<RepoInfo> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(project_root)
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    parse_remote_url(&url)
}

async fn detect_provider(
    gcx: Arc<ARwLock<GlobalContext>>,
    repo: &RepoInfo,
) -> Result<Option<IssueProvider>, String> {
    let active = crate::files_correction::get_active_project_path(gcx.clone()).await;
    let (config_dirs, global_config_dir) =
        crate::integrations::setting_up_integrations::get_config_dirs(gcx.clone(), &active).await;
    let mut search_dirs: Vec<PathBuf> = config_dirs;
    search_dirs.push(global_config_dir);

    let mut gh_provider: Option<IssueProvider> = None;
    let mut gl_provider: Option<IssueProvider> = None;

    for dir in &search_dirs {
        if gh_provider.is_none() {
            gh_provider = try_read_github(dir).await?;
        }
        if gl_provider.is_none() {
            gl_provider = try_read_gitlab(dir).await?;
        }
    }

    Ok(match &repo.host {
        RepoHost::GitHub => gh_provider,
        RepoHost::GitLab | RepoHost::GitLabSelfHosted(_) => gl_provider,
    })
}

pub async fn has_github_mcp(gcx: Arc<ARwLock<GlobalContext>>) -> bool {
    let groups = crate::tools::tools_list::get_integration_tools(gcx).await;
    groups.into_iter().any(|group| {
        group.tools.into_iter().any(|tool| {
            let desc = tool.tool_description();
            let name = desc.name;
            let cfg = desc.source.config_path;
            (name.contains("github") || cfg.contains("github"))
                && (name.ends_with("_create_issue")
                    || name.ends_with("_get_file_contents")
                    || name.ends_with("_search_code"))
        })
    })
}

pub async fn investigation_logs(
    gcx: Arc<ARwLock<GlobalContext>>,
    error: &str,
    collected_at: Option<&str>,
) -> Result<String, String> {
    let ccx = Arc::new(AMutex::new(
        AtCommandsContext::new(
            gcx,
            4000,
            20,
            false,
            vec![ChatMessage::new("user".to_string(), error.to_string())],
            String::new(),
            None,
            String::new(),
            None,
        )
        .await,
    ));
    let mut tool = crate::tools::tool_buddy_get_logs::ToolBuddyGetLogs {
        config_path: String::new(),
    };
    let mut args = HashMap::new();
    args.insert("lines".to_string(), serde_json::json!(80));
    args.insert("errors_only".to_string(), serde_json::json!(true));
    let (_, out) = tool
        .tool_execute(ccx, &"buddy_logs".to_string(), &args)
        .await?;
    let text = extract_tool_text(out, "Investigation logs were unavailable.");
    let Some(collected_at) = collected_at else {
        return Ok(text);
    };

    let mut kept = Vec::new();
    for line in text.lines() {
        if line.starts_with("Log lines (")
            || line.trim().is_empty()
            || crate::buddy::actor::same_day_log_filter(line, collected_at)
        {
            kept.push(line.to_string());
        }
    }

    if kept.len() <= 1 {
        return Ok("No log lines found matching the diagnostic timestamp window.".to_string());
    }

    Ok(kept.join("\n"))
}

pub async fn investigation_internal_context(
    gcx: Arc<ARwLock<GlobalContext>>,
) -> Result<String, String> {
    let ccx = Arc::new(AMutex::new(
        AtCommandsContext::new(
            gcx,
            4000,
            20,
            false,
            vec![],
            String::new(),
            None,
            String::new(),
            None,
        )
        .await,
    ));
    let mut tool = crate::tools::tool_buddy_get_context::ToolBuddyGetContext {
        config_path: String::new(),
    };
    let mut args = HashMap::new();
    args.insert(
        "sections".to_string(),
        serde_json::json!([
            "integrations",
            "mcp_servers",
            "setup_status",
            "project_info"
        ]),
    );
    let (_, out) = tool
        .tool_execute(ccx, &"buddy_ctx".to_string(), &args)
        .await?;
    Ok(extract_tool_text(
        out,
        "Investigation context was unavailable.",
    ))
}

pub(crate) fn mcp_issue_args(
    owner: &str,
    repo: &str,
    title: &str,
    body: &str,
    labels: Vec<String>,
) -> serde_json::Value {
    serde_json::json!({
        "owner": owner,
        "repo": repo,
        "title": title,
        "body": body,
        "labels": labels,
    })
}

pub async fn create_issue_via_mcp(
    gcx: Arc<ARwLock<GlobalContext>>,
    title: &str,
    body: &str,
    labels: Vec<String>,
) -> Result<BuddyIssueCreateResult, String> {
    let project_root = crate::files_correction::get_project_dirs(gcx.clone())
        .await
        .into_iter()
        .next()
        .ok_or_else(|| "no project root".to_string())?;
    let repo = detect_repo_from_git(&project_root)
        .ok_or_else(|| "could not detect issue repository from git origin remote".to_string())?;
    if !matches!(&repo.host, RepoHost::GitHub) {
        return Err("GitHub MCP issue creation requires a GitHub origin remote".to_string());
    }

    let ccx = Arc::new(AMutex::new(
        AtCommandsContext::new(
            gcx.clone(),
            4000,
            20,
            false,
            vec![],
            String::new(),
            None,
            String::new(),
            None,
        )
        .await,
    ));
    let mut tool = crate::tools::tool_mcp_call::ToolMcpCall {};
    let mut args = HashMap::new();
    let mcp_tool = {
        let groups = crate::tools::tools_list::get_integration_tools(gcx.clone()).await;
        groups
            .into_iter()
            .flat_map(|group| group.tools)
            .find_map(|tool| {
                let desc = tool.tool_description();
                let name = desc.name;
                let cfg = desc.source.config_path;
                if (name.contains("github") || cfg.contains("github"))
                    && name.ends_with("_create_issue")
                {
                    Some(name)
                } else {
                    None
                }
            })
            .ok_or_else(|| "GitHub MCP issue tool not available".to_string())?
    };
    args.insert("tool_name".to_string(), serde_json::json!(mcp_tool));
    args.insert(
        "args".to_string(),
        mcp_issue_args(&repo.owner, &repo.repo, title, body, labels),
    );
    let (_, out) = tool
        .tool_execute(ccx, &"buddy_mcp_issue".to_string(), &args)
        .await?;
    let text = extract_tool_text(out, "");

    Ok(BuddyIssueCreateResult {
        url: text,
        provider: "github_mcp".to_string(),
        repo: repo.full_name(),
    })
}

pub async fn create_issue_via_native(
    gcx: Arc<ARwLock<GlobalContext>>,
    diagnostic_index: Option<usize>,
    diagnostic_id: Option<String>,
    collected_at: Option<String>,
    error: Option<String>,
) -> Result<BuddyIssueCreateResult, String> {
    let pre_diag = if diagnostic_index.is_none() {
        match error.as_ref() {
            Some(err) => {
                Some(crate::buddy::diagnostics::collect_diagnostics(gcx.clone(), err).await)
            }
            None => None,
        }
    } else {
        None
    };

    let ctx = crate::buddy::actor::resolve_diagnostic(
        gcx.clone(),
        diagnostic_index,
        diagnostic_id.as_deref(),
        collected_at.as_deref(),
        pre_diag,
    )
    .await?;

    let (auto_enabled, last_issue_at, recent_errors) = {
        let buddy_arc = gcx.read().await.buddy.clone();
        let lock = buddy_arc.lock().await;
        let svc = lock
            .as_ref()
            .ok_or_else(|| "buddy service not initialized".to_string())?;

        (
            svc.settings.auto_issue_creation,
            svc.last_issue_at,
            svc.recent_issue_errors.clone(),
        )
    };

    let (url, _activity) = create_issue(
        gcx.clone(),
        &ctx,
        auto_enabled,
        false,
        last_issue_at,
        &recent_errors,
    )
    .await?;
    let project_root = crate::files_correction::get_project_dirs(gcx)
        .await
        .into_iter()
        .next()
        .ok_or_else(|| "no project root".to_string())?;
    let repo = detect_repo_from_git(&project_root)
        .ok_or_else(|| "could not detect issue repository from git origin remote".to_string())?;

    Ok(BuddyIssueCreateResult {
        url,
        provider: "native".to_string(),
        repo: repo.full_name(),
    })
}

pub(crate) fn redact_diagnostic_text(text: &str) -> String {
    let mut result = redact_sensitive(text);
    if let Ok(home) = std::env::var("HOME") {
        if !home.is_empty() {
            result = result.replace(&home, "~");
        }
    }
    if result.chars().count() > 2000 {
        result = result.chars().take(2000).collect();
    }
    result
}

pub(crate) fn sanitize_title(raw: &str) -> String {
    let single: String = raw.chars().filter(|&c| c != '\n' && c != '\r').collect();
    if single.chars().count() > 120 {
        single.chars().take(120).collect()
    } else {
        single
    }
}

pub(crate) fn sanitize_body(raw: &str) -> String {
    let escaped = raw.replace("```", "'''");
    if escaped.chars().count() > 4000 {
        escaped.chars().take(4000).collect()
    } else {
        escaped
    }
}

fn format_issue_body(ctx: &DiagnosticContext) -> String {
    let mut body = format!(
        "**Error type**: {}\n**Severity**: {:?}\n**Collected at**: {}\n\n**Message**:\n```\n{}\n```",
        ctx.error_type, ctx.severity, ctx.collected_at, ctx.error_message
    );
    if let Some(ref file) = ctx.source_file {
        body.push_str(&format!("\n\n**Source file**: `{}`", file));
    }
    if let Some(ref tool) = ctx.tool_name {
        body.push_str(&format!("\n**Tool**: `{}`", tool));
    }
    if let Some(ref chat) = ctx.chat_id {
        body.push_str(&format!("\n**Chat ID**: `{}`", chat));
    }
    body.push_str("\n\n_Auto-created by Buddy diagnostics pipeline._");
    body
}

pub(crate) fn issue_title_and_body(ctx: &DiagnosticContext) -> (String, String) {
    let mut redacted = ctx.clone();
    redacted.error_message = redact_diagnostic_text(&ctx.error_message);
    let raw_title = format!(
        "[Buddy] {}: {}",
        ctx.error_type,
        &redacted.error_message.chars().take(80).collect::<String>()
    );
    let title = sanitize_title(&raw_title);
    let raw_body = format_issue_body(&redacted);
    let body = sanitize_body(&raw_body);
    (title, body)
}

pub async fn create_issue(
    gcx: Arc<ARwLock<GlobalContext>>,
    context: &DiagnosticContext,
    auto_creation_enabled: bool,
    manual: bool,
    last_issue_at: Option<std::time::Instant>,
    recent_errors: &[(String, chrono::DateTime<chrono::Utc>)],
) -> Result<(String, BuddyActivity), String> {
    let project_root = crate::files_correction::get_project_dirs(gcx.clone())
        .await
        .into_iter()
        .next()
        .ok_or_else(|| "no project root".to_string())?;

    let repo = detect_repo_from_git(&project_root)
        .ok_or_else(|| "could not detect issue repository from git origin remote".to_string())?;
    let provider = detect_provider(gcx.clone(), &repo).await?;

    let gate = IssueGate {
        has_diagnostics: !context.error_message.is_empty()
            && (context.source_file.is_some() || context.tool_name.is_some()),
        has_repro_context: context.source_file.is_some() || context.tool_name.is_some(),
        integration_configured: provider.is_some(),
        auto_creation_enabled,
        within_rate_limit: last_issue_at
            .map(|t| t.elapsed().as_secs() >= RATE_LIMIT_SECS)
            .unwrap_or(true),
    };

    let passed = if manual {
        check_manual_issue_gate(&gate)
    } else {
        check_issue_gate(&gate)
    };

    if !passed {
        return Err(gate_error(&gate, manual));
    }

    let now = chrono::Utc::now();
    for (msg, ts) in recent_errors {
        let age = now.signed_duration_since(*ts).num_seconds();
        if age < DEDUP_SECS && msg == &context.error_message {
            return Err("Duplicate issue suppressed (same error within 24h)".to_string());
        }
    }

    let (title, body) = issue_title_and_body(context);

    let url = run_issue_create(provider.unwrap(), &repo, &project_root, &title, &body).await?;

    info!("buddy: created issue {}", url);

    let activity = BuddyActivity {
        icon: "🐛".to_string(),
        title: "Issue created".to_string(),
        description: format!("Auto-created issue: {}", url),
        timestamp: chrono::Utc::now().to_rfc3339(),
        activity_type: "issue_created".to_string(),
    };
    Ok((url, activity))
}

async fn run_issue_create(
    provider: IssueProvider,
    repo: &RepoInfo,
    project_root: &Path,
    title: &str,
    body: &str,
) -> Result<String, String> {
    let repo_name = repo.full_name();
    match provider {
        IssueProvider::GitHub { binary, token } => {
            let out = Command::new(&binary)
                .args([
                    "issue", "create", "-R", &repo_name, "--title", title, "--body", body,
                ])
                .current_dir(project_root)
                .env("GH_TOKEN", &token)
                .env("GITHUB_TOKEN", &token)
                .stdin(std::process::Stdio::null())
                .output()
                .await
                .map_err(|e| format!("gh failed: {}", e))?;
            if out.status.success() {
                Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
            } else {
                Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
            }
        }
        IssueProvider::GitLab { binary, token } => {
            let out = Command::new(&binary)
                .args([
                    "issue",
                    "create",
                    "-R",
                    &repo_name,
                    "--title",
                    title,
                    "--description",
                    body,
                ])
                .current_dir(project_root)
                .env("GITLAB_TOKEN", &token)
                .stdin(std::process::Stdio::null())
                .output()
                .await
                .map_err(|e| format!("glab failed: {}", e))?;
            if out.status.success() {
                Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
            } else {
                Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
            }
        }
    }
}
