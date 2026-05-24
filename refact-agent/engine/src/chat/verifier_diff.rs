use std::collections::HashSet;
use std::path::Path;

use tokio::process::Command;

// TODO: dedupe this with tool_agent_diff once its git helpers are shared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VerifierDiffBase {
    refish: String,
    label: String,
}

fn present(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(crate) fn resolve_verifier_diff_base(
    base_commit: Option<String>,
    base_branch: Option<String>,
) -> Result<VerifierDiffBase, String> {
    if let Some(commit) = present(base_commit) {
        return Ok(VerifierDiffBase {
            refish: commit.clone(),
            label: format!("commit {}", commit),
        });
    }
    if let Some(branch) = present(base_branch) {
        return Ok(VerifierDiffBase {
            refish: branch.clone(),
            label: format!("branch {}", branch),
        });
    }
    Err("Task has no base commit or base branch set for verifier diff".to_string())
}

async fn run_git(worktree: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(worktree)
        .output()
        .await
        .map_err(|e| format!("failed to run git {:?}: {}", args, e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!(
            "git {:?} failed: {}",
            args,
            if stderr.is_empty() {
                "unknown git error"
            } else {
                stderr.as_str()
            }
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn push_name_only(names: &mut Vec<String>, seen: &mut HashSet<String>, output: &str) {
    for line in output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if seen.insert(line.to_string()) {
            names.push(line.to_string());
        }
    }
}

async fn changed_files(worktree: &Path, base: &VerifierDiffBase) -> Result<Vec<String>, String> {
    let range = format!("{}...HEAD", base.refish);
    let committed = run_git(worktree, &["diff", &range, "--name-only"]).await?;
    let staged = run_git(worktree, &["diff", "--cached", "--name-only"]).await?;
    let unstaged = run_git(worktree, &["diff", "--name-only"]).await?;
    let untracked = run_git(worktree, &["ls-files", "--others", "--exclude-standard"]).await?;

    let mut names = Vec::new();
    let mut seen = HashSet::new();
    push_name_only(&mut names, &mut seen, &committed);
    push_name_only(&mut names, &mut seen, &staged);
    push_name_only(&mut names, &mut seen, &unstaged);
    push_name_only(&mut names, &mut seen, &untracked);
    Ok(names)
}

fn truncate_lines(lines: Vec<String>, max_lines: usize) -> Vec<String> {
    if lines.len() <= max_lines {
        return lines;
    }
    let omitted = lines.len().saturating_sub(max_lines);
    let mut truncated = lines.into_iter().take(max_lines).collect::<Vec<_>>();
    truncated.push(format!("... ({} more changed files omitted)", omitted));
    truncated
}

pub(crate) async fn git_changed_files_summary(
    worktree: &Path,
    base: &VerifierDiffBase,
    max_lines: usize,
) -> Result<String, String> {
    let files = changed_files(worktree, base).await?;
    let mut lines = vec![format!("Base: {}", base.label)];
    if files.is_empty() {
        lines.push("(no changes detected)".to_string());
    } else {
        lines.push("Changed files:".to_string());
        lines.extend(files);
    }
    Ok(truncate_lines(lines, max_lines).join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::process::Command as StdCommand;

    fn run_git(cwd: &Path, args: &[&str]) -> String {
        let output = StdCommand::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap_or_else(|e| panic!("failed to run git {:?}: {}", args, e));
        if !output.status.success() {
            panic!(
                "git {:?} failed: {}",
                args,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        String::from_utf8_lossy(&output.stdout).to_string()
    }

    fn init_repo(root: &Path) {
        run_git(root, &["init"]);
        run_git(root, &["checkout", "-b", "main"]);
        run_git(root, &["config", "user.email", "test@example.com"]);
        run_git(root, &["config", "user.name", "Test User"]);
        std::fs::write(root.join("file.txt"), "hello\n").unwrap();
        run_git(root, &["add", "file.txt"]);
        run_git(root, &["commit", "-m", "initial"]);
    }

    fn write_file(root: &Path, path: &str, content: &str) {
        let full_path = root.join(path);
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(full_path, content).unwrap();
    }

    fn commit_file(root: &Path, path: &str, content: &str, message: &str) {
        write_file(root, path, content);
        run_git(root, &["add", path]);
        run_git(root, &["commit", "-m", message]);
    }

    #[tokio::test]
    async fn verifier_diff_uses_base_commit_when_available() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        init_repo(&repo);
        let base_commit = run_git(&repo, &["rev-parse", "HEAD"]).trim().to_string();
        run_git(&repo, &["checkout", "-b", "agent-branch"]);
        commit_file(&repo, "agent.txt", "agent\n", "agent change");
        let base = resolve_verifier_diff_base(
            Some(base_commit.clone()),
            Some("missing-base-branch".to_string()),
        )
        .unwrap();

        let summary = git_changed_files_summary(&repo, &base, 200).await.unwrap();

        assert!(summary.contains(&format!("Base: commit {}", base_commit)));
        assert!(summary.contains("agent.txt"));
    }

    #[tokio::test]
    async fn verifier_diff_falls_back_to_base_branch() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        init_repo(&repo);
        run_git(&repo, &["checkout", "-b", "agent-branch"]);
        commit_file(&repo, "agent.txt", "agent\n", "agent change");
        let base = resolve_verifier_diff_base(None, Some("main".to_string())).unwrap();

        let summary = git_changed_files_summary(&repo, &base, 200).await.unwrap();

        assert!(summary.contains("Base: branch main"));
        assert!(summary.contains("agent.txt"));
    }

    #[tokio::test]
    async fn verifier_diff_includes_uncommitted_changes() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        init_repo(&repo);
        let base_commit = run_git(&repo, &["rev-parse", "HEAD"]).trim().to_string();
        run_git(&repo, &["checkout", "-b", "agent-branch"]);
        commit_file(&repo, "committed.txt", "committed\n", "committed change");
        write_file(&repo, "staged.txt", "staged\n");
        run_git(&repo, &["add", "staged.txt"]);
        write_file(&repo, "file.txt", "hello\nunstaged\n");
        write_file(&repo, "untracked.txt", "untracked\n");
        let base = resolve_verifier_diff_base(Some(base_commit), None).unwrap();

        let summary = git_changed_files_summary(&repo, &base, 200).await.unwrap();

        for expected in ["committed.txt", "staged.txt", "file.txt", "untracked.txt"] {
            assert!(
                summary.contains(expected),
                "missing {expected} in {summary}"
            );
        }
    }
}
