use std::path::PathBuf;
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::RwLock as ARwLock;
use tracing::info;

use crate::global_context::GlobalContext;
use super::diagnostics::DiagnosticContext;
use super::types::BuddyActivity;

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
        && gate.integration_configured
        && gate.auto_creation_enabled
        && gate.within_rate_limit
}

#[derive(Debug, Clone)]
enum IssueProvider {
    GitHub { binary: String, token: String },
    GitLab { binary: String, token: String },
}

async fn detect_provider(gcx: Arc<ARwLock<GlobalContext>>) -> Option<IssueProvider> {
    let active = crate::files_correction::get_active_project_path(gcx.clone()).await;
    let (config_dirs, global_config_dir) =
        crate::integrations::setting_up_integrations::get_config_dirs(gcx.clone(), &active).await;

    let mut search_dirs: Vec<PathBuf> = config_dirs;
    search_dirs.push(global_config_dir);

    for dir in search_dirs {
        if let Some(p) = try_read_github(&dir).await {
            return Some(p);
        }
        if let Some(p) = try_read_gitlab(&dir).await {
            return Some(p);
        }
    }
    None
}

async fn try_read_github(config_dir: &PathBuf) -> Option<IssueProvider> {
    let path = config_dir.join("integrations.d").join("github.yaml");
    let content = tokio::fs::read_to_string(&path).await.ok()?;
    let val: serde_yaml::Value = serde_yaml::from_str(&content).ok()?;
    let token = val.get("gh_token")?.as_str()?.to_string();
    if token.is_empty() {
        return None;
    }
    let binary = val
        .get("gh_binary_path")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("gh")
        .to_string();
    Some(IssueProvider::GitHub { binary, token })
}

async fn try_read_gitlab(config_dir: &PathBuf) -> Option<IssueProvider> {
    let path = config_dir.join("integrations.d").join("gitlab.yaml");
    let content = tokio::fs::read_to_string(&path).await.ok()?;
    let val: serde_yaml::Value = serde_yaml::from_str(&content).ok()?;
    let token = val.get("glab_token")?.as_str()?.to_string();
    if token.is_empty() {
        return None;
    }
    let binary = val
        .get("glab_binary_path")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("glab")
        .to_string();
    Some(IssueProvider::GitLab { binary, token })
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

pub async fn create_issue(
    gcx: Arc<ARwLock<GlobalContext>>,
    context: &DiagnosticContext,
    auto_creation_enabled: bool,
    last_issue_at: Option<std::time::Instant>,
    recent_errors: &[(String, chrono::DateTime<chrono::Utc>)],
) -> Result<(String, BuddyActivity), String> {
    let provider = detect_provider(gcx.clone()).await;
    let gate = IssueGate {
        has_diagnostics: true,
        has_repro_context: context.source_file.is_some() || context.tool_name.is_some(),
        integration_configured: provider.is_some(),
        auto_creation_enabled,
        within_rate_limit: last_issue_at
            .map(|t| t.elapsed().as_secs() >= RATE_LIMIT_SECS)
            .unwrap_or(true),
    };

    if !check_issue_gate(&gate) {
        return Err(format!(
            "Issue gate blocked: auto_creation={}, integration={}, rate_limit_ok={}",
            gate.auto_creation_enabled, gate.integration_configured, gate.within_rate_limit
        ));
    }

    let now = chrono::Utc::now();
    for (msg, ts) in recent_errors {
        let age = now.signed_duration_since(*ts).num_seconds();
        if age < DEDUP_SECS && msg == &context.error_message {
            return Err("Duplicate issue suppressed (same error within 24h)".to_string());
        }
    }

    let project_root = crate::files_correction::get_project_dirs(gcx.clone())
        .await
        .into_iter()
        .next()
        .ok_or_else(|| "no project root".to_string())?;

    let title = format!("[Buddy] {}: {}", context.error_type, &context.error_message.chars().take(80).collect::<String>());
    let body = format_issue_body(context);
    let url = run_issue_create(provider.unwrap(), &project_root, &title, &body).await?;

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
    project_root: &std::path::Path,
    title: &str,
    body: &str,
) -> Result<String, String> {
    match provider {
        IssueProvider::GitHub { binary, token } => {
            let out = Command::new(&binary)
                .args(["issue", "create", "--title", title, "--body", body])
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
                .args(["issue", "create", "--title", title, "--description", body])
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
