use chrono::{DateTime, Local, Timelike, Utc};

use crate::app_state::AppState;
use crate::buddy::actor::redact_sensitive;
use crate::buddy::scheduler::{BuddyJob, BuddyJobContext, BuddyJobResult};
use crate::buddy::settings::BuddySettings;
use crate::buddy::types::{BuddyChatPhrase, BuddyChatPhraseBank, BuddyFactKind};
use crate::buddy::voice_service::{voice_service, VoiceCtx};

pub struct ChatPhraseBankJob;

const FAILED_RESULT_PREFIX: &str = "failed:";
const JOB_ID: &str = "chat_phrase_bank";
const COOLDOWN_SECONDS: u64 = 0;
const PRIORITY: u32 = 4;
const MAX_EVIDENCE_LINES: usize = 18;
const MAX_JSON_ARRAY_ITEMS: usize = 12;
const MAX_JSON_OBJECT_FIELDS: usize = 24;
const MAX_JSON_DEPTH: usize = 6;
const MAX_JSON_STRING_CHARS: usize = 120;
const PHRASE_KINDS: [&str; 4] = ["humor", "insight", "bug", "ambient"];
const PHRASES_PER_KIND: usize = 3;

fn refresh_hour(settings: &BuddySettings) -> u32 {
    settings.daily_digest_hour.unwrap_or(18).min(23) as u32
}

fn date_key(now: &DateTime<Local>) -> String {
    now.date_naive().to_string()
}

fn should_run_at(ctx: &BuddyJobContext, now: DateTime<Local>) -> bool {
    let today = date_key(&now);
    let failed_today = format!("{FAILED_RESULT_PREFIX}{today}");
    if ctx
        .job_state
        .last_result
        .as_deref()
        .is_some_and(|result| result == today || result == failed_today.as_str())
    {
        return false;
    }
    now.hour() >= refresh_hour(&ctx.settings)
}

fn kind_token(kind: BuddyFactKind) -> &'static str {
    match kind {
        BuddyFactKind::TaskStuck => "task-stuck",
        BuddyFactKind::TaskAbandoned => "task-abandoned",
        BuddyFactKind::TaskClusterDuplicate => "task-duplicates",
        BuddyFactKind::TrajectoryClutter => "trajectory-clutter",
        BuddyFactKind::ChatRetryStreak => "chat-retries",
        BuddyFactKind::MemoryOrphan => "memory-orphan",
        BuddyFactKind::MemoryStaleConflict => "memory-conflict",
        BuddyFactKind::MemoryRecurringLesson => "memory-lesson",
        BuddyFactKind::ModePromptOverlap => "prompt-overlap",
        BuddyFactKind::SkillTriggerWeak => "skill-trigger",
        BuddyFactKind::AgentsMdGapDetected => "agents-md-gap",
        BuddyFactKind::DefaultModelMissing => "missing-model",
        BuddyFactKind::BrokenModelReference => "broken-model",
        BuddyFactKind::McpAuthExpired => "mcp-auth",
        BuddyFactKind::IntegrationFailing => "integration-failing",
        BuddyFactKind::DiagnosticCluster => "diagnostic-cluster",
        BuddyFactKind::FrontendErrorBurst => "frontend-errors",
        BuddyFactKind::GitDiffWidening => "git-diff-widening",
        BuddyFactKind::UncommittedPressure => "uncommitted-pressure",
        BuddyFactKind::WorktreeHygiene => "worktree-hygiene",
    }
}

fn cap_text(value: &str, max_chars: usize) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max_chars)
        .collect()
}

fn safe_evidence_text(value: &str, max_chars: usize) -> String {
    cap_text(&redact_sensitive(value), max_chars)
}

fn sensitive_json_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    [
        "api_key",
        "apikey",
        "authorization",
        "cookie",
        "password",
        "secret",
        "token",
    ]
    .into_iter()
    .any(|needle| key.contains(needle))
}

fn redact_evidence_json(value: &serde_json::Value) -> serde_json::Value {
    redact_evidence_json_at_depth(value, MAX_JSON_DEPTH)
}

fn redact_evidence_json_at_depth(
    value: &serde_json::Value,
    remaining_depth: usize,
) -> serde_json::Value {
    if remaining_depth == 0 {
        return serde_json::Value::String("[TRUNCATED]".to_string());
    }
    match value {
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .iter()
                .take(MAX_JSON_ARRAY_ITEMS)
                .map(|value| redact_evidence_json_at_depth(value, remaining_depth - 1))
                .collect(),
        ),
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .take(MAX_JSON_OBJECT_FIELDS)
                .map(|(key, value)| {
                    let value = if sensitive_json_key(key) {
                        serde_json::Value::String("[REDACTED]".to_string())
                    } else {
                        redact_evidence_json_at_depth(value, remaining_depth - 1)
                    };
                    (redact_sensitive(key), value)
                })
                .collect(),
        ),
        serde_json::Value::String(text) => {
            serde_json::Value::String(safe_evidence_text(text, MAX_JSON_STRING_CHARS))
        }
        other => other.clone(),
    }
}

fn safe_evidence_json(value: &serde_json::Value, max_chars: usize) -> String {
    safe_evidence_text(&redact_evidence_json(value).to_string(), max_chars)
}

fn phrase_bank_pulse_summary(ctx: &BuddyJobContext) -> String {
    let top_errors = safe_evidence_text(&ctx.pulse.diagnostics.top_error_types.join("|"), 160);
    format!(
        "tasks={} recent_stuck_alerts_1h={} memory={} pending={} mcp={} failing={} diagnostics={} top_errors={} git_files={} diff_lines={} worktrees={}",
        ctx.pulse.tasks.total,
        ctx.pulse.tasks.recent_stuck_alert_count_1h(),
        ctx.pulse.memory.total,
        ctx.pulse.memory.pending_ops,
        ctx.pulse.mcp.total,
        ctx.pulse.mcp.failing,
        ctx.pulse.diagnostics.last_hour,
        top_errors,
        ctx.pulse.git.uncommitted_files,
        ctx.pulse.git.diff_lines_4h,
        ctx.pulse.worktrees.total,
    )
}

fn phrase_bank_evidence(ctx: &BuddyJobContext) -> String {
    let mut lines = vec![format!("pulse: {}", phrase_bank_pulse_summary(ctx))];

    for activity in ctx.recent_activities.iter().rev().take(6) {
        lines.push(format!(
            "activity:{}:{}:{}",
            safe_evidence_text(&activity.activity_type, 32),
            safe_evidence_text(&activity.title, 80),
            safe_evidence_text(&activity.description, 120),
        ));
    }

    for fact in ctx.facts.iter().rev().take(8) {
        lines.push(format!(
            "fact:{} confidence={:.2} payload={}",
            kind_token(fact.kind),
            fact.confidence,
            safe_evidence_json(&fact.payload, 160),
        ));
    }

    for workflow in ctx.workflow_summaries.iter().rev().take(3) {
        let outcome = safe_evidence_text(workflow.last_outcome.as_deref().unwrap_or("none"), 120);
        lines.push(format!(
            "workflow:{} runs={} outcome={}",
            safe_evidence_text(&workflow.workflow_id, 48),
            workflow.run_count,
            outcome,
        ));
    }

    lines.truncate(MAX_EVIDENCE_LINES);
    lines.join("\n")
}

fn usable_phrase_count(lines: &[BuddyChatPhrase]) -> bool {
    lines.len() >= PHRASE_KINDS.len() * PHRASES_PER_KIND
        && PHRASE_KINDS
            .into_iter()
            .all(|kind| lines.iter().filter(|line| line.kind == kind).count() >= PHRASES_PER_KIND)
}

fn normalize_phrase_lines(lines: Vec<BuddyChatPhrase>) -> Vec<BuddyChatPhrase> {
    let mut normalized = Vec::with_capacity(PHRASE_KINDS.len() * PHRASES_PER_KIND);
    for kind in PHRASE_KINDS {
        normalized.extend(
            lines
                .iter()
                .filter(|line| line.kind == kind)
                .take(PHRASES_PER_KIND)
                .cloned(),
        );
    }
    normalized
}

pub async fn build_chat_phrase_bank(
    gcx: AppState,
    ctx: &BuddyJobContext,
    now: DateTime<Utc>,
) -> Option<BuddyChatPhraseBank> {
    let evidence = phrase_bank_evidence(ctx);
    let voice = voice_service().await;
    let voice_ctx = VoiceCtx {
        persona: &ctx.personality,
        identity_name: ctx.identity_name.as_str(),
        pulse_one_liner: phrase_bank_pulse_summary(ctx),
        workflow_id: Some(JOB_ID),
        workflow_summary: Some(evidence.as_str()),
    };
    let lines = voice.render_chat_phrase_bank(gcx, voice_ctx).await;
    if !usable_phrase_count(&lines) {
        return None;
    }
    let lines = normalize_phrase_lines(lines);
    Some(BuddyChatPhraseBank {
        day: now.date_naive().to_string(),
        generated_at: now.to_rfc3339(),
        evidence_summary: evidence,
        lines,
    })
}

#[async_trait::async_trait]
impl BuddyJob for ChatPhraseBankJob {
    fn id(&self) -> &str {
        JOB_ID
    }

    fn cooldown_seconds(&self) -> u64 {
        COOLDOWN_SECONDS
    }

    fn priority(&self) -> u32 {
        PRIORITY
    }

    fn records_empty_result(&self) -> bool {
        false
    }

    async fn should_run(&self, _gcx: AppState, ctx: &BuddyJobContext) -> bool {
        should_run_at(ctx, Local::now())
    }

    async fn execute(&self, gcx: AppState, ctx: BuddyJobContext) -> BuddyJobResult {
        let now = Utc::now();
        let key = date_key(&Local::now());
        match build_chat_phrase_bank(gcx, &ctx, now).await {
            Some(chat_phrase_bank) => BuddyJobResult {
                chat_phrase_bank: Some(chat_phrase_bank),
                last_result: Some(key),
                ..Default::default()
            },
            None => BuddyJobResult {
                last_result: Some(format!("{FAILED_RESULT_PREFIX}{key}")),
                ..Default::default()
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    use chrono::TimeZone;
    use crate::buddy::types::{
        BuddyActivity, BuddyFact, BuddyJobState, BuddyOnboarding, BuddyPetState, BuddyPulse,
        BuddyWorkflowSummary,
    };

    fn test_context(project_root: &Path) -> BuddyJobContext {
        BuddyJobContext {
            identity_name: "Pixel".to_string(),
            personality: Default::default(),
            onboarding: BuddyOnboarding::default(),
            recent_diagnostics: vec![],
            project_root: project_root.to_path_buf(),
            job_state: BuddyJobState::default(),
            workflow_summaries: vec![],
            total_workflow_runs: 0,
            suggestion_state: vec![],
            pet: BuddyPetState::default(),
            active_quest: None,
            settings: BuddySettings::default(),
            pulse: BuddyPulse::default(),
            facts: vec![],
            recent_activities: vec![],
        }
    }

    #[tokio::test]
    async fn phrase_bank_runs_once_per_day_at_digest_hour() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
        let hour = Local::now().hour() as u8;
        let mut ctx = test_context(dir.path());
        ctx.settings.daily_digest_hour = Some(hour);
        let job = ChatPhraseBankJob;

        assert!(job.should_run(gcx.clone(), &ctx).await);

        ctx.settings.daily_digest_hour = Some(hour.saturating_sub(1));
        assert!(job.should_run(gcx.clone(), &ctx).await);

        ctx.job_state.last_result = Some(date_key(&Local::now()));
        assert!(!job.should_run(gcx, &ctx).await);
    }

    #[test]
    fn phrase_bank_dedupes_on_local_date_key() {
        let dir = tempfile::tempdir().unwrap();
        let mut ctx = test_context(dir.path());
        ctx.settings.daily_digest_hour = Some(18);
        let now = Local
            .with_ymd_and_hms(2026, 6, 5, 23, 0, 0)
            .single()
            .unwrap();
        ctx.job_state.last_result = Some(date_key(&now));

        assert!(!should_run_at(&ctx, now));
    }

    #[tokio::test]
    async fn phrase_bank_prompt_uses_recent_signals() {
        let response = Some(
            [
                "humor: Tiny task goblin is juggling stuck work with oven mitts.",
                "humor: MCP crumbs formed a conga line near the logs.",
                "humor: Git gremlins stacked diffs into a tiny wobbly tower.",
                "insight: Pin the stuck task first; the breadcrumb parade needs a marshal.",
                "insight: Memory cleanup has snack-sized knots worth untangling.",
                "insight: Worktree clutter wants one tidy sweep before more sparkle.",
                "bug: Error crumbs are circling; bring the detective spoon.",
                "bug: Frontend hiccups look bug-shaped, with suspicious tap shoes.",
                "bug: The failing integration left muddy pawprints in the log hallway.",
                "ambient: Pixel is orbiting quietly with a tiny clipboard and snacks.",
                "ambient: Companion confetti on standby while the project hums.",
                "ambient: I am watching the trail without nibbling the evidence.",
            ]
            .join("\n"),
        );
        let (service, renderer) =
            crate::buddy::voice_service::test_voice_service_with_responses(vec![response]);
        let _guard = crate::buddy::voice_service::install_test_voice_service(service).await;
        let dir = tempfile::tempdir().unwrap();
        let gcx = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
        let mut ctx = test_context(dir.path());
        ctx.pulse.tasks.total = 7;
        ctx.pulse.tasks.recent_stuck_alerts_1h = 2;
        ctx.pulse.mcp.failing = 1;
        ctx.recent_activities.push(BuddyActivity {
            icon: "•".to_string(),
            title: "Reviewed chat bubble behavior".to_string(),
            description: "Kept repeated errors on Buddy Home".to_string(),
            timestamp: Utc::now().to_rfc3339(),
            activity_type: "review".to_string(),
            chat_id: None,
            failure_category: None,
            failure_summary: None,
        });

        let bank = build_chat_phrase_bank(gcx, &ctx, Utc::now())
            .await
            .expect("bank generated");

        assert_eq!(bank.lines.len(), 12);
        assert!(bank.lines.iter().any(|line| line.kind == "humor"));
        let prompts = renderer.prompts();
        let (system, user) = prompts.first().expect("prompt captured");
        assert!(system.contains("reusable daily Buddy chat phrase bank"));
        assert!(system.contains("Do not use generic canned phrases"));
        assert!(user.contains("tasks=7 recent_stuck_alerts_1h=2"));
        assert!(user.contains("Reviewed chat bubble behavior"));
        assert!(user.contains("Build 12 reusable one-liners"));
    }

    #[tokio::test]
    async fn phrase_bank_evidence_is_redacted_before_prompt_and_storage() {
        let response = Some(
            [
                "humor: Tiny task goblin is juggling stuck work with oven mitts.",
                "humor: MCP crumbs formed a conga line near the logs.",
                "humor: Git gremlins stacked diffs into a tiny wobbly tower.",
                "insight: Pin the stuck task first; the breadcrumb parade needs a marshal.",
                "insight: Memory cleanup has snack-sized knots worth untangling.",
                "insight: Worktree clutter wants one tidy sweep before more sparkle.",
                "bug: Error crumbs are circling; bring the detective spoon.",
                "bug: Frontend hiccups look bug-shaped, with suspicious tap shoes.",
                "bug: The failing integration left muddy pawprints in the log hallway.",
                "ambient: Pixel is orbiting quietly with a tiny clipboard and snacks.",
                "ambient: Companion confetti on standby while the project hums.",
                "ambient: I am watching the trail without nibbling the evidence.",
            ]
            .join("\n"),
        );
        let (service, renderer) =
            crate::buddy::voice_service::test_voice_service_with_responses(vec![response]);
        let _guard = crate::buddy::voice_service::install_test_voice_service(service).await;
        let dir = tempfile::tempdir().unwrap();
        let gcx = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
        let mut ctx = test_context(dir.path());
        ctx.pulse
            .diagnostics
            .top_error_types
            .push("Bearer sk-pulsesecret123456".to_string());
        ctx.recent_activities.push(BuddyActivity {
            icon: "•".to_string(),
            title: "Auth trail sk-activitysecret123456".to_string(),
            description: "token=activity-secret and /home/svakhreev/private/file.rs".to_string(),
            timestamp: Utc::now().to_rfc3339(),
            activity_type: "review api_key=activity-kind-value".to_string(),
            chat_id: None,
            failure_category: None,
            failure_summary: None,
        });
        ctx.facts.push(BuddyFact {
            kind: BuddyFactKind::McpAuthExpired,
            key: "auth".to_string(),
            source: "test",
            payload: serde_json::json!({
                "token": "fact-secret-value",
                "nested": {
                    "Authorization": "Bearer nestedsecret",
                    "message": "password=fact-password and sk-factsecret123456",
                },
            }),
            seen_at: Utc::now(),
            confidence: 0.9,
        });
        ctx.workflow_summaries.push(BuddyWorkflowSummary {
            workflow_id: "workflow sk-workflowsecret123456".to_string(),
            last_run: None,
            run_count: 1,
            last_outcome: Some("Bearer workflow-secret and token=workflow-secret".to_string()),
            failure_category: None,
            failure_summary: None,
            ..Default::default()
        });

        let bank = build_chat_phrase_bank(gcx, &ctx, Utc::now())
            .await
            .expect("bank generated");
        let prompts = renderer.prompts();
        let (_, user) = prompts.first().expect("prompt captured");

        for text in [&bank.evidence_summary, user] {
            assert!(
                text.contains("[REDACTED"),
                "redaction marker missing: {text}"
            );
            for secret in [
                "sk-pulsesecret123456",
                "sk-activitysecret123456",
                "activity-secret",
                "/home/svakhreev/private/file.rs",
                "activity-kind-value",
                "fact-secret-value",
                "nestedsecret",
                "fact-password",
                "sk-factsecret123456",
                "sk-workflowsecret123456",
                "workflow-secret",
            ] {
                assert!(
                    !text.contains(secret),
                    "secret leaked into phrase-bank evidence: {secret}"
                );
            }
        }
    }

    #[tokio::test]
    async fn phrase_bank_failed_generation_records_daily_backoff() {
        let (service, _renderer) = crate::buddy::voice_service::test_voice_service_with_responses(
            vec![Some("humor: only one usable kind".to_string())],
        );
        let _guard = crate::buddy::voice_service::install_test_voice_service(service).await;
        let dir = tempfile::tempdir().unwrap();
        let gcx = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
        let mut ctx = test_context(dir.path());
        let job = ChatPhraseBankJob;

        let result = job.execute(gcx, ctx.clone()).await;

        assert!(result.chat_phrase_bank.is_none());
        let last_result = result.last_result.expect("failed run records backoff");
        assert!(last_result.starts_with(FAILED_RESULT_PREFIX));
        ctx.job_state.last_result = Some(last_result);
        assert!(!should_run_at(&ctx, Local::now()));
    }

    #[tokio::test]
    async fn phrase_bank_rejects_underfilled_generated_bank() {
        let response = Some(
            [
                "humor: Tiny task goblin is juggling stuck work with oven mitts.",
                "insight: Pin the stuck task first; the breadcrumb parade needs a marshal.",
                "bug: Error crumbs are circling; bring the detective spoon.",
                "ambient: Pixel is orbiting quietly with a tiny clipboard and snacks.",
            ]
            .join("\n"),
        );
        let (service, _renderer) =
            crate::buddy::voice_service::test_voice_service_with_responses(vec![response]);
        let _guard = crate::buddy::voice_service::install_test_voice_service(service).await;
        let dir = tempfile::tempdir().unwrap();
        let gcx = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
        let ctx = test_context(dir.path());

        let bank = build_chat_phrase_bank(gcx, &ctx, Utc::now()).await;

        assert!(bank.is_none(), "daily bank must have 3 lines per kind");
    }

    #[tokio::test]
    async fn phrase_bank_caps_overfilled_generated_bank_to_expected_lines() {
        let response = Some(
            [
                "humor: Tiny task goblin is juggling stuck work with oven mitts.",
                "humor: MCP crumbs formed a conga line near the logs.",
                "humor: Git gremlins stacked diffs into a tiny wobbly tower.",
                "humor: Extra humor line should not be stored.",
                "insight: Pin the stuck task first; the breadcrumb parade needs a marshal.",
                "insight: Memory cleanup has snack-sized knots worth untangling.",
                "insight: Worktree clutter wants one tidy sweep before more sparkle.",
                "insight: Extra insight line should not be stored.",
                "bug: Error crumbs are circling; bring the detective spoon.",
                "bug: Frontend hiccups look bug-shaped, with suspicious tap shoes.",
                "bug: The failing integration left muddy pawprints in the log hallway.",
                "bug: Extra bug line should not be stored.",
                "ambient: Pixel is orbiting quietly with a tiny clipboard and snacks.",
                "ambient: Companion confetti on standby while the project hums.",
                "ambient: I am watching the trail without nibbling the evidence.",
                "ambient: Extra ambient line should not be stored.",
            ]
            .join("\n"),
        );
        let (service, _renderer) =
            crate::buddy::voice_service::test_voice_service_with_responses(vec![response]);
        let _guard = crate::buddy::voice_service::install_test_voice_service(service).await;
        let dir = tempfile::tempdir().unwrap();
        let gcx = AppState::from_gcx(crate::global_context::tests::make_test_gcx().await).await;
        let ctx = test_context(dir.path());

        let bank = build_chat_phrase_bank(gcx, &ctx, Utc::now())
            .await
            .expect("overfilled bank still has enough usable lines");

        assert_eq!(bank.lines.len(), PHRASE_KINDS.len() * PHRASES_PER_KIND);
        for kind in PHRASE_KINDS {
            assert_eq!(
                bank.lines.iter().filter(|line| line.kind == kind).count(),
                3
            );
        }
        assert!(!bank.lines.iter().any(|line| line.text.contains("Extra")));
    }
}
