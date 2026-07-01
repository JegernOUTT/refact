use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use chrono::{SecondsFormat, Utc};
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;
use tokio::sync::OwnedMutexGuard;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::buddy::jobs::autonomous_chats::redact_and_cap_text;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::files_correction::get_project_dirs;
use crate::global_context::GlobalContext;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

#[derive(Clone, Debug, PartialEq)]
struct UserPreference {
    slug: String,
    statement: String,
    evidence: String,
    confidence: f64,
    last_updated: String,
    updates: u32,
}

pub struct ToolBuddyUserPrefList {
    pub config_path: String,
}

pub struct ToolBuddyUserPrefUpsert {
    pub config_path: String,
}

pub struct ToolBuddyUserPrefRemove {
    pub config_path: String,
}

fn source(config_path: &str) -> ToolSource {
    ToolSource {
        source_type: ToolSourceType::Builtin,
        config_path: config_path.to_string(),
    }
}

fn desc(
    config_path: &str,
    name: &str,
    display_name: &str,
    description: &str,
    input_schema: Value,
) -> ToolDesc {
    ToolDesc {
        name: name.to_string(),
        display_name: display_name.to_string(),
        source: source(config_path),
        experimental: false,
        allow_parallel: false,
        description: description.to_string(),
        input_schema,
        output_schema: None,
        annotations: None,
    }
}

fn result(tool_call_id: &String, text: impl Into<String>) -> (bool, Vec<ContextEnum>) {
    (
        false,
        vec![ContextEnum::ChatMessage(ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(text.into()),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            ..Default::default()
        })],
    )
}

fn required_string_arg<'a>(
    args: &'a HashMap<String, Value>,
    name: &str,
) -> Result<&'a str, String> {
    args.get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("argument `{name}` is missing or not a non-empty string"))
}

fn limited_text_arg(
    args: &HashMap<String, Value>,
    name: &str,
    max_chars: usize,
) -> Result<String, String> {
    let value = required_string_arg(args, name)?;
    if value.chars().count() > max_chars {
        return Err(format!(
            "argument `{name}` must be at most {max_chars} chars"
        ));
    }
    Ok(clean_value(&redact_and_cap_text(value, max_chars)))
}

fn optional_limited_text_arg(
    args: &HashMap<String, Value>,
    name: &str,
    max_chars: usize,
) -> Result<String, String> {
    let Some(value) = args.get(name) else {
        return Ok(String::new());
    };
    let value = value
        .as_str()
        .ok_or_else(|| format!("argument `{name}` must be a string"))?
        .trim();
    if value.chars().count() > max_chars {
        return Err(format!(
            "argument `{name}` must be at most {max_chars} chars"
        ));
    }
    Ok(clean_value(&redact_and_cap_text(value, max_chars)))
}

fn confidence_arg(args: &HashMap<String, Value>) -> Result<f64, String> {
    let confidence = args
        .get("confidence")
        .and_then(Value::as_f64)
        .ok_or_else(|| "argument `confidence` is missing or not a number".to_string())?;
    if !(0.0..=1.0).contains(&confidence) {
        return Err("argument `confidence` must be between 0.0 and 1.0".to_string());
    }
    Ok(confidence)
}

fn top_k_arg(args: &HashMap<String, Value>) -> Result<usize, String> {
    match args.get("top_k") {
        Some(value) => value
            .as_u64()
            .map(|value| (value as usize).min(20))
            .ok_or_else(|| "argument `top_k` must be a non-negative integer".to_string()),
        None => Ok(5),
    }
}

async fn project_root(gcx: Arc<GlobalContext>) -> Result<PathBuf, String> {
    get_project_dirs(gcx)
        .await
        .into_iter()
        .next()
        .ok_or_else(|| "No workspace folder found".to_string())
}

async fn profile_path(gcx: Arc<GlobalContext>) -> Result<PathBuf, String> {
    Ok(project_root(gcx)
        .await?
        .join(".refact")
        .join("buddy")
        .join("user_profile.md"))
}

fn profile_locks() -> &'static AMutex<HashMap<PathBuf, Arc<AMutex<()>>>> {
    static LOCKS: OnceLock<AMutex<HashMap<PathBuf, Arc<AMutex<()>>>>> = OnceLock::new();
    LOCKS.get_or_init(|| AMutex::new(HashMap::new()))
}

async fn lock_for(path: &Path) -> OwnedMutexGuard<()> {
    let mut map = profile_locks().lock().await;
    let lock = map
        .entry(path.to_path_buf())
        .or_insert_with(|| Arc::new(AMutex::new(())))
        .clone();
    drop(map);
    lock.lock_owned().await
}

fn clean_value(value: &str) -> String {
    value
        .replace(['\n', '\r'], " ")
        .replace('"', "'")
        .trim()
        .to_string()
}

fn parse_value(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        value[1..value.len() - 1].replace("\\\"", "\"")
    } else {
        value.to_string()
    }
}

fn parse_user_profile(text: &str) -> Vec<UserPreference> {
    let mut prefs = Vec::new();
    let mut current: Option<UserPreference> = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(slug) = line.strip_prefix("## ") {
            if let Some(pref) = current.take() {
                if !pref.slug.is_empty() && !pref.statement.is_empty() {
                    prefs.push(pref);
                }
            }
            current = Some(UserPreference {
                slug: slug.trim().to_string(),
                statement: String::new(),
                evidence: String::new(),
                confidence: 0.0,
                last_updated: String::new(),
                updates: 0,
            });
            continue;
        }
        let Some(pref) = current.as_mut() else {
            continue;
        };
        if let Some(value) = line.strip_prefix("- statement:") {
            pref.statement = parse_value(value);
        } else if let Some(value) = line.strip_prefix("- evidence:") {
            pref.evidence = parse_value(value);
        } else if let Some(value) = line.strip_prefix("- confidence:") {
            pref.confidence = value.trim().parse::<f64>().unwrap_or(0.0);
        } else if let Some(value) = line.strip_prefix("- last_updated:") {
            pref.last_updated = value.trim().to_string();
        } else if let Some(value) = line.strip_prefix("- updates:") {
            pref.updates = value.trim().parse::<u32>().unwrap_or(0);
        }
    }
    if let Some(pref) = current {
        if !pref.slug.is_empty() && !pref.statement.is_empty() {
            prefs.push(pref);
        }
    }
    prefs
}

fn render_user_profile(prefs: &[UserPreference]) -> String {
    let mut out = String::from("# User Profile\n");
    for pref in prefs {
        out.push('\n');
        out.push_str(&format!("## {}\n", pref.slug));
        out.push_str(&format!("- statement: \"{}\"\n", pref.statement));
        out.push_str(&format!("- evidence: \"{}\"\n", pref.evidence));
        out.push_str(&format!("- confidence: {:.2}\n", pref.confidence));
        out.push_str(&format!("- last_updated: {}\n", pref.last_updated));
        out.push_str(&format!("- updates: {}\n", pref.updates));
    }
    out
}

async fn read_profile(path: &Path) -> Result<Vec<UserPreference>, String> {
    match tokio::fs::read_to_string(path).await {
        Ok(text) => Ok(parse_user_profile(&text)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(err) => Err(format!("failed to read user profile: {err}")),
    }
}

async fn write_profile(path: &Path, prefs: &[UserPreference]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|err| format!("failed to create user profile directory: {err}"))?;
    }
    tokio::fs::write(path, render_user_profile(prefs))
        .await
        .map_err(|err| format!("failed to write user profile: {err}"))
}

fn normalized(value: &str) -> String {
    value
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn slugify(statement: &str) -> String {
    let parts = statement
        .split_whitespace()
        .take(4)
        .filter_map(|word| {
            let part = word
                .chars()
                .filter(|ch| ch.is_ascii_alphanumeric())
                .flat_map(|ch| ch.to_lowercase())
                .collect::<String>();
            (!part.is_empty()).then_some(part)
        })
        .collect::<Vec<_>>();
    if parts.is_empty() {
        "preference".to_string()
    } else {
        parts.join("-")
    }
}

fn unique_slug(prefs: &[UserPreference], statement: &str) -> String {
    let base = slugify(statement);
    let mut candidate = base.clone();
    let mut n = 2;
    while prefs.iter().any(|pref| pref.slug == candidate) {
        candidate = format!("{base}-{n}");
        n += 1;
    }
    candidate
}

fn dedup_index(prefs: &[UserPreference], statement: &str) -> Option<usize> {
    let needle = normalized(statement);
    prefs.iter().position(|pref| {
        let hay = normalized(&pref.statement);
        hay == needle
    })
}

async fn upsert_user_pref_at(
    path: &Path,
    statement: &str,
    evidence: &str,
    confidence: f64,
    now: &str,
) -> Result<(String, bool, u32), String> {
    if !(0.0..=1.0).contains(&confidence) {
        return Err("argument `confidence` must be between 0.0 and 1.0".to_string());
    }
    let _guard = lock_for(path).await;
    let statement = clean_value(statement);
    let evidence = clean_value(evidence);
    let mut prefs = read_profile(path).await?;
    if let Some(index) = dedup_index(&prefs, &statement) {
        let pref = &mut prefs[index];
        pref.statement = statement;
        pref.evidence = evidence;
        pref.confidence = confidence;
        pref.last_updated = now.to_string();
        pref.updates = pref.updates.saturating_add(1);
        let slug = pref.slug.clone();
        let updates = pref.updates;
        write_profile(path, &prefs).await?;
        return Ok((slug, false, updates));
    }
    let slug = unique_slug(&prefs, &statement);
    prefs.push(UserPreference {
        slug: slug.clone(),
        statement,
        evidence,
        confidence,
        last_updated: now.to_string(),
        updates: 1,
    });
    write_profile(path, &prefs).await?;
    Ok((slug, true, 1))
}

async fn remove_user_prefs(path: &Path, substring_match: &str) -> Result<usize, String> {
    let _guard = lock_for(path).await;
    let needle = normalized(substring_match);
    let mut prefs = read_profile(path).await?;
    let before = prefs.len();
    prefs.retain(|pref| {
        let hay = normalized(&format!(
            "{} {} {}",
            pref.slug, pref.statement, pref.evidence
        ));
        !hay.contains(&needle)
    });
    let removed = before - prefs.len();
    write_profile(path, &prefs).await?;
    Ok(removed)
}

fn table_escape(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', " ")
}

fn sorted_for_list(mut prefs: Vec<UserPreference>) -> Vec<UserPreference> {
    prefs.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(Ordering::Equal)
            .then_with(|| b.last_updated.cmp(&a.last_updated))
            .then_with(|| a.slug.cmp(&b.slug))
    });
    prefs
}

fn user_pref_table(prefs: Vec<UserPreference>, top_k: usize) -> String {
    let mut lines = vec![
        "| preference | confidence | updates | last_updated | statement | evidence |".to_string(),
        "|---|---:|---:|---|---|---|".to_string(),
    ];
    for pref in sorted_for_list(prefs).into_iter().take(top_k) {
        lines.push(format!(
            "| {} | {:.2} | {} | {} | {} | {} |",
            table_escape(&pref.slug),
            pref.confidence,
            pref.updates,
            table_escape(&pref.last_updated),
            table_escape(&pref.statement),
            table_escape(&pref.evidence)
        ));
    }
    lines.join("\n")
}

#[async_trait]
impl Tool for ToolBuddyUserPrefList {
    fn tool_description(&self) -> ToolDesc {
        desc(
            &self.config_path,
            "buddy_user_pref_list",
            "Buddy User Preference List",
            "List stored Buddy user preferences as a markdown table.",
            json!({
                "type": "object",
                "properties": {
                    "top_k": {"type": "integer", "default": 5, "minimum": 0, "maximum": 20}
                },
                "additionalProperties": false
            }),
        )
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let gcx = ccx.lock().await.app.gcx.clone();
        let prefs = read_profile(&profile_path(gcx).await?).await?;
        Ok(result(
            tool_call_id,
            user_pref_table(prefs, top_k_arg(args)?),
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolBuddyUserPrefUpsert {
    fn tool_description(&self) -> ToolDesc {
        desc(
            &self.config_path,
            "buddy_user_pref_upsert",
            "Buddy User Preference Upsert",
            "Create or update a stored Buddy user preference.",
            json!({
                "type": "object",
                "properties": {
                    "statement": {"type": "string", "maxLength": 600},
                    "evidence": {"type": "string", "maxLength": 600},
                    "confidence": {"type": "number", "minimum": 0.0, "maximum": 1.0}
                },
                "required": ["statement", "confidence"],
                "additionalProperties": false
            }),
        )
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let statement = limited_text_arg(args, "statement", 600)?;
        let evidence = optional_limited_text_arg(args, "evidence", 600)?;
        let confidence = confidence_arg(args)?;
        let gcx = ccx.lock().await.app.gcx.clone();
        let now = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
        let (slug, created, updates) = upsert_user_pref_at(
            &profile_path(gcx).await?,
            &statement,
            &evidence,
            confidence,
            &now,
        )
        .await?;
        let action = if created { "created" } else { "updated" };
        Ok(result(
            tool_call_id,
            format!("Preference {slug} {action}; updates={updates}"),
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolBuddyUserPrefRemove {
    fn tool_description(&self) -> ToolDesc {
        desc(
            &self.config_path,
            "buddy_user_pref_remove",
            "Buddy User Preference Remove",
            "Remove stored Buddy user preferences by case-insensitive substring match.",
            json!({
                "type": "object",
                "properties": {
                    "substring_match": {"type": "string"}
                },
                "required": ["substring_match"],
                "additionalProperties": false
            }),
        )
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let substring = required_string_arg(args, "substring_match")?;
        let gcx = ccx.lock().await.app.gcx.clone();
        let removed = remove_user_prefs(&profile_path(gcx).await?, substring).await?;
        Ok(result(
            tool_call_id,
            format!("Removed {removed} preferences"),
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn buddy_user_pref_upsert_creates_new_then_increments_on_dedup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("user_profile.md");
        let first = upsert_user_pref_at(
            &path,
            "Prefers Rust over Python for systems work",
            "Picked Rust repeatedly",
            0.85,
            "2026-05-14T16:00:00Z",
        )
        .await
        .unwrap();
        let second = upsert_user_pref_at(
            &path,
            "prefers rust over python",
            "Chose Rust again",
            0.90,
            "2026-05-14T17:00:00Z",
        )
        .await
        .unwrap();
        let prefs = read_profile(&path).await.unwrap();
        assert!(first.1);
        assert!(!second.1);
        assert_eq!(prefs.len(), 1);
        assert_eq!(prefs[0].updates, 2);
        assert_eq!(prefs[0].statement, "prefers rust over python");
    }

    #[tokio::test]
    async fn buddy_user_pref_upsert_allows_empty_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("user_profile.md");
        upsert_user_pref_at(
            &path,
            "Prefers terse explanations",
            "",
            0.75,
            "2026-05-14T16:00:00Z",
        )
        .await
        .unwrap();

        let prefs = read_profile(&path).await.unwrap();
        assert_eq!(prefs.len(), 1);
        assert_eq!(prefs[0].evidence, "");
    }

    #[tokio::test]
    async fn buddy_user_pref_upsert_does_not_merge_substring_preferences() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("user_profile.md");
        upsert_user_pref_at(
            &path,
            "Prefers Rust",
            "Evidence",
            0.80,
            "2026-05-14T16:00:00Z",
        )
        .await
        .unwrap();
        upsert_user_pref_at(
            &path,
            "Prefers Rust for systems code but Python for ML pipelines and data exploration",
            "Evidence",
            0.85,
            "2026-05-14T17:00:00Z",
        )
        .await
        .unwrap();

        let prefs = read_profile(&path).await.unwrap();
        assert_eq!(prefs.len(), 2);
    }

    #[tokio::test]
    async fn upsert_user_pref_serializes_concurrent_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = Arc::new(dir.path().join("user_profile.md"));
        let mut handles = Vec::new();
        for index in 0..5 {
            let path = path.clone();
            handles.push(tokio::spawn(async move {
                upsert_user_pref_at(
                    path.as_ref(),
                    &format!("Preference {index}"),
                    "Concurrent write",
                    0.80,
                    &format!("2026-05-14T10:00:0{index}Z"),
                )
                .await
                .unwrap();
            }));
        }
        for handle in handles {
            handle.await.unwrap();
        }
        let prefs = read_profile(path.as_ref()).await.unwrap();
        assert_eq!(prefs.len(), 5);
        for index in 0..5 {
            assert!(prefs
                .iter()
                .any(|pref| pref.statement == format!("Preference {index}")));
        }
    }

    #[tokio::test]
    async fn buddy_user_pref_list_returns_top_k_by_confidence_then_recency() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("user_profile.md");
        upsert_user_pref_at(
            &path,
            "Uses concise answers",
            "Asked for brevity",
            0.70,
            "2026-05-14T10:00:00Z",
        )
        .await
        .unwrap();
        upsert_user_pref_at(
            &path,
            "Likes detailed plans",
            "Approved planning",
            0.90,
            "2026-05-14T09:00:00Z",
        )
        .await
        .unwrap();
        upsert_user_pref_at(
            &path,
            "Prefers direct fixes",
            "Rejected fallback",
            0.90,
            "2026-05-14T11:00:00Z",
        )
        .await
        .unwrap();
        let table = user_pref_table(read_profile(&path).await.unwrap(), 2);
        let direct = table.find("Prefers direct fixes").unwrap();
        let detailed = table.find("Likes detailed plans").unwrap();
        assert!(direct < detailed);
        assert!(!table.contains("Uses concise answers"));
    }

    #[tokio::test]
    async fn buddy_user_pref_remove_by_substring_works() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("user_profile.md");
        upsert_user_pref_at(
            &path,
            "Prefers Rust",
            "Evidence",
            0.8,
            "2026-05-14T10:00:00Z",
        )
        .await
        .unwrap();
        upsert_user_pref_at(
            &path,
            "Likes Python notebooks",
            "Evidence",
            0.7,
            "2026-05-14T10:00:00Z",
        )
        .await
        .unwrap();
        let removed = remove_user_prefs(&path, "rust").await.unwrap();
        let prefs = read_profile(&path).await.unwrap();
        assert_eq!(removed, 1);
        assert_eq!(prefs.len(), 1);
        assert_eq!(prefs[0].statement, "Likes Python notebooks");
    }

    #[tokio::test]
    async fn buddy_user_pref_upsert_rejects_invalid_confidence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("user_profile.md");
        let err = upsert_user_pref_at(
            &path,
            "Prefers Rust",
            "Evidence",
            1.2,
            "2026-05-14T10:00:00Z",
        )
        .await
        .unwrap_err();
        assert!(err.contains("confidence"));
    }
}
