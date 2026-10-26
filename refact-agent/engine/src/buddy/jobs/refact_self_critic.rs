use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;

use crate::buddy::autonomous_workflows::{autonomous_workflow_meta, REFACT_SELF_CRITIC_WORKFLOW_ID};
use crate::buddy::jobs::autonomous_chats::{execute_autonomous_spec, AutonomousBuddyChatSpec};
use crate::buddy::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};
use crate::app_state::AppState;

pub struct RefactSelfCriticJob;

const COOLDOWN_SECONDS: u64 = 24 * 60 * 60;
const PRIORITY: u32 = 24;

fn is_refact_engine_workspace(root: &Path) -> bool {
    root.join("refact-agent/engine/Cargo.toml").is_file()
}

fn hash_prompt_tree_at(defaults_dir: &Path) -> String {
    fn collect_files(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_files(&path, files);
            } else if path.is_file() {
                files.push(path);
            }
        }
    }

    let mut files = Vec::new();
    collect_files(defaults_dir, &mut files);
    files.sort();

    let mut hasher = Sha256::new();
    hasher.update(b"refact-default-prompts-v1\0");
    for path in files {
        let rel = path.strip_prefix(defaults_dir).unwrap_or(path.as_path());
        hasher.update(rel.to_string_lossy().as_bytes());
        hasher.update(b"\0");
        match std::fs::read(&path) {
            Ok(bytes) => hasher.update(bytes),
            Err(err) => hasher.update(format!("read_error:{err}").as_bytes()),
        }
        hasher.update(b"\0");
    }
    format!("{:x}", hasher.finalize())
}

fn default_prompt_fingerprint() -> String {
    let defaults_dir =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("crates/refact-yaml-configs/src/defaults");
    hash_prompt_tree_at(&defaults_dir)
}

fn top_diagnostic_counts(ctx: &BuddyJobContext) -> String {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for diag in &ctx.recent_diagnostics {
        *counts.entry(diag.error_type.clone()).or_default() += 1;
    }
    let mut counts = counts.into_iter().collect::<Vec<_>>();
    counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    counts
        .into_iter()
        .take(3)
        .map(|(error_type, count)| format!("{}={}", error_type, count))
        .collect::<Vec<_>>()
        .join(", ")
}

fn build_self_critic_spec(ctx: &BuddyJobContext) -> AutonomousBuddyChatSpec {
    let meta = autonomous_workflow_meta(REFACT_SELF_CRITIC_WORKFLOW_ID).unwrap();
    let project_root = ctx.project_root.to_string_lossy().to_string();
    let prompt_fingerprint = default_prompt_fingerprint();
    let evidence = format!(
        "prompt_fingerprint={}\nproject_root={}\nworkflow_runs={}\ntop_diagnostics={}",
        prompt_fingerprint,
        project_root,
        ctx.total_workflow_runs,
        top_diagnostic_counts(ctx)
    );
    AutonomousBuddyChatSpec::new(
        meta.id,
        meta.title,
        "Review Refact default prompts and YAML configs for self-critique opportunities.",
        evidence,
    )
    .with_display(meta.icon, meta.badge, meta.priority)
    .with_project_root(project_root)
}

#[async_trait::async_trait]
impl BuddyJob for RefactSelfCriticJob {
    fn id(&self) -> &str {
        REFACT_SELF_CRITIC_WORKFLOW_ID
    }

    fn cooldown_seconds(&self) -> u64 {
        COOLDOWN_SECONDS
    }

    fn priority(&self) -> u32 {
        PRIORITY
    }

    async fn should_run(&self, _gcx: AppState, ctx: &BuddyJobContext) -> bool {
        is_refact_engine_workspace(&ctx.project_root)
    }

    async fn execute(&self, gcx: AppState, ctx: BuddyJobContext) -> BuddyJobResult {
        if !is_refact_engine_workspace(&ctx.project_root) {
            return BuddyJobResult::default();
        }
        execute_autonomous_spec(
            gcx,
            &ctx,
            build_self_critic_spec(&ctx),
            self.cooldown_seconds(),
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
}
