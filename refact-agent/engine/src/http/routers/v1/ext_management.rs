use axum::Extension;
use axum::extract::{Path, Query};
use axum::response::Result;
use hyper::{Body, Response, StatusCode};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::RwLock as ARwLock;

use crate::custom_error::ScratchError;
use crate::ext::config_dirs::{CommandSource, ExtDirs, get_ext_dirs};
use crate::ext::hooks::{HookConfig, HookEvent, load_hooks};
use crate::ext::skills::{load_skill_full, load_skill_indices};
use crate::ext::slash_commands::{load_slash_commands, parse_frontmatter_and_body};
use crate::files_correction::get_project_dirs;
use crate::global_context::GlobalContext;
use crate::http::routers::v1::at_commands::invalidate_slash_cache;
use crate::buddy::drafts::{draft_kind_str, DraftTarget, DraftValidationError};
use crate::buddy::types::DraftKind;

#[derive(serde::Serialize)]
struct DraftMetadata {
    draft_id: String,
    kind: String,
    title: String,
    explanation: String,
    source_opportunity_id: Option<String>,
    expires_at: String,
}

#[derive(serde::Serialize)]
struct DraftExtResponse {
    data: serde_json::Value,
    draft_metadata: DraftMetadata,
}

#[derive(serde::Serialize)]
struct ConsumedDraft {
    draft_id: String,
}

fn json_error(status: StatusCode, msg: &str) -> Result<Response<Body>, ScratchError> {
    let body = serde_json::json!({"error": msg});
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

fn json_response<T: Serialize>(
    status: StatusCode,
    data: &T,
) -> Result<Response<Body>, ScratchError> {
    let body_str = serde_json::to_string(data).map_err(|e| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("JSON serialization error: {}", e),
        )
    })?;
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Body::from(body_str))
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

fn draft_error_response(err: DraftValidationError) -> Result<Response<Body>, ScratchError> {
    match err {
        DraftValidationError::NotFound => json_error(StatusCode::NOT_FOUND, "draft_not_found"),
        DraftValidationError::KindMismatch { expected, actual } => json_error(
            StatusCode::CONFLICT,
            &format!(
                "draft_kind_mismatch: expected {}, got {}",
                draft_kind_str(&expected),
                draft_kind_str(&actual)
            ),
        ),
        DraftValidationError::TargetMismatch { expected, actual } => json_error(
            StatusCode::CONFLICT,
            &format!(
                "draft_target_mismatch: expected {}, got {}",
                expected, actual
            ),
        ),
        DraftValidationError::Parse(err) => json_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            &format!("draft_parse_failed: {}", err),
        ),
    }
}

async fn validate_draft_for_target(
    gcx: Arc<ARwLock<GlobalContext>>,
    draft_id: &str,
    expected_kind: DraftKind,
    target: DraftTarget<'_>,
) -> std::result::Result<(), DraftValidationError> {
    let buddy_arc = gcx.read().await.buddy.clone();
    let lock = buddy_arc.lock().await;
    let Some(svc) = lock.as_ref() else {
        return Err(DraftValidationError::NotFound);
    };
    svc.draft_store
        .get_validated(draft_id, expected_kind, target)
        .map(|_| ())
}

async fn consume_draft_for_target(
    gcx: Arc<ARwLock<GlobalContext>>,
    draft_id: &str,
    expected_kind: DraftKind,
    target: DraftTarget<'_>,
) -> std::result::Result<ConsumedDraft, DraftValidationError> {
    let buddy_arc = gcx.read().await.buddy.clone();
    let mut lock = buddy_arc.lock().await;
    let Some(svc) = lock.as_mut() else {
        return Err(DraftValidationError::NotFound);
    };
    svc.consume_validated_draft(draft_id, expected_kind, target)
        .map(|d| ConsumedDraft { draft_id: d.id })
}

fn source_str(source: &CommandSource) -> String {
    match source {
        CommandSource::GlobalClaude => "global_claude".to_string(),
        CommandSource::GlobalRefact => "global_refact".to_string(),
        CommandSource::ProjectClaude(_) => "project_claude".to_string(),
        CommandSource::ProjectRefact(_) => "project_refact".to_string(),
        CommandSource::InstalledPlugin(name) => format!("plugin:{}", name),
    }
}

fn source_label_str(source: &CommandSource) -> String {
    match source {
        CommandSource::GlobalClaude => "Global (.claude)".to_string(),
        CommandSource::GlobalRefact => "Global".to_string(),
        CommandSource::ProjectClaude(_) => "Project (.claude)".to_string(),
        CommandSource::ProjectRefact(_) => "Project (.refact)".to_string(),
        CommandSource::InstalledPlugin(name) => format!("Plugin ({})", name),
    }
}

fn scope_str(source: &CommandSource) -> String {
    match source {
        CommandSource::GlobalClaude | CommandSource::GlobalRefact => "global".to_string(),
        CommandSource::ProjectClaude(_) | CommandSource::ProjectRefact(_) => "local".to_string(),
        CommandSource::InstalledPlugin(_) => "plugin".to_string(),
    }
}

fn is_read_only(source: &CommandSource) -> bool {
    matches!(
        source,
        CommandSource::GlobalClaude
            | CommandSource::ProjectClaude(_)
            | CommandSource::InstalledPlugin(_)
    )
}

fn skill_file_path(source: &CommandSource, config_dir: &std::path::Path, name: &str) -> String {
    match source {
        CommandSource::GlobalRefact => config_dir
            .join("skills")
            .join(name)
            .join("SKILL.md")
            .display()
            .to_string(),
        CommandSource::GlobalClaude => home::home_dir()
            .map(|h| {
                h.join(".claude")
                    .join("skills")
                    .join(name)
                    .join("SKILL.md")
                    .display()
                    .to_string()
            })
            .unwrap_or_default(),
        CommandSource::ProjectRefact(root) => root
            .join(".refact")
            .join("skills")
            .join(name)
            .join("SKILL.md")
            .display()
            .to_string(),
        CommandSource::ProjectClaude(root) => root
            .join(".claude")
            .join("skills")
            .join(name)
            .join("SKILL.md")
            .display()
            .to_string(),
        CommandSource::InstalledPlugin(plugin_name) => config_dir
            .join("plugins")
            .join("installed")
            .join(plugin_name)
            .join("skills")
            .join(name)
            .join("SKILL.md")
            .display()
            .to_string(),
    }
}

fn validate_name(name: &str) -> std::result::Result<(), String> {
    if name.is_empty() {
        return Err("name cannot be empty".to_string());
    }
    if name.starts_with('.') {
        return Err("name cannot start with '.'".to_string());
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err("name contains invalid path characters".to_string());
    }
    Ok(())
}

pub(crate) async fn write_file_atomic(
    path: &std::path::Path,
    content: &str,
) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    tokio::fs::create_dir_all(parent).await?;
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("ext");
    let tmp_path = parent.join(format!(".{}.{}.tmp", file_name, uuid::Uuid::new_v4()));
    let write_result = async {
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)
            .await?;
        file.write_all(content.as_bytes()).await?;
        file.flush().await?;
        tokio::fs::rename(&tmp_path, path).await
    }
    .await;
    if write_result.is_err() {
        let _ = tokio::fs::remove_file(&tmp_path).await;
    }
    write_result
}

async fn resolve_scope_dir(
    gcx: Arc<ARwLock<GlobalContext>>,
    scope: Option<&str>,
    require_local: bool,
) -> std::result::Result<(PathBuf, String), String> {
    let config_dir = gcx.read().await.config_dir.clone();
    let project_dirs = get_project_dirs(gcx.clone()).await;
    let project_root = project_dirs.into_iter().next();

    match scope {
        Some("global") => Ok((config_dir, "global".to_string())),
        Some("local") => match project_root {
            Some(root) => Ok((root.join(".refact"), "local".to_string())),
            None => Err("no project root for local scope".to_string()),
        },
        Some(other) => Err(format!(
            "invalid scope: '{}'; expected 'global' or 'local'",
            other
        )),
        None => {
            if require_local {
                match project_root {
                    Some(root) => Ok((root.join(".refact"), "local".to_string())),
                    None => Ok((config_dir, "global".to_string())),
                }
            } else {
                Ok((config_dir, "global".to_string()))
            }
        }
    }
}

fn make_scope_ext_dirs(base_dir: PathBuf, scope: &str) -> ExtDirs {
    if scope == "global" {
        ExtDirs {
            global_dirs: vec![base_dir],
            project_dirs: vec![],
            installed_dirs: vec![],
        }
    } else {
        ExtDirs {
            global_dirs: vec![],
            project_dirs: vec![base_dir],
            installed_dirs: vec![],
        }
    }
}

fn hook_event_to_str(event: &HookEvent) -> &'static str {
    match event {
        HookEvent::PreToolUse => "PreToolUse",
        HookEvent::PostToolUse => "PostToolUse",
        HookEvent::UserPromptSubmit => "UserPromptSubmit",
        HookEvent::SessionStart => "SessionStart",
        HookEvent::SessionEnd => "SessionEnd",
        HookEvent::Stop => "Stop",
        HookEvent::SubagentStop => "SubagentStop",
        HookEvent::Notification => "Notification",
        HookEvent::PreCompact => "PreCompact",
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookConfigFlat {
    pub event: HookEvent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matcher: Option<String>,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
}

impl From<&HookConfig> for HookConfigFlat {
    fn from(h: &HookConfig) -> Self {
        HookConfigFlat {
            event: h.event.clone(),
            matcher: h.matcher.clone(),
            command: h.command.clone(),
            timeout: h.timeout,
        }
    }
}

fn hooks_to_yaml_string(hooks: &[HookConfigFlat]) -> std::result::Result<String, String> {
    use serde_yaml::Value;

    let mut by_event: std::collections::HashMap<String, Vec<&HookConfigFlat>> =
        std::collections::HashMap::new();
    for hook in hooks {
        by_event
            .entry(hook_event_to_str(&hook.event).to_string())
            .or_default()
            .push(hook);
    }

    if by_event.is_empty() {
        return Ok("hooks: {}\n".to_string());
    }

    let mut hooks_map = serde_yaml::Mapping::new();
    let mut event_order: Vec<String> = by_event.keys().cloned().collect();
    event_order.sort();
    for event_str in event_order {
        let hook_list = &by_event[&event_str];
        let mut entries = Vec::new();
        for hook in hook_list {
            let mut hook_cmd = serde_yaml::Mapping::new();
            hook_cmd.insert(
                Value::String("type".to_string()),
                Value::String("command".to_string()),
            );
            hook_cmd.insert(
                Value::String("command".to_string()),
                Value::String(hook.command.clone()),
            );
            if let Some(t) = hook.timeout {
                hook_cmd.insert(
                    Value::String("timeout".to_string()),
                    Value::Number(serde_yaml::Number::from(t)),
                );
            }
            let mut entry = serde_yaml::Mapping::new();
            if let Some(m) = &hook.matcher {
                entry.insert(
                    Value::String("matcher".to_string()),
                    Value::String(m.clone()),
                );
            }
            entry.insert(
                Value::String("hooks".to_string()),
                Value::Sequence(vec![Value::Mapping(hook_cmd)]),
            );
            entries.push(Value::Mapping(entry));
        }
        hooks_map.insert(Value::String(event_str), Value::Sequence(entries));
    }

    let mut root = serde_yaml::Mapping::new();
    root.insert(
        Value::String("hooks".to_string()),
        Value::Mapping(hooks_map),
    );
    serde_yaml::to_string(&Value::Mapping(root)).map_err(|e| e.to_string())
}

fn build_skill_frontmatter(
    existing_fm: serde_yaml::Value,
    name: &str,
    description: Option<&str>,
    user_invocable: Option<bool>,
    disable_model_invocation: Option<bool>,
    argument_hint: Option<&str>,
    allowed_tools: Option<&[String]>,
    model: Option<Option<&str>>,
    context: Option<Option<&str>>,
    agent: Option<Option<&str>>,
) -> serde_yaml::Value {
    let mut map = match existing_fm {
        serde_yaml::Value::Mapping(m) => m,
        _ => serde_yaml::Mapping::new(),
    };
    map.insert(
        serde_yaml::Value::String("name".to_string()),
        serde_yaml::Value::String(name.to_string()),
    );
    if let Some(d) = description {
        map.insert(
            serde_yaml::Value::String("description".to_string()),
            serde_yaml::Value::String(d.to_string()),
        );
    }
    if let Some(ui) = user_invocable {
        map.insert(
            serde_yaml::Value::String("user-invocable".to_string()),
            serde_yaml::Value::Bool(ui),
        );
    }
    if let Some(dmi) = disable_model_invocation {
        map.insert(
            serde_yaml::Value::String("disable-model-invocation".to_string()),
            serde_yaml::Value::Bool(dmi),
        );
    }
    if let Some(ah) = argument_hint {
        if ah.is_empty() {
            map.remove(serde_yaml::Value::String("argument-hint".to_string()));
        } else {
            map.insert(
                serde_yaml::Value::String("argument-hint".to_string()),
                serde_yaml::Value::String(ah.to_string()),
            );
        }
    }
    if let Some(tools) = allowed_tools {
        if tools.is_empty() {
            map.remove(serde_yaml::Value::String("allowed-tools".to_string()));
        } else {
            let seq = tools
                .iter()
                .map(|t| serde_yaml::Value::String(t.clone()))
                .collect();
            map.insert(
                serde_yaml::Value::String("allowed-tools".to_string()),
                serde_yaml::Value::Sequence(seq),
            );
        }
    }
    if let Some(m_opt) = model {
        match m_opt {
            Some(m) => {
                map.insert(
                    serde_yaml::Value::String("model".to_string()),
                    serde_yaml::Value::String(m.to_string()),
                );
            }
            None => {
                map.remove(serde_yaml::Value::String("model".to_string()));
            }
        }
    }
    if let Some(c_opt) = context {
        match c_opt {
            Some(c) => {
                map.insert(
                    serde_yaml::Value::String("context".to_string()),
                    serde_yaml::Value::String(c.to_string()),
                );
            }
            None => {
                map.remove(serde_yaml::Value::String("context".to_string()));
            }
        }
    }
    if let Some(a_opt) = agent {
        match a_opt {
            Some(a) => {
                map.insert(
                    serde_yaml::Value::String("agent".to_string()),
                    serde_yaml::Value::String(a.to_string()),
                );
            }
            None => {
                map.remove(serde_yaml::Value::String("agent".to_string()));
            }
        }
    }
    serde_yaml::Value::Mapping(map)
}

fn build_command_frontmatter(
    existing_fm: serde_yaml::Value,
    description: Option<&str>,
    argument_hint: Option<&str>,
    allowed_tools: Option<&[String]>,
    model: Option<Option<&str>>,
) -> serde_yaml::Value {
    let mut map = match existing_fm {
        serde_yaml::Value::Mapping(m) => m,
        _ => serde_yaml::Mapping::new(),
    };
    if let Some(d) = description {
        map.insert(
            serde_yaml::Value::String("description".to_string()),
            serde_yaml::Value::String(d.to_string()),
        );
    }
    if let Some(ah) = argument_hint {
        if ah.is_empty() {
            map.remove(serde_yaml::Value::String("argument-hint".to_string()));
        } else {
            map.insert(
                serde_yaml::Value::String("argument-hint".to_string()),
                serde_yaml::Value::String(ah.to_string()),
            );
        }
    }
    if let Some(tools) = allowed_tools {
        if tools.is_empty() {
            map.remove(serde_yaml::Value::String("allowed-tools".to_string()));
        } else {
            let seq = tools
                .iter()
                .map(|t| serde_yaml::Value::String(t.clone()))
                .collect();
            map.insert(
                serde_yaml::Value::String("allowed-tools".to_string()),
                serde_yaml::Value::Sequence(seq),
            );
        }
    }
    if let Some(m_opt) = model {
        match m_opt {
            Some(m) => {
                map.insert(
                    serde_yaml::Value::String("model".to_string()),
                    serde_yaml::Value::String(m.to_string()),
                );
            }
            None => {
                map.remove(serde_yaml::Value::String("model".to_string()));
            }
        }
    }
    serde_yaml::Value::Mapping(map)
}

fn serialize_md_content(fm: &serde_yaml::Value, body: &str) -> std::result::Result<String, String> {
    let is_empty_map = fm.as_mapping().map(|m| m.is_empty()).unwrap_or(true);
    if is_empty_map {
        return Ok(body.to_string());
    }
    let fm_str = serde_yaml::to_string(fm).map_err(|e| e.to_string())?;
    Ok(format!("---\n{}---\n{}", fm_str, body))
}

fn validate_raw_skill_name(name: &str, raw: &str) -> std::result::Result<(), String> {
    let (frontmatter, _) = parse_frontmatter_and_body(raw);
    let Some(raw_name) = frontmatter.get("name").and_then(|v| v.as_str()) else {
        return Err("raw skill content must include frontmatter name".to_string());
    };
    if raw_name != name {
        return Err(format!(
            "raw skill content name '{}' does not match route '{}'",
            raw_name, name
        ));
    }
    Ok(())
}

fn validate_raw_hooks(raw: &str) -> std::result::Result<(), String> {
    serde_yaml::from_str::<serde_yaml::Value>(raw)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[derive(Serialize)]
pub struct RegistrySkillEntry {
    pub name: String,
    pub description: String,
    pub source: String,
    pub source_label: String,
    pub scope: String,
    pub read_only: bool,
    pub file_path: String,
}

#[derive(Serialize)]
pub struct RegistryCommandEntry {
    pub name: String,
    pub description: String,
    pub source: String,
    pub source_label: String,
    pub scope: String,
    pub read_only: bool,
    pub file_path: String,
}

#[derive(Serialize)]
pub struct RegistryHookEntry {
    pub event: HookEvent,
    pub matcher: Option<String>,
    pub command: String,
    pub timeout: Option<u64>,
    pub source: String,
    pub source_label: String,
    pub scope: String,
    pub read_only: bool,
}

#[derive(Serialize)]
pub struct ExtRegistryResponse {
    pub skills: Vec<RegistrySkillEntry>,
    pub slash_commands: Vec<RegistryCommandEntry>,
    pub hooks: Vec<RegistryHookEntry>,
    pub has_project_root: bool,
}

pub async fn handle_v1_ext_registry(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<Response<Body>, ScratchError> {
    let config_dir = gcx.read().await.config_dir.clone();
    let ext_dirs = get_ext_dirs(gcx.clone()).await;

    let skill_indices = load_skill_indices(&ext_dirs).await;
    let commands = load_slash_commands(&ext_dirs).await;
    let hooks = load_hooks(&ext_dirs).await;
    let has_project_root = !get_project_dirs(gcx.clone()).await.is_empty();

    let skills: Vec<RegistrySkillEntry> = skill_indices
        .iter()
        .filter(|s| {
            !matches!(
                s.source,
                CommandSource::GlobalClaude | CommandSource::ProjectClaude(_)
            )
        })
        .map(|s| RegistrySkillEntry {
            name: s.name.clone(),
            description: s.description.clone(),
            source: source_str(&s.source),
            source_label: source_label_str(&s.source),
            scope: scope_str(&s.source),
            read_only: is_read_only(&s.source),
            file_path: skill_file_path(&s.source, &config_dir, &s.name),
        })
        .collect();

    let slash_commands: Vec<RegistryCommandEntry> = commands
        .iter()
        .filter(|c| {
            !matches!(
                c.source,
                CommandSource::GlobalClaude | CommandSource::ProjectClaude(_)
            )
        })
        .map(|c| RegistryCommandEntry {
            name: c.name.clone(),
            description: c.description.clone(),
            source: source_str(&c.source),
            source_label: source_label_str(&c.source),
            scope: scope_str(&c.source),
            read_only: is_read_only(&c.source),
            file_path: c.file_path.display().to_string(),
        })
        .collect();

    let hooks_entries: Vec<RegistryHookEntry> = hooks
        .iter()
        .map(|h| RegistryHookEntry {
            event: h.event.clone(),
            matcher: h.matcher.clone(),
            command: h.command.clone(),
            timeout: h.timeout,
            source: source_str(&h.source),
            source_label: source_label_str(&h.source),
            scope: scope_str(&h.source),
            read_only: is_read_only(&h.source),
        })
        .collect();

    json_response(
        StatusCode::OK,
        &ExtRegistryResponse {
            skills,
            slash_commands,
            hooks: hooks_entries,
            has_project_root,
        },
    )
}

#[derive(Deserialize)]
pub struct ScopeQuery {
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub draft_id: Option<String>,
}

#[derive(Serialize)]
pub struct SkillDetailResponse {
    pub name: String,
    pub description: String,
    pub user_invocable: bool,
    pub disable_model_invocation: bool,
    pub allowed_tools: Vec<String>,
    pub model: Option<String>,
    pub context: Option<String>,
    pub agent: Option<String>,
    pub argument_hint: String,
    pub body: String,
    pub raw_content: String,
    pub source: String,
    pub file_path: String,
}

pub async fn handle_v1_ext_skill_get(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(name): Path<String>,
    Query(query): Query<ScopeQuery>,
) -> Result<Response<Body>, ScratchError> {
    if let Err(e) = validate_name(&name) {
        return json_error(StatusCode::BAD_REQUEST, &e);
    }

    if let Some(ref draft_id) = query.draft_id {
        let draft_data = {
            let buddy_arc = gcx.read().await.buddy.clone();
            let lock = buddy_arc.lock().await;
            match lock.as_ref() {
                Some(svc) => match svc.draft_store.get_validated(
                    draft_id,
                    DraftKind::Skill,
                    DraftTarget::Id(&name),
                ) {
                    Ok(draft) => Ok((
                        draft.id.clone(),
                        draft.kind,
                        draft.title.clone(),
                        draft.explanation.clone(),
                        draft.expires_at,
                        draft.yaml_or_json.clone(),
                    )),
                    Err(err) => Err(err),
                },
                None => Err(DraftValidationError::NotFound),
            }
        };
        return match draft_data {
            Err(err) => draft_error_response(err),
            Ok((did, dkind, title, explanation, expires_at, yaml_or_json)) => {
                let data: serde_json::Value = match serde_yaml::from_str(&yaml_or_json) {
                    Ok(v) => v,
                    Err(e) => {
                        return json_error(
                            StatusCode::UNPROCESSABLE_ENTITY,
                            &format!("draft_parse_failed: {}", e),
                        )
                    }
                };
                json_response(
                    StatusCode::OK,
                    &DraftExtResponse {
                        data,
                        draft_metadata: DraftMetadata {
                            draft_id: did,
                            kind: draft_kind_str(&dkind).to_string(),
                            title,
                            explanation,
                            source_opportunity_id: None,
                            expires_at: expires_at.to_rfc3339(),
                        },
                    },
                )
            }
        };
    }

    let config_dir = gcx.read().await.config_dir.clone();
    let ext_dirs = match query.scope.as_deref() {
        Some(s @ "global") | Some(s @ "local") => {
            match resolve_scope_dir(gcx.clone(), Some(s), true).await {
                Ok((dir, scope)) => make_scope_ext_dirs(dir, &scope),
                Err(e) => return json_error(StatusCode::BAD_REQUEST, &e),
            }
        }
        _ => get_ext_dirs(gcx.clone()).await,
    };

    let skill = load_skill_full(&ext_dirs, &name).await;
    match skill {
        None => json_error(StatusCode::NOT_FOUND, "skill not found"),
        Some(full) => {
            let file_path = full.skill_dir.join("SKILL.md");
            let raw_content = tokio::fs::read_to_string(&file_path)
                .await
                .unwrap_or_default();
            json_response(
                StatusCode::OK,
                &SkillDetailResponse {
                    name: full.index.name.clone(),
                    description: full.index.description.clone(),
                    user_invocable: full.index.user_invocable,
                    disable_model_invocation: full.index.disable_model_invocation,
                    allowed_tools: full.allowed_tools.clone(),
                    model: full.model.clone(),
                    context: full.context.clone(),
                    agent: full.agent.clone(),
                    argument_hint: full.argument_hint.clone(),
                    body: full.body.clone(),
                    raw_content,
                    source: source_str(&full.index.source),
                    file_path: skill_file_path(&full.index.source, &config_dir, &full.index.name),
                },
            )
        }
    }
}

#[derive(Deserialize)]
pub struct SaveSkillRequest {
    #[serde(default)]
    pub raw_content: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub user_invocable: Option<bool>,
    #[serde(default)]
    pub disable_model_invocation: Option<bool>,
    #[serde(default)]
    pub allowed_tools: Option<Vec<String>>,
    #[serde(default)]
    pub model: Option<serde_json::Value>,
    #[serde(default)]
    pub context: Option<serde_json::Value>,
    #[serde(default)]
    pub agent: Option<serde_json::Value>,
    #[serde(default)]
    pub argument_hint: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub draft_id: Option<String>,
}

pub async fn handle_v1_ext_skill_put(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(name): Path<String>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    if let Err(e) = validate_name(&name) {
        return json_error(StatusCode::BAD_REQUEST, &e);
    }
    let req: SaveSkillRequest = match serde_json::from_slice(&body_bytes) {
        Ok(r) => r,
        Err(e) => return json_error(StatusCode::BAD_REQUEST, &e.to_string()),
    };

    let scope_str_val = req.scope.as_deref().unwrap_or("local");
    let (base_dir, scope_name) =
        match resolve_scope_dir(gcx.clone(), Some(scope_str_val), true).await {
            Ok(d) => d,
            Err(e) => return json_error(StatusCode::BAD_REQUEST, &e),
        };

    let skill_dir = base_dir.join("skills").join(&name);
    let file_path = skill_dir.join("SKILL.md");

    if let Some(ref draft_id) = req.draft_id {
        if let Err(err) = validate_draft_for_target(
            gcx.clone(),
            draft_id,
            DraftKind::Skill,
            DraftTarget::Id(&name),
        )
        .await
        {
            return draft_error_response(err);
        }
    }

    if let Some(raw) = &req.raw_content {
        if let Err(e) = validate_raw_skill_name(&name, raw) {
            return json_error(StatusCode::BAD_REQUEST, &e);
        }
        if let Err(e) = write_file_atomic(&file_path, raw).await {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("write error: {}", e),
            );
        }
    } else {
        let existing_content = tokio::fs::read_to_string(&file_path)
            .await
            .unwrap_or_default();
        let (existing_fm, existing_body) = parse_frontmatter_and_body(&existing_content);
        let body = req.body.as_deref().unwrap_or(&existing_body);
        let model_opt = req.model.as_ref().map(|v| {
            if v.is_null() {
                None
            } else {
                v.as_str().map(|s| s)
            }
        });
        let context_opt = req.context.as_ref().map(|v| {
            if v.is_null() {
                None
            } else {
                v.as_str().map(|s| s)
            }
        });
        let agent_opt = req.agent.as_ref().map(|v| {
            if v.is_null() {
                None
            } else {
                v.as_str().map(|s| s)
            }
        });
        let fm = build_skill_frontmatter(
            existing_fm,
            &name,
            req.description.as_deref(),
            req.user_invocable,
            req.disable_model_invocation,
            req.argument_hint.as_deref(),
            req.allowed_tools.as_deref(),
            model_opt,
            context_opt,
            agent_opt,
        );
        let content = match serialize_md_content(&fm, body) {
            Ok(c) => c,
            Err(e) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, &e),
        };
        if let Some(parent) = file_path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        if let Err(e) = write_file_atomic(&file_path, &content).await {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("write error: {}", e),
            );
        }
    }

    invalidate_slash_cache().await;
    let ext_dirs = make_scope_ext_dirs(base_dir.clone(), &scope_name);
    if load_skill_full(&ext_dirs, &name).await.is_none() {
        return json_error(StatusCode::UNPROCESSABLE_ENTITY, "skill_validation_failed");
    }
    let consumed = if let Some(ref draft_id) = req.draft_id {
        match consume_draft_for_target(
            gcx.clone(),
            draft_id,
            DraftKind::Skill,
            DraftTarget::Id(&name),
        )
        .await
        {
            Ok(consumed) => Some(consumed),
            Err(err) => return draft_error_response(err),
        }
    } else {
        None
    };
    let mut resp = serde_json::json!({"ok": true, "scope": scope_name, "file_path": file_path.display().to_string()});
    if let Some(c) = consumed {
        resp["consumed_draft"] = serde_json::to_value(c).unwrap_or_default();
    }
    json_response(StatusCode::OK, &resp)
}

#[derive(Deserialize)]
pub struct CreateSkillRequest {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub user_invocable: Option<bool>,
    #[serde(default)]
    pub disable_model_invocation: Option<bool>,
    #[serde(default)]
    pub allowed_tools: Option<Vec<String>>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub argument_hint: Option<String>,
}

pub async fn handle_v1_ext_skill_post(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let req: CreateSkillRequest = match serde_json::from_slice(&body_bytes) {
        Ok(r) => r,
        Err(e) => return json_error(StatusCode::BAD_REQUEST, &e.to_string()),
    };
    if let Err(e) = validate_name(&req.name) {
        return json_error(StatusCode::BAD_REQUEST, &e);
    }
    if req.description.is_empty() {
        return json_error(StatusCode::BAD_REQUEST, "description is required");
    }

    let scope_str_val = req.scope.as_deref().unwrap_or("local");
    let (base_dir, scope_name) =
        match resolve_scope_dir(gcx.clone(), Some(scope_str_val), true).await {
            Ok(d) => d,
            Err(e) => return json_error(StatusCode::BAD_REQUEST, &e),
        };

    let skill_dir = base_dir.join("skills").join(&req.name);
    let file_path = skill_dir.join("SKILL.md");

    if file_path.exists() {
        return json_error(StatusCode::CONFLICT, "skill already exists");
    }

    let fm = build_skill_frontmatter(
        serde_yaml::Value::Mapping(serde_yaml::Mapping::new()),
        &req.name,
        Some(&req.description),
        req.user_invocable,
        req.disable_model_invocation,
        req.argument_hint.as_deref(),
        req.allowed_tools.as_deref(),
        req.model.as_ref().map(|m| Some(m.as_str())),
        req.context.as_ref().map(|c| Some(c.as_str())),
        req.agent.as_ref().map(|a| Some(a.as_str())),
    );
    let body = req.body.as_deref().unwrap_or("");
    let content = match serialize_md_content(&fm, body) {
        Ok(c) => c,
        Err(e) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    };
    if let Err(e) = tokio::fs::create_dir_all(&skill_dir).await {
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("mkdir error: {}", e),
        );
    }
    if let Err(e) = write_file_atomic(&file_path, &content).await {
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("write error: {}", e),
        );
    }

    invalidate_slash_cache().await;
    json_response(
        StatusCode::CREATED,
        &serde_json::json!({"ok": true, "scope": scope_name, "file_path": file_path.display().to_string()}),
    )
}

pub async fn handle_v1_ext_skill_delete(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(name): Path<String>,
    Query(query): Query<ScopeQuery>,
) -> Result<Response<Body>, ScratchError> {
    if let Err(e) = validate_name(&name) {
        return json_error(StatusCode::BAD_REQUEST, &e);
    }
    let scope_str_val = match query.scope.as_deref() {
        Some(s) => s,
        None => {
            return json_error(
                StatusCode::BAD_REQUEST,
                "scope parameter required for delete",
            )
        }
    };
    let (base_dir, _scope_name) =
        match resolve_scope_dir(gcx.clone(), Some(scope_str_val), true).await {
            Ok(d) => d,
            Err(e) => return json_error(StatusCode::BAD_REQUEST, &e),
        };

    let skill_dir = base_dir.join("skills").join(&name);
    if !skill_dir.exists() {
        return json_error(StatusCode::NOT_FOUND, "skill not found");
    }
    if let Err(e) = tokio::fs::remove_dir_all(&skill_dir).await {
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("delete error: {}", e),
        );
    }

    invalidate_slash_cache().await;
    json_response(StatusCode::OK, &serde_json::json!({"ok": true}))
}

#[derive(Serialize)]
pub struct CommandDetailResponse {
    pub name: String,
    pub description: String,
    pub argument_hint: String,
    pub allowed_tools: Vec<String>,
    pub model: Option<String>,
    pub body: String,
    pub raw_content: String,
    pub source: String,
    pub file_path: String,
}

pub async fn handle_v1_ext_command_get(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(name): Path<String>,
    Query(query): Query<ScopeQuery>,
) -> Result<Response<Body>, ScratchError> {
    if let Err(e) = validate_name(&name) {
        return json_error(StatusCode::BAD_REQUEST, &e);
    }

    if let Some(ref draft_id) = query.draft_id {
        let draft_data = {
            let buddy_arc = gcx.read().await.buddy.clone();
            let lock = buddy_arc.lock().await;
            lock.as_ref().and_then(|svc| {
                let draft = svc.draft_store.get(draft_id)?;
                if draft.kind != DraftKind::Command {
                    return None;
                }
                Some((
                    draft.id.clone(),
                    draft.kind.clone(),
                    draft.title.clone(),
                    draft.explanation.clone(),
                    draft.expires_at,
                    draft.yaml_or_json.clone(),
                ))
            })
        };
        return match draft_data {
            None => json_error(StatusCode::NOT_FOUND, "draft_not_found_or_kind_mismatch"),
            Some((did, dkind, title, explanation, expires_at, yaml_or_json)) => {
                let data: serde_json::Value = match serde_yaml::from_str(&yaml_or_json) {
                    Ok(v) => v,
                    Err(e) => {
                        return json_error(
                            StatusCode::UNPROCESSABLE_ENTITY,
                            &format!("draft_parse_failed: {}", e),
                        )
                    }
                };
                json_response(
                    StatusCode::OK,
                    &DraftExtResponse {
                        data,
                        draft_metadata: DraftMetadata {
                            draft_id: did,
                            kind: draft_kind_str(&dkind).to_string(),
                            title,
                            explanation,
                            source_opportunity_id: None,
                            expires_at: expires_at.to_rfc3339(),
                        },
                    },
                )
            }
        };
    }

    let ext_dirs = match query.scope.as_deref() {
        Some(s @ "global") | Some(s @ "local") => {
            match resolve_scope_dir(gcx.clone(), Some(s), true).await {
                Ok((dir, scope)) => make_scope_ext_dirs(dir, &scope),
                Err(e) => return json_error(StatusCode::BAD_REQUEST, &e),
            }
        }
        _ => get_ext_dirs(gcx.clone()).await,
    };

    let commands = load_slash_commands(&ext_dirs).await;
    match commands.into_iter().find(|c| c.name == name) {
        None => json_error(StatusCode::NOT_FOUND, "command not found"),
        Some(cmd) => {
            let fp = cmd.file_path.clone();
            let raw_content = tokio::fs::read_to_string(&fp).await.unwrap_or_default();
            json_response(
                StatusCode::OK,
                &CommandDetailResponse {
                    name: cmd.name.clone(),
                    description: cmd.description.clone(),
                    argument_hint: cmd.argument_hint.clone(),
                    allowed_tools: cmd.allowed_tools.clone(),
                    model: cmd.model.clone(),
                    body: cmd.body.clone(),
                    raw_content,
                    source: source_str(&cmd.source),
                    file_path: fp.display().to_string(),
                },
            )
        }
    }
}

#[derive(Deserialize)]
pub struct SaveCommandRequest {
    #[serde(default)]
    pub raw_content: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub argument_hint: Option<String>,
    #[serde(default)]
    pub allowed_tools: Option<Vec<String>>,
    #[serde(default)]
    pub model: Option<serde_json::Value>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub draft_id: Option<String>,
}

pub async fn handle_v1_ext_command_put(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(name): Path<String>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    if let Err(e) = validate_name(&name) {
        return json_error(StatusCode::BAD_REQUEST, &e);
    }
    let req: SaveCommandRequest = match serde_json::from_slice(&body_bytes) {
        Ok(r) => r,
        Err(e) => return json_error(StatusCode::BAD_REQUEST, &e.to_string()),
    };

    let scope_str_val = req.scope.as_deref().unwrap_or("local");
    let (base_dir, scope_name) =
        match resolve_scope_dir(gcx.clone(), Some(scope_str_val), true).await {
            Ok(d) => d,
            Err(e) => return json_error(StatusCode::BAD_REQUEST, &e),
        };

    let file_path = base_dir.join("commands").join(format!("{}.md", name));

    if let Some(ref draft_id) = req.draft_id {
        if let Err(err) = validate_draft_for_target(
            gcx.clone(),
            draft_id,
            DraftKind::Command,
            DraftTarget::Id(&name),
        )
        .await
        {
            return draft_error_response(err);
        }
    }

    if let Some(raw) = &req.raw_content {
        if let Some(parent) = file_path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        if let Err(e) = write_file_atomic(&file_path, raw).await {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("write error: {}", e),
            );
        }
    } else {
        let existing_content = tokio::fs::read_to_string(&file_path)
            .await
            .unwrap_or_default();
        let (existing_fm, existing_body) = parse_frontmatter_and_body(&existing_content);
        let body = req.body.as_deref().unwrap_or(&existing_body);
        let model_opt = req.model.as_ref().map(|v| {
            if v.is_null() {
                None
            } else {
                v.as_str().map(|s| s)
            }
        });
        let fm = build_command_frontmatter(
            existing_fm,
            req.description.as_deref(),
            req.argument_hint.as_deref(),
            req.allowed_tools.as_deref(),
            model_opt,
        );
        let content = match serialize_md_content(&fm, body) {
            Ok(c) => c,
            Err(e) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, &e),
        };
        if let Some(parent) = file_path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        if let Err(e) = write_file_atomic(&file_path, &content).await {
            return json_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("write error: {}", e),
            );
        }
    }

    invalidate_slash_cache().await;
    let ext_dirs = make_scope_ext_dirs(base_dir.clone(), &scope_name);
    let commands = load_slash_commands(&ext_dirs).await;
    if !commands.iter().any(|cmd| cmd.name == name) {
        return json_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "command_validation_failed",
        );
    }
    let consumed = if let Some(ref draft_id) = req.draft_id {
        match consume_draft_for_target(
            gcx.clone(),
            draft_id,
            DraftKind::Command,
            DraftTarget::Id(&name),
        )
        .await
        {
            Ok(consumed) => Some(consumed),
            Err(err) => return draft_error_response(err),
        }
    } else {
        None
    };
    let mut resp = serde_json::json!({"ok": true, "scope": scope_name, "file_path": file_path.display().to_string()});
    if let Some(c) = consumed {
        resp["consumed_draft"] = serde_json::to_value(c).unwrap_or_default();
    }
    json_response(StatusCode::OK, &resp)
}

#[derive(Deserialize)]
pub struct CreateCommandRequest {
    pub name: String,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub argument_hint: Option<String>,
    #[serde(default)]
    pub allowed_tools: Option<Vec<String>>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
}

pub async fn handle_v1_ext_command_post(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let req: CreateCommandRequest = match serde_json::from_slice(&body_bytes) {
        Ok(r) => r,
        Err(e) => return json_error(StatusCode::BAD_REQUEST, &e.to_string()),
    };
    if let Err(e) = validate_name(&req.name) {
        return json_error(StatusCode::BAD_REQUEST, &e);
    }

    let scope_str_val = req.scope.as_deref().unwrap_or("local");
    let (base_dir, scope_name) =
        match resolve_scope_dir(gcx.clone(), Some(scope_str_val), true).await {
            Ok(d) => d,
            Err(e) => return json_error(StatusCode::BAD_REQUEST, &e),
        };

    let file_path = base_dir.join("commands").join(format!("{}.md", req.name));
    if file_path.exists() {
        return json_error(StatusCode::CONFLICT, "command already exists");
    }

    let fm = build_command_frontmatter(
        serde_yaml::Value::Mapping(serde_yaml::Mapping::new()),
        req.description.as_deref(),
        req.argument_hint.as_deref(),
        req.allowed_tools.as_deref(),
        req.model.as_ref().map(|m| Some(m.as_str())),
    );
    let body = req.body.as_deref().unwrap_or("");
    let content = match serialize_md_content(&fm, body) {
        Ok(c) => c,
        Err(e) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    };
    if let Some(parent) = file_path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    if let Err(e) = write_file_atomic(&file_path, &content).await {
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("write error: {}", e),
        );
    }

    invalidate_slash_cache().await;
    json_response(
        StatusCode::CREATED,
        &serde_json::json!({"ok": true, "scope": scope_name, "file_path": file_path.display().to_string()}),
    )
}

pub async fn handle_v1_ext_command_delete(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(name): Path<String>,
    Query(query): Query<ScopeQuery>,
) -> Result<Response<Body>, ScratchError> {
    if let Err(e) = validate_name(&name) {
        return json_error(StatusCode::BAD_REQUEST, &e);
    }
    let scope_str_val = match query.scope.as_deref() {
        Some(s) => s,
        None => {
            return json_error(
                StatusCode::BAD_REQUEST,
                "scope parameter required for delete",
            )
        }
    };
    let (base_dir, _scope_name) =
        match resolve_scope_dir(gcx.clone(), Some(scope_str_val), true).await {
            Ok(d) => d,
            Err(e) => return json_error(StatusCode::BAD_REQUEST, &e),
        };

    let file_path = base_dir.join("commands").join(format!("{}.md", name));
    if !file_path.exists() {
        return json_error(StatusCode::NOT_FOUND, "command not found");
    }
    if let Err(e) = tokio::fs::remove_file(&file_path).await {
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("delete error: {}", e),
        );
    }

    invalidate_slash_cache().await;
    json_response(StatusCode::OK, &serde_json::json!({"ok": true}))
}

#[derive(Serialize)]
pub struct HooksResponse {
    pub hooks: Vec<HookConfigFlat>,
    pub raw_content: String,
    pub file_path: String,
}

pub async fn handle_v1_ext_hooks_get(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Query(query): Query<ScopeQuery>,
) -> Result<Response<Body>, ScratchError> {
    if let Some(ref draft_id) = query.draft_id {
        let draft_data = {
            let buddy_arc = gcx.read().await.buddy.clone();
            let lock = buddy_arc.lock().await;
            lock.as_ref().and_then(|svc| {
                let draft = svc.draft_store.get(draft_id)?;
                if draft.kind != DraftKind::Hook {
                    return None;
                }
                Some((
                    draft.id.clone(),
                    draft.kind.clone(),
                    draft.title.clone(),
                    draft.explanation.clone(),
                    draft.expires_at,
                    draft.yaml_or_json.clone(),
                ))
            })
        };
        return match draft_data {
            None => json_error(StatusCode::NOT_FOUND, "draft_not_found_or_kind_mismatch"),
            Some((did, dkind, title, explanation, expires_at, yaml_or_json)) => {
                let data: serde_json::Value = match serde_yaml::from_str(&yaml_or_json) {
                    Ok(v) => v,
                    Err(e) => {
                        return json_error(
                            StatusCode::UNPROCESSABLE_ENTITY,
                            &format!("draft_parse_failed: {}", e),
                        )
                    }
                };
                json_response(
                    StatusCode::OK,
                    &DraftExtResponse {
                        data,
                        draft_metadata: DraftMetadata {
                            draft_id: did,
                            kind: draft_kind_str(&dkind).to_string(),
                            title,
                            explanation,
                            source_opportunity_id: None,
                            expires_at: expires_at.to_rfc3339(),
                        },
                    },
                )
            }
        };
    }

    let scope_str_val = query.scope.as_deref().unwrap_or("local");
    let (base_dir, scope_name) =
        match resolve_scope_dir(gcx.clone(), Some(scope_str_val), true).await {
            Ok(d) => d,
            Err(e) => return json_error(StatusCode::BAD_REQUEST, &e),
        };

    let file_path = base_dir.join("hooks.yaml");
    let raw_content = tokio::fs::read_to_string(&file_path)
        .await
        .unwrap_or_default();

    let ext_dirs = make_scope_ext_dirs(base_dir, &scope_name);
    let hooks = load_hooks(&ext_dirs).await;
    let flat_hooks: Vec<HookConfigFlat> = hooks.iter().map(HookConfigFlat::from).collect();

    json_response(
        StatusCode::OK,
        &HooksResponse {
            hooks: flat_hooks,
            raw_content,
            file_path: file_path.display().to_string(),
        },
    )
}

#[derive(Deserialize)]
pub struct SaveHooksRequest {
    #[serde(default)]
    pub raw_content: Option<String>,
    #[serde(default)]
    pub hooks: Option<Vec<HookConfigFlat>>,
    #[serde(default)]
    pub draft_id: Option<String>,
}

pub async fn handle_v1_ext_hooks_put(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Query(query): Query<ScopeQuery>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let req: SaveHooksRequest = match serde_json::from_slice(&body_bytes) {
        Ok(r) => r,
        Err(e) => return json_error(StatusCode::BAD_REQUEST, &e.to_string()),
    };

    let scope_str_val = query.scope.as_deref().unwrap_or("local");
    let (base_dir, scope_name) =
        match resolve_scope_dir(gcx.clone(), Some(scope_str_val), true).await {
            Ok(d) => d,
            Err(e) => return json_error(StatusCode::BAD_REQUEST, &e),
        };

    let file_path = base_dir.join("hooks.yaml");

    if let Some(ref draft_id) = req.draft_id {
        if let Err(err) =
            validate_draft_for_target(gcx.clone(), draft_id, DraftKind::Hook, DraftTarget::Any)
                .await
        {
            return draft_error_response(err);
        }
    }

    let content = if let Some(raw) = &req.raw_content {
        if let Err(e) = validate_raw_hooks(raw) {
            return json_error(StatusCode::UNPROCESSABLE_ENTITY, &e);
        }
        raw.clone()
    } else if let Some(hooks) = &req.hooks {
        for h in hooks {
            if h.command.is_empty() {
                return json_error(StatusCode::BAD_REQUEST, "hook command cannot be empty");
            }
        }
        match hooks_to_yaml_string(hooks) {
            Ok(s) => s,
            Err(e) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, &e),
        }
    } else {
        return json_error(
            StatusCode::BAD_REQUEST,
            "either raw_content or hooks must be provided",
        );
    };

    if let Some(parent) = file_path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    if let Err(e) = write_file_atomic(&file_path, &content).await {
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("write error: {}", e),
        );
    }

    let consumed = if let Some(ref draft_id) = req.draft_id {
        match consume_draft_for_target(gcx.clone(), draft_id, DraftKind::Hook, DraftTarget::Any)
            .await
        {
            Ok(consumed) => Some(consumed),
            Err(err) => return draft_error_response(err),
        }
    } else {
        None
    };
    let mut resp = serde_json::json!({"ok": true, "scope": scope_name, "file_path": file_path.display().to_string()});
    if let Some(c) = consumed {
        resp["consumed_draft"] = serde_json::to_value(c).unwrap_or_default();
    }
    json_response(StatusCode::OK, &resp)
}

pub async fn handle_v1_ext_hooks_delete_by_index(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    Path(index): Path<usize>,
    Query(query): Query<ScopeQuery>,
) -> Result<Response<Body>, ScratchError> {
    let scope_str_val = match query.scope.as_deref() {
        Some(s) => s,
        None => {
            return json_error(
                StatusCode::BAD_REQUEST,
                "scope parameter required for delete",
            )
        }
    };
    let (base_dir, scope_name) =
        match resolve_scope_dir(gcx.clone(), Some(scope_str_val), true).await {
            Ok(d) => d,
            Err(e) => return json_error(StatusCode::BAD_REQUEST, &e),
        };

    let file_path = base_dir.join("hooks.yaml");
    let ext_dirs = make_scope_ext_dirs(base_dir, &scope_name);
    let mut hooks: Vec<HookConfigFlat> = load_hooks(&ext_dirs)
        .await
        .iter()
        .map(HookConfigFlat::from)
        .collect();

    if index >= hooks.len() {
        return json_error(StatusCode::NOT_FOUND, "hook index out of bounds");
    }
    hooks.remove(index);

    let content = match hooks_to_yaml_string(&hooks) {
        Ok(s) => s,
        Err(e) => return json_error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    };
    if let Some(parent) = file_path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    if let Err(e) = write_file_atomic(&file_path, &content).await {
        return json_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("write error: {}", e),
        );
    }

    json_response(StatusCode::OK, &serde_json::json!({"ok": true}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ext::config_dirs::ExtDirs;

    fn validate_name_clean(name: &str) -> bool {
        !name.is_empty()
            && !name.starts_with('.')
            && !name.contains('/')
            && !name.contains('\\')
            && !name.contains("..")
    }

    fn build_skill_roundtrip(name: &str, description: &str, body: &str) -> String {
        let mut map = serde_yaml::Mapping::new();
        map.insert(
            serde_yaml::Value::String("name".to_string()),
            serde_yaml::Value::String(name.to_string()),
        );
        map.insert(
            serde_yaml::Value::String("description".to_string()),
            serde_yaml::Value::String(description.to_string()),
        );
        let fm = serde_yaml::Value::Mapping(map);
        serialize_md_content(&fm, body).unwrap_or_default()
    }

    #[test]
    fn test_validate_name_valid() {
        assert!(validate_name("my-skill").is_ok());
        assert!(validate_name("my_skill").is_ok());
        assert!(validate_name("skill123").is_ok());
        assert!(validate_name("a").is_ok());
    }

    #[test]
    fn test_validate_name_rejects_empty() {
        assert!(validate_name("").is_err());
    }

    #[test]
    fn test_validate_name_rejects_dot_prefix() {
        assert!(validate_name(".hidden").is_err());
    }

    #[test]
    fn test_validate_name_rejects_slash() {
        assert!(validate_name("path/traversal").is_err());
    }

    #[test]
    fn test_validate_name_rejects_backslash() {
        assert!(validate_name("path\\traversal").is_err());
    }

    #[test]
    fn test_validate_name_rejects_dotdot() {
        assert!(validate_name("../../etc/passwd").is_err());
        assert!(validate_name("..").is_err());
    }

    #[test]
    fn test_skill_content_roundtrip() {
        let content = build_skill_roundtrip("my-skill", "A useful skill", "Do something");
        assert!(content.contains("---"));
        assert!(content.contains("name: my-skill"));
        assert!(content.contains("description: A useful skill"));
        assert!(content.contains("Do something"));

        let (fm, body) = parse_frontmatter_and_body(&content);
        assert_eq!(fm.get("name").and_then(|v| v.as_str()), Some("my-skill"));
        assert_eq!(
            fm.get("description").and_then(|v| v.as_str()),
            Some("A useful skill")
        );
        assert_eq!(body.trim(), "Do something");
    }

    #[test]
    fn test_hooks_to_yaml_roundtrip() {
        let hooks = vec![
            HookConfigFlat {
                event: HookEvent::PreToolUse,
                matcher: Some("Bash".to_string()),
                command: "./check.sh".to_string(),
                timeout: Some(30),
            },
            HookConfigFlat {
                event: HookEvent::SessionStart,
                matcher: None,
                command: "echo start".to_string(),
                timeout: None,
            },
        ];
        let yaml = hooks_to_yaml_string(&hooks).expect("should serialize");
        assert!(yaml.contains("PreToolUse"));
        assert!(yaml.contains("SessionStart"));
        assert!(yaml.contains("./check.sh"));
        assert!(yaml.contains("echo start"));
        assert!(yaml.contains("Bash"));
        assert!(yaml.contains("30"));
    }

    #[test]
    fn test_hooks_to_yaml_empty() {
        let hooks: Vec<HookConfigFlat> = vec![];
        let yaml = hooks_to_yaml_string(&hooks).expect("should serialize");
        assert!(yaml.contains("hooks"));
    }

    #[test]
    fn test_validate_name_clean_helper() {
        assert!(validate_name_clean("skill-name"));
        assert!(!validate_name_clean(""));
        assert!(!validate_name_clean(".hidden"));
        assert!(!validate_name_clean("a/b"));
        assert!(!validate_name_clean(".."));
    }

    #[test]
    fn test_build_skill_frontmatter_preserves_unknown() {
        let mut existing = serde_yaml::Mapping::new();
        existing.insert(
            serde_yaml::Value::String("name".to_string()),
            serde_yaml::Value::String("old".to_string()),
        );
        existing.insert(
            serde_yaml::Value::String("custom-field".to_string()),
            serde_yaml::Value::String("preserved".to_string()),
        );
        let fm = build_skill_frontmatter(
            serde_yaml::Value::Mapping(existing),
            "new-name",
            Some("new desc"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        );
        let map = fm.as_mapping().unwrap();
        assert_eq!(map.get("name").and_then(|v| v.as_str()), Some("new-name"));
        assert_eq!(
            map.get("description").and_then(|v| v.as_str()),
            Some("new desc")
        );
        assert_eq!(
            map.get("custom-field").and_then(|v| v.as_str()),
            Some("preserved")
        );
    }

    #[test]
    fn test_source_str_values() {
        assert_eq!(source_str(&CommandSource::GlobalRefact), "global_refact");
        assert_eq!(source_str(&CommandSource::GlobalClaude), "global_claude");
        assert_eq!(
            source_str(&CommandSource::ProjectRefact(PathBuf::new())),
            "project_refact"
        );
        assert_eq!(
            source_str(&CommandSource::ProjectClaude(PathBuf::new())),
            "project_claude"
        );
    }

    #[test]
    fn test_scope_str_values() {
        assert_eq!(scope_str(&CommandSource::GlobalRefact), "global");
        assert_eq!(scope_str(&CommandSource::GlobalClaude), "global");
        assert_eq!(
            scope_str(&CommandSource::ProjectRefact(PathBuf::new())),
            "local"
        );
        assert_eq!(
            scope_str(&CommandSource::ProjectClaude(PathBuf::new())),
            "local"
        );
    }

    #[test]
    fn test_read_only_values() {
        assert!(!is_read_only(&CommandSource::GlobalRefact));
        assert!(is_read_only(&CommandSource::GlobalClaude));
        assert!(!is_read_only(&CommandSource::ProjectRefact(PathBuf::new())));
        assert!(is_read_only(&CommandSource::ProjectClaude(PathBuf::new())));
    }

    #[tokio::test]
    async fn test_hooks_get_from_file() {
        let tmp = tempfile::tempdir().unwrap();
        let hooks_yaml = r#"hooks:
  PreToolUse:
    - matcher: "Bash"
      hooks:
        - type: command
          command: "./check.sh"
          timeout: 30
"#;
        tokio::fs::write(tmp.path().join("hooks.yaml"), hooks_yaml)
            .await
            .unwrap();
        let ext_dirs = ExtDirs {
            global_dirs: vec![tmp.path().to_path_buf()],
            project_dirs: vec![],
            installed_dirs: vec![],
        };
        let hooks = load_hooks(&ext_dirs).await;
        let flat: Vec<HookConfigFlat> = hooks.iter().map(HookConfigFlat::from).collect();
        assert_eq!(flat.len(), 1);
        assert_eq!(flat[0].command, "./check.sh");
        assert_eq!(flat[0].matcher, Some("Bash".to_string()));
        assert_eq!(flat[0].timeout, Some(30));
    }
}
