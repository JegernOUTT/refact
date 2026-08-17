use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::global_context::GlobalContext;
use crate::subchat::ExplicitSubchatSpec;
use crate::tools::review_agents::agentic::{
    build_agent_task_prompt, run_agentic_instance, AgenticInstance,
};
use crate::tools::review_agents::{AgentCtx, AgentOutcome};
use crate::tools::review_scope::ReviewScope;

pub const AGENT_ID: &str = "a3_execution";
const MAX_REPRO_TARGETS: usize = 8;
const MAX_UNTRACKED_COPY: usize = 200;
const MAX_UNTRACKED_BYTES: u64 = 1_000_000;

#[derive(Debug, Clone, Serialize)]
pub struct ReproTarget {
    pub id: String,
    pub file: String,
    pub line1: u32,
    pub line2: u32,
    pub severity: String,
    pub claim: String,
}

#[derive(Deserialize)]
struct RefutedEnvelope {
    #[serde(default)]
    refuted: Vec<String>,
}

pub fn extract_refuted_ids(text: &str) -> Vec<String> {
    let Ok(json) = crate::tools::review_candidates::extract_last_json_block(text) else {
        return vec![];
    };
    serde_json::from_str::<RefutedEnvelope>(json)
        .map(|envelope| envelope.refuted)
        .unwrap_or_default()
}

async fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .await
        .map_err(|e| format!("git spawn: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub struct ScratchWorktree {
    pub path: PathBuf,
    repo_root: PathBuf,
}

impl ScratchWorktree {
    pub async fn create(repo_root: &Path) -> Option<Self> {
        let scratch_dir = repo_root
            .join(".refact")
            .join("review_scratch")
            .join(uuid::Uuid::new_v4().to_string()[..8].to_string());
        if tokio::fs::create_dir_all(scratch_dir.parent()?)
            .await
            .is_err()
        {
            return None;
        }
        let scratch_str = scratch_dir.to_string_lossy().to_string();
        if let Err(error) = git(
            repo_root,
            &["worktree", "add", "--detach", &scratch_str, "HEAD"],
        )
        .await
        {
            tracing::warn!("review a3: scratch worktree failed: {error}");
            return None;
        }
        let scratch = Self {
            path: scratch_dir.clone(),
            repo_root: repo_root.to_path_buf(),
        };

        match git(repo_root, &["diff", "HEAD", "--binary"]).await {
            Ok(patch) if !patch.trim().is_empty() => {
                let apply = async {
                    use tokio::io::AsyncWriteExt;
                    let mut child = Command::new("git")
                        .args(["apply", "--whitespace=nowarn"])
                        .current_dir(&scratch_dir)
                        .stdin(std::process::Stdio::piped())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn()
                        .map_err(|e| e.to_string())?;
                    if let Some(stdin) = child.stdin.as_mut() {
                        stdin
                            .write_all(patch.as_bytes())
                            .await
                            .map_err(|e| e.to_string())?;
                    }
                    drop(child.stdin.take());
                    let status = child.wait().await.map_err(|e| e.to_string())?;
                    if status.success() {
                        Ok(())
                    } else {
                        Err("git apply failed".to_string())
                    }
                };
                if let Err(error) = apply.await {
                    tracing::warn!("review a3: applying working diff to scratch failed: {error}");
                }
            }
            _ => {}
        }

        if let Ok(untracked) = git(repo_root, &["ls-files", "--others", "--exclude-standard"]).await
        {
            for rel in untracked.lines().take(MAX_UNTRACKED_COPY) {
                let src = repo_root.join(rel);
                let dst = scratch_dir.join(rel);
                if let Ok(meta) = tokio::fs::metadata(&src).await {
                    if meta.len() > MAX_UNTRACKED_BYTES {
                        continue;
                    }
                }
                if let Some(parent) = dst.parent() {
                    let _ = tokio::fs::create_dir_all(parent).await;
                }
                let _ = tokio::fs::copy(&src, &dst).await;
            }
        }

        Some(scratch)
    }

    pub async fn cleanup(self) {
        let scratch_str = self.path.to_string_lossy().to_string();
        if git(
            &self.repo_root,
            &["worktree", "remove", "--force", &scratch_str],
        )
        .await
        .is_err()
        {
            let _ = tokio::fs::remove_dir_all(&self.path).await;
        }
    }
}

fn repo_root_for(gcx: &GlobalContext, scope: &ReviewScope) -> Option<PathBuf> {
    let workspace_folders = gcx.documents_state.workspace_folders.lock().ok()?.clone();
    scope
        .files
        .iter()
        .map(|path| {
            if path.is_dir() {
                path.as_path()
            } else {
                path.parent().unwrap_or(path.as_path())
            }
        })
        .chain(workspace_folders.iter().map(PathBuf::as_path))
        .find_map(|path| {
            let repo = refact_worktrees::git::discover_repo(path).ok()?;
            refact_worktrees::git::repo_root(&repo).ok()
        })
}

pub struct ExecAgentInput {
    pub slot_label: String,
    pub spec: ExplicitSubchatSpec,
    pub system_prompt: String,
    pub tools: Vec<String>,
    pub max_steps: usize,
    pub mutation_probe_cap: usize,
    pub repro_targets: Vec<ReproTarget>,
}

pub(crate) async fn run_exec_agent(
    gcx: Arc<GlobalContext>,
    ctx: AgentCtx,
    input: ExecAgentInput,
    scope: Arc<ReviewScope>,
) -> AgentOutcome {
    let repo_root = repo_root_for(gcx.as_ref(), &scope);
    let scratch = match &repo_root {
        Some(root) => ScratchWorktree::create(root).await,
        None => None,
    };

    let mut targets = input.repro_targets;
    targets.truncate(MAX_REPRO_TARGETS);
    let targets_json = serde_json::to_string_pretty(&targets).unwrap_or_else(|_| "[]".to_string());

    let mut extra = String::new();
    match &scratch {
        Some(scratch) => extra.push_str(&format!(
            "# Scratch checkout\nA disposable scratch checkout mirroring the current working tree exists at:\n{}\nRun ALL mutation probes and destructive experiments there, never in the primary tree. Restore is unnecessary — the scratch is discarded after the review.\n",
            scratch.path.to_string_lossy()
        )),
        None => extra.push_str(
            "# Scratch checkout\nNo scratch checkout is available. Do NOT modify any files; skip mutation probes and run only non-destructive commands (builds, tests, targeted scripts in temp files under the project).\n",
        ),
    }
    extra.push_str(&format!(
        "\n# Mutation probe cap\nRun at most {} mutation probes; prioritize the most load-bearing changed lines.\n",
        input.mutation_probe_cap
    ));
    if !targets.is_empty() {
        extra.push_str(&format!(
            "\n# Repro targets from other reviewers\nAttempt to reproduce or refute each claim below with a minimal script or targeted test run. List every refuted id in a top-level \"refuted\" array of your final json envelope.\n```json\n{targets_json}\n```\n"
        ));
    }

    let task_prompt = build_agent_task_prompt(&scope, Some(&extra));
    let instance = AgenticInstance {
        agent_id: AGENT_ID.to_string(),
        slot_label: input.slot_label,
        spec: input.spec,
        system_prompt: input.system_prompt,
        task_prompt,
        tools: input.tools,
        max_steps: input.max_steps,
        title: "Review: Execution Agent".to_string(),
        verify: false,
    };

    let mut outcome = run_agentic_instance(gcx, ctx, instance, scope, None).await;
    if let Some(text) = outcome.raw_final_answer.as_deref() {
        outcome.refuted = extract_refuted_ids(text);
    }
    if let Some(scratch) = scratch {
        scratch.cleanup().await;
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a3_extracts_refuted_ids_from_final_envelope() {
        let text = r#"analysis text
```json
{"summary":"ran the suites","candidates":[],"refuted":["rf-11aa22bb","rf-33cc44dd"]}
```"#;
        assert_eq!(
            extract_refuted_ids(text),
            vec!["rf-11aa22bb".to_string(), "rf-33cc44dd".to_string()]
        );
    }

    #[test]
    fn a3_refuted_defaults_to_empty() {
        let text = "```json\n{\"summary\":\"s\",\"candidates\":[]}\n```";
        assert!(extract_refuted_ids(text).is_empty());
        assert!(extract_refuted_ids("no json").is_empty());
    }
}
