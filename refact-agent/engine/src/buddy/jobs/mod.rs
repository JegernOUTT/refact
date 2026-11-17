pub mod autonomous_chats;
pub mod buddy_briefing;
pub mod buddy_daily_digest;
pub mod buddy_friday_retro;
pub mod buddy_handbook_distiller;
pub mod buddy_idle_suggester;
pub mod buddy_onboarding;
pub mod buddy_pr_issue_matchmaker;
pub mod buddy_refactor_hunter;
pub mod buddy_skill_author;
pub mod buddy_test_coverage_watcher;
pub mod chat_phrase_bank;
pub mod config_watcher;
pub mod error_triage;
pub mod greeting;
pub mod health_watcher;
pub mod organization;
pub mod proactive_suggestions;
pub mod quest_prompt;
pub mod refact_compile_sniffer;
pub mod refact_self_critic;
pub mod speaker_insight;
pub mod speaker_memory_pulse_commentary;
pub mod speaker_win;
pub mod stats_watcher;
pub mod tour;

pub(super) async fn trusted_git_output(
    project_root: &std::path::Path,
    args: &[&str],
) -> Option<String> {
    let mut command = tokio::process::Command::new("git");
    command
        .arg("-C")
        .arg(project_root)
        .args(args)
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(std::time::Duration::from_secs(10), command.output())
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

pub(super) fn recent_activity_evidence(
    ctx: &super::scheduler::BuddyJobContext,
    limit: usize,
) -> String {
    let activities = ctx
        .recent_activities
        .iter()
        .rev()
        .take(limit)
        .map(|activity| {
            format!(
                "- {} [{}] {} — {}",
                activity.timestamp, activity.activity_type, activity.title, activity.description
            )
        })
        .collect::<Vec<_>>();
    if activities.is_empty() {
        "none".to_string()
    } else {
        activities.join("\n")
    }
}
