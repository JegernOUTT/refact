use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;

use crate::buddy::conversation_ledger::list_all_buddy_conversations;
use crate::buddy::jobs::autonomous_chats::redact_and_cap_text;
use crate::buddy::types::BuddyConversationEntry;
use crate::buddy::user_activity::time_of_day_pattern;
use refact_buddy_core::user_action::UserAction;
use crate::call_validation::{ChatContent, ChatMessage, ContextFile};
use crate::app_state::AppState;
use crate::knowledge_graph::kg_structs::KnowledgeFrontmatter;

pub const BUDDY_PULSE_MARKER: &str = "buddy_project_memory_pulse";
const MAX_MARKDOWN_CHARS: usize = 2000;
const MAX_PROFILE_BYTES: u64 = 64 * 1024;
const MAX_KNOWLEDGE_FILE_BYTES: u64 = 64 * 1024;
const MAX_KNOWLEDGE_AGGREGATE_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BuddyPulsePayload {
    pub preferences: Vec<PulsePreference>,
    pub lessons: Vec<PulseLesson>,
    pub friction: PulseFriction,
    pub recent_reports: Vec<PulseReport>,
    pub user_activity: PulseActivitySection,
    pub generated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PulsePreference {
    pub statement: String,
    pub confidence: f32,
    pub last_updated: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PulseLesson {
    pub title: String,
    pub preview: String,
    pub tags: Vec<String>,
    pub updated: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PulseFriction {
    pub top_error_types: Vec<(String, u32)>,
    pub stuck_tasks: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PulseReport {
    pub workflow_id: String,
    pub title: String,
    pub preview: String,
    pub chat_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PulseActivitySection {
    pub grouped: Vec<PulseActivityGroup>,
    pub time_of_day_pattern: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PulseActivityGroup {
    pub action_type: String,
    pub count: u32,
    pub details: Vec<String>,
}

#[derive(Clone)]
struct UserPreferenceRecord {
    statement: String,
    confidence: f32,
    last_updated: String,
}

struct LessonCandidate {
    lesson: PulseLesson,
    score: f64,
}

pub async fn build_buddy_pulse_payload(gcx: AppState) -> Option<BuddyPulsePayload> {
    let buddy_arc = gcx.buddy.buddy.clone();
    let project_root = {
        let lock = buddy_arc.lock().await;
        let service = lock.as_ref()?;
        service.project_root.clone()
    };

    let preferences = read_preferences(&project_root).await;
    let lessons = read_lessons(&project_root).await;
    let friction = build_friction(gcx.clone()).await;
    let recent_reports = build_recent_reports(&project_root).await;
    let user_activity = build_activity_section(gcx).await;

    let payload = BuddyPulsePayload {
        preferences,
        lessons,
        friction,
        recent_reports,
        user_activity,
        generated_at: redact(Utc::now().to_rfc3339(), 80),
    };

    if payload_is_empty(&payload) {
        None
    } else {
        Some(payload)
    }
}

pub async fn build_buddy_pulse_message(gcx: AppState) -> Option<ChatMessage> {
    let payload = build_buddy_pulse_payload(gcx).await?;
    let file_content = render_pulse_as_markdown(&payload);
    let mut extra = serde_json::Map::new();
    extra.insert(
        "buddy_pulse_payload".to_string(),
        serde_json::to_value(&payload).unwrap_or(serde_json::Value::Null),
    );
    Some(ChatMessage {
        role: "context_file".to_string(),
        content: ChatContent::ContextFiles(vec![ContextFile {
            file_name: "buddy_project_memory_pulse.md".to_string(),
            file_content,
            line1: 0,
            line2: 0,
            ..Default::default()
        }]),
        tool_call_id: BUDDY_PULSE_MARKER.to_string(),
        extra,
        ..Default::default()
    })
}

pub fn render_pulse_as_markdown(payload: &BuddyPulsePayload) -> String {
    let mut sections = Vec::new();

    if !payload.preferences.is_empty() {
        let mut lines = vec!["## USER PREFERENCES".to_string()];
        for pref in payload.preferences.iter().take(5) {
            lines.push(format!(
                "- {:.2}: {} (updated {})",
                pref.confidence,
                redact(&pref.statement, 180),
                redact(&pref.last_updated, 80)
            ));
        }
        sections.push(lines.join("\n"));
    }

    if !payload.lessons.is_empty() {
        let mut lines = vec!["## PROJECT LESSONS".to_string()];
        for lesson in payload.lessons.iter().take(5) {
            let tags = lesson
                .tags
                .iter()
                .take(4)
                .map(|tag| redact(tag, 40))
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!(
                "- {}: {} [{}]",
                redact(&lesson.title, 120),
                redact(&lesson.preview, 180),
                tags
            ));
        }
        sections.push(lines.join("\n"));
    }

    if !payload.friction.top_error_types.is_empty() || payload.friction.stuck_tasks > 0 {
        let mut lines = vec!["## RECENT FRICTION".to_string()];
        if !payload.friction.top_error_types.is_empty() {
            lines.push(format!(
                "- top errors: {}",
                payload
                    .friction
                    .top_error_types
                    .iter()
                    .take(3)
                    .map(|(kind, count)| format!("{} ({})", redact(kind, 60), count))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if payload.friction.stuck_tasks > 0 {
            lines.push(format!("- stuck tasks: {}", payload.friction.stuck_tasks));
        }
        sections.push(lines.join("\n"));
    }

    if !payload.recent_reports.is_empty() {
        let mut lines = vec!["## RECENT BUDDY REPORTS".to_string()];
        for report in payload.recent_reports.iter().take(2) {
            lines.push(format!(
                "- {}: {} ({})",
                redact(&report.title, 120),
                redact(&report.preview, 140),
                redact(&report.chat_id, 80)
            ));
        }
        sections.push(lines.join("\n"));
    }

    if !payload.user_activity.grouped.is_empty() {
        let mut lines = vec!["## USER ACTIVITY (last 24h)".to_string()];
        lines.push(format!(
            "- pattern: {}",
            redact(&payload.user_activity.time_of_day_pattern, 100)
        ));
        for group in payload.user_activity.grouped.iter().take(5) {
            let details = group
                .details
                .iter()
                .take(3)
                .map(|detail| redact(detail, 80))
                .collect::<Vec<_>>()
                .join("; ");
            if details.is_empty() {
                lines.push(format!(
                    "- {}: {}",
                    redact(&group.action_type, 60),
                    group.count
                ));
            } else {
                lines.push(format!(
                    "- {}: {} ({})",
                    redact(&group.action_type, 60),
                    group.count,
                    details
                ));
            }
        }
        sections.push(lines.join("\n"));
    }
    let footer = format!("_Generated {}_", redact(&payload.generated_at, 80));
    let mut section_budgets = vec![420, 470, 260, 310, 460];
    loop {
        let mut out = Vec::new();
        for (section, budget) in sections.iter().zip(section_budgets.iter()) {
            out.push(cap_chars(section, *budget));
        }
        out.push(footer.clone());
        let markdown = out.join("\n\n");
        if markdown.chars().count() <= MAX_MARKDOWN_CHARS {
            return markdown;
        }
        if let Some((idx, _)) = section_budgets.iter().enumerate().max_by_key(|(_, v)| **v) {
            if section_budgets[idx] <= 64 {
                return cap_chars(&markdown, MAX_MARKDOWN_CHARS);
            }
            section_budgets[idx] = section_budgets[idx].saturating_sub(32);
        } else {
            return cap_chars(&markdown, MAX_MARKDOWN_CHARS);
        }
    }
}
async fn read_preferences(project_root: &Path) -> Vec<PulsePreference> {
    let Some(text) = read_checked_project_file(
        &project_root.join(".refact/buddy"),
        &project_root.join(".refact/buddy/user_profile.md"),
        MAX_PROFILE_BYTES,
    )
    .await
    else {
        return Vec::new();
    };
    let mut prefs = parse_user_profile(&text)
        .into_iter()
        .filter(|pref| pref.confidence >= 0.5)
        .map(|pref| PulsePreference {
            statement: redact(pref.statement, 240),
            confidence: pref.confidence,
            last_updated: redact(pref.last_updated, 80),
        })
        .collect::<Vec<_>>();
    prefs.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(Ordering::Equal)
            .then_with(|| b.last_updated.cmp(&a.last_updated))
            .then_with(|| a.statement.cmp(&b.statement))
    });
    prefs.truncate(5);
    prefs
}

fn parse_user_profile(text: &str) -> Vec<UserPreferenceRecord> {
    let mut prefs = Vec::new();
    let mut current: Option<UserPreferenceRecord> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.strip_prefix("## ").is_some() {
            if let Some(pref) = current.take() {
                if !pref.statement.is_empty() {
                    prefs.push(pref);
                }
            }
            current = Some(UserPreferenceRecord {
                statement: String::new(),
                confidence: 0.0,
                last_updated: String::new(),
            });
            continue;
        }
        let Some(pref) = current.as_mut() else {
            continue;
        };
        if let Some(value) = line.strip_prefix("- statement:") {
            pref.statement = parse_profile_value(value);
        } else if let Some(value) = line.strip_prefix("- confidence:") {
            pref.confidence = value.trim().parse::<f32>().unwrap_or(0.0);
        } else if let Some(value) = line.strip_prefix("- last_updated:") {
            pref.last_updated = parse_profile_value(value);
        }
    }
    if let Some(pref) = current {
        if !pref.statement.is_empty() {
            prefs.push(pref);
        }
    }
    prefs
}

fn parse_profile_value(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        value[1..value.len() - 1].replace("\\\"", "\"")
    } else {
        value.to_string()
    }
}

async fn read_lessons(project_root: &Path) -> Vec<PulseLesson> {
    let dir = project_root.join(crate::file_filter::KNOWLEDGE_FOLDER_NAME);
    let Some(root) = canonical_safe_dir(&dir).await else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    let walker = walkdir::WalkDir::new(&dir).follow_links(false);
    for entry in walker.into_iter().filter_map(|entry| entry.ok()) {
        if entry.file_type().is_symlink() {
            continue;
        }
        if entry.file_type().is_dir() {
            continue;
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path().to_path_buf();
        if path_has_component(&path, "archive") {
            continue;
        }
        let ext = path.extension().and_then(|ext| ext.to_str()).unwrap_or("");
        if ext != "md" && ext != "mdx" {
            continue;
        }
        paths.push(path);
    }
    paths.sort();
    let mut lessons = Vec::new();
    let mut aggregate_bytes = 0_u64;
    for path in paths {
        let Some(text) = read_checked_project_file_with_canonical_root(
            &root,
            &path,
            MAX_KNOWLEDGE_FILE_BYTES,
            Some(&mut aggregate_bytes),
            MAX_KNOWLEDGE_AGGREGATE_BYTES,
        )
        .await
        else {
            continue;
        };
        let (frontmatter, content_start) = KnowledgeFrontmatter::parse(&text);
        if frontmatter.is_archived() || frontmatter.is_deprecated() {
            continue;
        }
        if !frontmatter
            .tags
            .iter()
            .any(|tag| matches!(tag.trim().to_lowercase().as_str(), "lesson" | "convention"))
        {
            continue;
        }
        let updated = frontmatter
            .updated
            .clone()
            .or_else(|| frontmatter.created_at.clone())
            .or_else(|| frontmatter.created.clone())
            .unwrap_or_default();
        let content = text.get(content_start..).unwrap_or("");
        let preview = frontmatter
            .description
            .clone()
            .or_else(|| frontmatter.summary.clone())
            .or_else(|| first_nonempty_line(content))
            .unwrap_or_default();
        let title = frontmatter.title.clone().unwrap_or_else(|| {
            path.file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        });
        lessons.push(LessonCandidate {
            score: lesson_score(&updated, frontmatter.source_confidence),
            lesson: PulseLesson {
                title: redact(title, 160),
                preview: redact(preview, 260),
                tags: frontmatter
                    .tags
                    .iter()
                    .map(|tag| redact(tag, 40))
                    .collect::<Vec<_>>(),
                updated: redact(updated, 80),
            },
        });
    }
    lessons.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| b.lesson.updated.cmp(&a.lesson.updated))
            .then_with(|| a.lesson.title.cmp(&b.lesson.title))
    });
    lessons
        .into_iter()
        .take(5)
        .map(|candidate| candidate.lesson)
        .collect()
}

async fn canonical_safe_dir(path: &Path) -> Option<PathBuf> {
    let metadata = tokio::fs::symlink_metadata(path).await.ok()?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return None;
    }
    tokio::fs::canonicalize(path).await.ok()
}

async fn read_checked_project_file(root: &Path, path: &Path, max_bytes: u64) -> Option<String> {
    let root = canonical_safe_dir(root).await?;
    read_checked_project_file_with_canonical_root(&root, path, max_bytes, None, u64::MAX).await
}

async fn read_checked_project_file_with_canonical_root(
    canonical_root: &Path,
    path: &Path,
    max_bytes: u64,
    aggregate_bytes: Option<&mut u64>,
    aggregate_max_bytes: u64,
) -> Option<String> {
    let metadata = tokio::fs::symlink_metadata(path).await.ok()?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > max_bytes {
        return None;
    }
    let canonical_path = tokio::fs::canonicalize(path).await.ok()?;
    if !canonical_path.starts_with(canonical_root) {
        return None;
    }
    if let Some(total) = aggregate_bytes {
        let next_total = total.checked_add(metadata.len())?;
        if next_total > aggregate_max_bytes {
            return None;
        }
        *total = next_total;
    }
    read_bounded_to_string(&canonical_path, max_bytes).await
}

async fn read_bounded_to_string(path: &Path, max_bytes: u64) -> Option<String> {
    let file = tokio::fs::File::open(path).await.ok()?;
    let mut reader = file.take(max_bytes);
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).await.ok()?;
    String::from_utf8(bytes).ok()
}

async fn build_friction(gcx: AppState) -> PulseFriction {
    let buddy_arc = gcx.buddy.buddy.clone();
    let (diagnostics, pulse_diagnostic_types, stuck_tasks) = {
        let lock = buddy_arc.lock().await;
        let Some(service) = lock.as_ref() else {
            return PulseFriction::default();
        };
        (
            service.recent_diagnostics.clone(),
            service.pulse.diagnostics.top_error_types.clone(),
            service.pulse.tasks.stuck,
        )
    };
    let cutoff = Utc::now() - Duration::hours(1);
    let mut counts: HashMap<String, u32> = HashMap::new();
    for diag in &diagnostics {
        let recent = DateTime::parse_from_rfc3339(&diag.collected_at)
            .map(|dt| dt.with_timezone(&Utc) >= cutoff)
            .unwrap_or(true);
        if recent {
            *counts.entry(redact(&diag.error_type, 80)).or_insert(0) += 1;
        }
    }
    for error_type in pulse_diagnostic_types {
        counts.entry(redact(error_type, 80)).or_insert(1);
    }
    let mut top_error_types = counts.into_iter().collect::<Vec<_>>();
    top_error_types.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    top_error_types.truncate(3);
    PulseFriction {
        top_error_types,
        stuck_tasks,
    }
}

async fn build_recent_reports(project_root: &Path) -> Vec<PulseReport> {
    list_all_buddy_conversations(project_root, None)
        .await
        .into_iter()
        .take(2)
        .map(report_from_entry)
        .collect()
}

fn report_from_entry(entry: BuddyConversationEntry) -> PulseReport {
    PulseReport {
        workflow_id: redact(entry.kind, 80),
        title: redact(entry.title, 160),
        preview: redact(
            format!(
                "{}; {} messages; updated {}",
                entry.status, entry.message_count, entry.updated_at
            ),
            180,
        ),
        chat_id: redact(entry.id, 120),
    }
}

async fn build_activity_section(gcx: AppState) -> PulseActivitySection {
    let user_activity = gcx.buddy.user_activity.clone();
    let actions = user_activity.lock().await.last_hours(24);
    if actions.is_empty() {
        return PulseActivitySection {
            grouped: Vec::new(),
            time_of_day_pattern: redact(time_of_day_pattern(&actions), 100),
        };
    }
    PulseActivitySection {
        grouped: group_activity(&actions),
        time_of_day_pattern: redact(time_of_day_pattern(&actions), 100),
    }
}

fn group_activity(actions: &[UserAction]) -> Vec<PulseActivityGroup> {
    let mut grouped: BTreeMap<String, PulseActivityGroup> = BTreeMap::new();
    for action in actions {
        let (action_type, detail) = action_summary(action);
        let group = grouped
            .entry(action_type.clone())
            .or_insert_with(|| PulseActivityGroup {
                action_type,
                count: 0,
                details: Vec::new(),
            });
        group.count = group.count.saturating_add(1);
        if let Some(detail) = detail {
            if group.details.len() < 3 && !group.details.contains(&detail) {
                group.details.push(detail);
            }
        }
    }
    let mut groups = grouped.into_values().collect::<Vec<_>>();
    groups.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.action_type.cmp(&b.action_type))
    });
    groups
}

fn action_summary(action: &UserAction) -> (String, Option<String>) {
    match action {
        UserAction::FileOpened { path, .. } => ("file_opened".to_string(), Some(redact(path, 120))),
        UserAction::SnippetSelected { path, lines, .. } => (
            "snippet_selected".to_string(),
            Some(redact(format!("{}:{}-{}", path, lines.0, lines.1), 140)),
        ),
        UserAction::ToolApproved { tool_name, .. } => {
            ("tool_approved".to_string(), Some(redact(tool_name, 80)))
        }
        UserAction::ToolRejected { tool_name, .. } => {
            ("tool_rejected".to_string(), Some(redact(tool_name, 80)))
        }
        UserAction::CommandRun {
            command_preview, ..
        } => (
            "command_run".to_string(),
            Some(redact(command_preview, 120)),
        ),
        UserAction::WorkspaceChanged {
            folders_added,
            folders_removed,
            ..
        } => (
            "workspace_changed".to_string(),
            Some(redact(
                format!(
                    "+{} -{}",
                    folders_added.join(","),
                    folders_removed.join(",")
                ),
                140,
            )),
        ),
        UserAction::CommitMade {
            sha,
            message_first_line,
            files,
            ..
        } => (
            "commit_made".to_string(),
            Some(redact(
                format!("{} {} files {}", sha, files, message_first_line),
                140,
            )),
        ),
        UserAction::TaskFailed {
            task_id,
            reason_short,
            ..
        } => (
            "task_failed".to_string(),
            Some(redact(format!("{}: {}", task_id, reason_short), 140)),
        ),
        UserAction::ChatStarted {
            first_user_text_preview,
            ..
        } => (
            "chat_started".to_string(),
            Some(redact(first_user_text_preview, 120)),
        ),
    }
}

fn payload_is_empty(payload: &BuddyPulsePayload) -> bool {
    payload.preferences.is_empty()
        && payload.lessons.is_empty()
        && payload.recent_reports.is_empty()
        && payload.user_activity.grouped.is_empty()
        && payload.friction.top_error_types.is_empty()
        && payload.friction.stuck_tasks == 0
}

fn path_has_component(path: &Path, component: &str) -> bool {
    path.components().any(|part| part.as_os_str() == component)
}

fn first_nonempty_line(content: &str) -> Option<String> {
    content
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.trim_start_matches('#').trim().to_string())
}

fn lesson_score(updated: &str, source_confidence: Option<f32>) -> f64 {
    let confidence = source_confidence.unwrap_or(1.0).clamp(0.0, 1.0) as f64;
    let recency = parse_timestamp(updated)
        .map(|dt| {
            let days = Utc::now().signed_duration_since(dt).num_days().max(0) as f64;
            1.0 / (1.0 + days)
        })
        .unwrap_or(0.1);
    recency * confidence
}

fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .ok()
                .and_then(|date| date.and_hms_opt(0, 0, 0))
                .map(|dt| DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc))
        })
}

fn redact(text: impl AsRef<str>, max_chars: usize) -> String {
    redact_and_cap_text(text.as_ref(), max_chars)
}

fn cap_chars(text: &str, max_chars: usize) -> String {
    let mut out = text.chars().take(max_chars).collect::<String>();
    if out.chars().count() < text.chars().count() {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buddy::actor::BuddyService;
    use crate::buddy::runtime_queue::RuntimeQueue;
    use crate::buddy::settings::BuddySettings;
    use chrono::Duration;
    use tokio::sync::broadcast;

    #[cfg(unix)]
    fn symlink_file(from: &Path, to: &Path) {
        std::os::unix::fs::symlink(from, to).unwrap();
    }

    fn valid_profile(statement: &str) -> String {
        format!(
            "# User Profile\n\n## pref\n- statement: \"{}\"\n- confidence: 0.90\n- last_updated: 2026-05-14T10:00:00Z\n",
            statement
        )
    }

    fn valid_lesson(title: &str, body: &str) -> String {
        format!(
            "---\ntitle: \"{}\"\ntags: [\"lesson\"]\nupdated: 2026-05-14\n---\n\n{}\n",
            title, body
        )
    }

    async fn write_knowledge_file(root: &Path, name: &str, content: String) -> PathBuf {
        let path = root
            .join(crate::file_filter::KNOWLEDGE_FOLDER_NAME)
            .join(name);
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, content).await.unwrap();
        path
    }

    async fn make_gcx_with_buddy(project_root: &Path) -> AppState {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let (tx, _) = broadcast::channel(16);
        let service = BuddyService::new(
            project_root.to_path_buf(),
            crate::buddy::state::default_buddy_state(),
            BuddySettings::default(),
            Vec::new(),
            RuntimeQueue::new(),
            tx,
            None,
        );
        let app = AppState::from_gcx(gcx).await;
        let buddy_arc = app.buddy.buddy.clone();
        *buddy_arc.lock().await = Some(service);
        app
    }

    fn full_payload() -> BuddyPulsePayload {
        BuddyPulsePayload {
            preferences: (0..8)
                .map(|idx| PulsePreference {
                    statement: format!(
                        "prefers detailed direct implementation plans with no fallbacks {idx} {}",
                        "x".repeat(160)
                    ),
                    confidence: 0.95,
                    last_updated: "2026-05-14T10:00:00Z".to_string(),
                })
                .collect(),
            lessons: (0..8)
                .map(|idx| PulseLesson {
                    title: format!("Lesson {idx} {}", "y".repeat(80)),
                    preview: format!(
                        "Keep the implementation focused and verified {idx} {}",
                        "z".repeat(180)
                    ),
                    tags: vec!["lesson".to_string(), "convention".to_string()],
                    updated: "2026-05-14".to_string(),
                })
                .collect(),
            friction: PulseFriction {
                top_error_types: vec![
                    ("timeout".to_string(), 4),
                    ("compile_error".to_string(), 3),
                    ("tool_error".to_string(), 2),
                ],
                stuck_tasks: 2,
            },
            recent_reports: vec![
                PulseReport {
                    workflow_id: "buddy_report".to_string(),
                    title: "Report A".to_string(),
                    preview: "A recent report with useful context".to_string(),
                    chat_id: "chat-a".to_string(),
                },
                PulseReport {
                    workflow_id: "buddy_report".to_string(),
                    title: "Report B".to_string(),
                    preview: "Another recent report".to_string(),
                    chat_id: "chat-b".to_string(),
                },
            ],
            user_activity: PulseActivitySection {
                grouped: vec![PulseActivityGroup {
                    action_type: "file_opened".to_string(),
                    count: 6,
                    details: vec!["src/main.rs".to_string(), "src/lib.rs".to_string()],
                }],
                time_of_day_pattern: "mostly active 09–12".to_string(),
            },
            generated_at: "2026-05-14T10:00:00Z".to_string(),
        }
    }

    #[test]
    fn pulse_under_2000_chars_when_full() {
        let markdown = render_pulse_as_markdown(&full_payload());
        assert!(markdown.chars().count() <= 2000);
        assert!(markdown.contains("USER PREFERENCES"));
        assert!(markdown.contains("USER ACTIVITY"));
    }

    #[tokio::test]
    async fn pulse_returns_none_when_empty() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = make_gcx_with_buddy(dir.path()).await;
        let payload = build_buddy_pulse_payload(gcx).await;
        assert!(payload.is_none());
    }

    #[tokio::test]
    async fn pulse_excludes_low_confidence_prefs() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join(".refact/buddy/user_profile.md");
        tokio::fs::create_dir_all(profile.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            &profile,
            "# User Profile\n\n## low\n- statement: \"Ignore me\"\n- confidence: 0.49\n- last_updated: 2026-05-14T09:00:00Z\n\n## high\n- statement: \"Keep me\"\n- confidence: 0.90\n- last_updated: 2026-05-14T10:00:00Z\n",
        )
        .await
        .unwrap();
        let gcx = make_gcx_with_buddy(dir.path()).await;
        let payload = build_buddy_pulse_payload(gcx).await.unwrap();
        assert_eq!(payload.preferences.len(), 1);
        assert_eq!(payload.preferences[0].statement, "Keep me");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pulse_ignores_symlinked_profile_pointing_outside_project() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        tokio::fs::write(outside.path(), valid_profile("Do not read outside"))
            .await
            .unwrap();
        let profile = dir.path().join(".refact/buddy/user_profile.md");
        tokio::fs::create_dir_all(profile.parent().unwrap())
            .await
            .unwrap();
        symlink_file(outside.path(), &profile);

        let prefs = read_preferences(dir.path()).await;

        assert!(prefs.is_empty());
    }

    #[tokio::test]
    async fn pulse_ignores_oversized_profile_before_reading() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join(".refact/buddy/user_profile.md");
        tokio::fs::create_dir_all(profile.parent().unwrap())
            .await
            .unwrap();
        let content = format!(
            "{}{}",
            valid_profile("Too large"),
            "x".repeat(MAX_PROFILE_BYTES as usize)
        );
        tokio::fs::write(&profile, content).await.unwrap();

        let prefs = read_preferences(dir.path()).await;

        assert!(prefs.is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pulse_ignores_symlinked_outside_knowledge_file() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        tokio::fs::write(outside.path(), valid_lesson("Outside", "do not read"))
            .await
            .unwrap();
        let knowledge_dir = dir.path().join(crate::file_filter::KNOWLEDGE_FOLDER_NAME);
        tokio::fs::create_dir_all(&knowledge_dir).await.unwrap();
        symlink_file(outside.path(), &knowledge_dir.join("outside.md"));

        let lessons = read_lessons(dir.path()).await;

        assert!(lessons.is_empty());
    }

    #[tokio::test]
    async fn pulse_ignores_oversized_knowledge_file() {
        let dir = tempfile::tempdir().unwrap();
        let oversized = valid_lesson("Too large", &"x".repeat(MAX_KNOWLEDGE_FILE_BYTES as usize));
        write_knowledge_file(dir.path(), "large.md", oversized).await;

        let lessons = read_lessons(dir.path()).await;

        assert!(lessons.is_empty());
    }

    #[tokio::test]
    async fn pulse_enforces_knowledge_aggregate_cap() {
        let dir = tempfile::tempdir().unwrap();
        let padding = "x".repeat((MAX_KNOWLEDGE_FILE_BYTES - 1024) as usize);
        for idx in 0..5 {
            write_knowledge_file(
                dir.path(),
                &format!("lesson-{idx}.md"),
                valid_lesson(&format!("Lesson {idx}"), &padding),
            )
            .await;
        }

        let lessons = read_lessons(dir.path()).await;

        assert_eq!(lessons.len(), 4);
        assert!(lessons.iter().all(|lesson| lesson.title != "Lesson 4"));
    }

    #[tokio::test]
    async fn pulse_includes_user_activity_section_with_pattern() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = make_gcx_with_buddy(dir.path()).await;
        let user_activity = gcx.buddy.user_activity.clone();
        {
            let mut ring = user_activity.lock().await;
            ring.push(UserAction::FileOpened {
                path: "src/main.rs".to_string(),
                ts: Utc::now() - Duration::minutes(10),
            });
        }
        let payload = build_buddy_pulse_payload(gcx).await.unwrap();
        assert_eq!(payload.user_activity.grouped[0].action_type, "file_opened");
        assert!(payload
            .user_activity
            .time_of_day_pattern
            .starts_with("mostly active"));
    }

    #[tokio::test]
    async fn pulse_message_uses_correct_marker() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join(".refact/buddy/user_profile.md");
        tokio::fs::create_dir_all(profile.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            &profile,
            "# User Profile\n\n## direct\n- statement: \"Prefers direct fixes\"\n- confidence: 0.90\n- last_updated: 2026-05-14T10:00:00Z\n",
        )
        .await
        .unwrap();
        let gcx = make_gcx_with_buddy(dir.path()).await;
        let message = build_buddy_pulse_message(gcx).await.unwrap();
        assert_eq!(message.role, "context_file");
        assert_eq!(message.tool_call_id, BUDDY_PULSE_MARKER);
        match message.content {
            ChatContent::ContextFiles(files) => {
                assert_eq!(files[0].file_name, "buddy_project_memory_pulse.md");
            }
            _ => panic!("expected context files"),
        }
    }

    #[tokio::test]
    async fn pulse_payload_serializes_in_extra_map() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join(".refact/buddy/user_profile.md");
        tokio::fs::create_dir_all(profile.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(
            &profile,
            "# User Profile\n\n## concise\n- statement: \"Likes concise summaries\"\n- confidence: 0.80\n- last_updated: 2026-05-14T10:00:00Z\n",
        )
        .await
        .unwrap();
        let gcx = make_gcx_with_buddy(dir.path()).await;
        let message = build_buddy_pulse_message(gcx).await.unwrap();
        let payload_value = message.extra.get("buddy_pulse_payload").unwrap();
        assert_eq!(
            payload_value["preferences"][0]["statement"],
            "Likes concise summaries"
        );
        assert!(serde_json::to_value(&message)
            .unwrap()
            .get("buddy_pulse_payload")
            .is_some());
    }
}
