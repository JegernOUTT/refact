use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::OnceLock;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex as AMutex;

use crate::app_state::AppState;
use crate::call_validation;
use crate::files_correction::get_project_dirs;
use crate::scratchpads::scratchpad_utils::HasRagResults;
use super::system_context::{
    self, create_instruction_files_message, create_memories_message, gather_system_context,
    generate_git_info_prompt, gather_git_info, PROJECT_CONTEXT_MARKER,
};
use crate::ext::skills_context::{
    build_skills_context_messages_tracked, build_skills_prompt_text, SkillsTrackingInfo,
    SKILLS_CONTEXT_MARKER,
};
use crate::tools::tool_task_memory::{MemoryKind, MemoryNamespace, MemoryStatus, TaskMemoryFrontmatter};
use crate::yaml_configs::project_information::load_project_information_config;
use crate::call_validation::{ChatMessage, ChatContent, ContextFile, canonical_mode_id};
use crate::tasks::storage::infer_task_id_from_chat_id;
use crate::yaml_configs::customization_registry::{get_mode_config, map_legacy_mode_to_id};

const BUDDY_PERSONALITY_MARKER: &str = "%BUDDY_PERSONALITY%";
const BUDDY_PULSE_MARKER: &str = "buddy_project_memory_pulse";

#[derive(Clone)]
struct BuddyPersonaCacheEntry {
    version: u64,
    identity_name: String,
    rendered: String,
}

static BUDDY_PERSONA_CACHE: OnceLock<AMutex<HashMap<(String, String), BuddyPersonaCacheEntry>>> =
    OnceLock::new();

fn buddy_persona_cache_mode_id(mode_id: &str) -> String {
    match mode_id {
        "openai_agent" => "openai_agent".to_string(),
        _ => map_legacy_mode_to_id(mode_id).to_string(),
    }
}

fn buddy_persona_cache() -> &'static AMutex<HashMap<(String, String), BuddyPersonaCacheEntry>> {
    BUDDY_PERSONA_CACHE.get_or_init(|| AMutex::new(HashMap::new()))
}

pub async fn get_mode_system_prompt(
    app: AppState,
    mode_id: &str,
    model_id: Option<&str>,
) -> String {
    let mode_id = map_legacy_mode_to_id(mode_id);

    match get_mode_config(app.gcx.clone(), mode_id, model_id).await {
        Some(mode_config) => mode_config.prompt,
        None => {
            tracing::warn!("Mode '{}' not found, using empty prompt", mode_id);
            String::new()
        }
    }
}

pub async fn render_mode_plan_template(
    app: AppState,
    template: &str,
    mode_name: &str,
    include_project_info: bool,
    task_meta: &Option<crate::chat::types::TaskMeta>,
) -> String {
    let mut rendered = template.replace("%MODE_NAME%", mode_name);

    if rendered.contains("%PROJECT_TREE%") {
        if include_project_info {
            match system_context::generate_compact_project_tree(app.gcx.clone(), 1).await {
                Ok(tree) if !tree.is_empty() => {
                    let first_level = tree
                        .lines()
                        .filter(|line| !line.starts_with("    "))
                        .collect::<Vec<_>>()
                        .join("\n");
                    let tree_content = truncate_to_chars(&first_level, 600);
                    rendered = rendered.replace("%PROJECT_TREE%", &tree_content);
                }
                _ => {
                    rendered = rendered.replace("%PROJECT_TREE%", "");
                }
            }
        } else {
            rendered = rendered.replace("%PROJECT_TREE%", "");
        }
    }

    if rendered.contains("%TASK_BRIEF%") {
        let task_brief = task_brief_for_plan_template(&app, task_meta).await;
        rendered = rendered.replace("%TASK_BRIEF%", &task_brief);
    }

    rendered
}

async fn task_brief_for_plan_template(
    app: &AppState,
    task_meta: &Option<crate::chat::types::TaskMeta>,
) -> String {
    let Some(meta) = task_meta else {
        return String::new();
    };

    match crate::tasks::storage::load_task_meta(app.gcx.clone(), &meta.task_id).await {
        Ok(task) => {
            let mut lines = vec![format!("Task: {} ({:?})", task.name, task.status)];
            if let Some(card_id) = meta.card_id.as_deref() {
                if let Ok(board) =
                    crate::tasks::storage::load_board(app.gcx.clone(), &meta.task_id).await
                {
                    if let Some(card) = board.get_card(card_id) {
                        lines.push(format!(
                            "Card: {} — {} ({}, {})",
                            card.id, card.title, card.column, card.priority
                        ));
                        if !card.target_files.is_empty() {
                            lines.push(format!("Target files: {}", card.target_files.join(", ")));
                        }
                        if !card.instructions.trim().is_empty() {
                            lines.push(
                                crate::llm::safe_truncate(card.instructions.trim(), 500)
                                    .to_string(),
                            );
                        }
                    }
                }
            }
            crate::llm::safe_truncate(&lines.join("\n"), 700).to_string()
        }
        Err(_) => String::new(),
    }
}

async fn _workspace_info(workspace_dirs: &[String], active_file_path: &Option<PathBuf>) -> String {
    async fn get_vcs_info(detect_vcs_at: &PathBuf) -> String {
        let mut info = String::new();
        if let Some((vcs_path, vcs_type)) =
            crate::files_in_workspace::detect_vcs_for_a_file_path(detect_vcs_at).await
        {
            info.push_str(&format!(
                "\nThe project is under {} version control, located at:\n{}",
                vcs_type,
                vcs_path.display()
            ));
        } else {
            info.push_str("\nThere's no version control detected, complain to user if they want to use anything git/hg/svn/etc.");
        }
        info
    }
    let mut info = String::new();
    if !workspace_dirs.is_empty() {
        info.push_str(&format!(
            "The current IDE workspace has these project directories:\n{}",
            workspace_dirs.join("\n")
        ));
    }
    let detect_vcs_at_option = active_file_path
        .clone()
        .or_else(|| workspace_dirs.get(0).map(PathBuf::from));
    if let Some(detect_vcs_at) = detect_vcs_at_option {
        let vcs_info = get_vcs_info(&detect_vcs_at).await;
        if let Some(active_file) = active_file_path {
            info.push_str(&format!(
                "\n\nThe active IDE file is:\n{}",
                active_file.display()
            ));
        } else {
            info.push_str("\n\nThere is no active file currently open in the IDE.");
        }
        info.push_str(&vcs_info);
    } else {
        info.push_str("\n\nThere is no active file with version control, complain to user if they want to use anything git/hg/svn/etc and ask to open a file in IDE for you to know which project is active.");
    }
    info
}

async fn workspace_files_info(app: &AppState) -> (Vec<String>, Option<PathBuf>) {
    let documents_state = &app.workspace.documents_state;
    let workspace_dirs: Vec<String> = {
        let dirs_locked = documents_state.workspace_folders.lock().unwrap();
        dirs_locked
            .iter()
            .map(|x| x.to_string_lossy().to_string())
            .collect()
    };
    let active_file_path = documents_state.active_file_path.lock().await.clone();
    (workspace_dirs, active_file_path)
}

fn truncate_to_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars).collect();
        format!("{}\n[TRUNCATED]", truncated)
    }
}

pub async fn system_prompt_add_extra_instructions(
    app: AppState,
    system_prompt: String,
    tool_names: HashSet<String>,
    chat_meta: &call_validation::ChatMeta,
    task_meta: &Option<crate::chat::types::TaskMeta>,
    mode_id: &str,
) -> String {
    let include_project_info = chat_meta.include_project_info;

    // Load project information config to respect user settings
    let config = load_project_information_config(app.gcx.clone()).await;
    // If config is globally disabled, treat as if include_project_info is false
    let include_project_info = include_project_info && config.enabled;

    let mut system_prompt = system_prompt.clone();

    if system_prompt.contains(BUDDY_PERSONALITY_MARKER) {
        let buddy_block = buddy_persona_block(app.clone(), mode_id).await;
        system_prompt = system_prompt.replace(BUDDY_PERSONALITY_MARKER, &buddy_block);
    }

    // %SYSTEM_INFO% - OS, datetime, username, architecture
    // Respects config.sections.system_info.enabled and max_chars
    if system_prompt.contains("%SYSTEM_INFO%") {
        if include_project_info && config.sections.system_info.enabled {
            let system_info = system_context::SystemInfo::gather();
            let mut content = system_info.to_prompt_string();
            if let Some(max_chars) = config.sections.system_info.max_chars {
                content = truncate_to_chars(&content, max_chars);
            }
            system_prompt = system_prompt.replace("%SYSTEM_INFO%", &content);
        } else {
            system_prompt = system_prompt.replace("%SYSTEM_INFO%", "");
        }
    }

    // %ENVIRONMENT_INFO% - Detected environments and usage instructions
    // Respects config.sections.environment_instructions.enabled and max_chars
    if system_prompt.contains("%ENVIRONMENT_INFO%") {
        if include_project_info && config.sections.environment_instructions.enabled {
            let project_dirs = get_project_dirs(app.gcx.clone()).await;
            let environments = system_context::detect_environments(&project_dirs).await;
            let mut env_instructions =
                system_context::generate_environment_instructions(&environments);
            if let Some(max_chars) = config.sections.environment_instructions.max_chars {
                env_instructions = truncate_to_chars(&env_instructions, max_chars);
            }
            system_prompt = system_prompt.replace("%ENVIRONMENT_INFO%", &env_instructions);
        } else {
            system_prompt = system_prompt.replace("%ENVIRONMENT_INFO%", "");
        }
    }

    // %PROJECT_CONFIGS% - Detected project configuration files
    // Respects config.sections.project_configs.enabled and max_items
    if system_prompt.contains("%PROJECT_CONFIGS%") {
        if include_project_info && config.sections.project_configs.enabled {
            let project_dirs = get_project_dirs(app.gcx.clone()).await;
            let configs = system_context::find_project_configs(&project_dirs).await;
            let max_items = config.sections.project_configs.max_items.unwrap_or(30);
            let configs_to_show: Vec<_> = configs.into_iter().take(max_items).collect();
            if !configs_to_show.is_empty() {
                let config_list = configs_to_show
                    .iter()
                    .map(|c| format!("- {} ({})", c.file_name, c.category))
                    .collect::<Vec<_>>()
                    .join("\n");
                let config_section = format!("## Project Configuration Files\n{}", config_list);
                system_prompt = system_prompt.replace("%PROJECT_CONFIGS%", &config_section);
            } else {
                system_prompt = system_prompt.replace("%PROJECT_CONFIGS%", "");
            }
        } else {
            system_prompt = system_prompt.replace("%PROJECT_CONFIGS%", "");
        }
    }

    // %PROJECT_TREE% - Project file tree
    // Respects config.sections.project_tree.enabled, max_depth, and max_chars
    if system_prompt.contains("%PROJECT_TREE%") {
        if include_project_info && config.sections.project_tree.enabled {
            let max_depth = config.sections.project_tree.max_depth.unwrap_or(4);
            let max_chars = config.sections.project_tree.max_chars.unwrap_or(16000);
            match system_context::generate_compact_project_tree(app.gcx.clone(), max_depth).await {
                Ok(tree) if !tree.is_empty() => {
                    let tree_content = truncate_to_chars(&tree, max_chars);
                    let tree_section = format!("## Project Structure\n```\n{}```", tree_content);
                    system_prompt = system_prompt.replace("%PROJECT_TREE%", &tree_section);
                }
                _ => {
                    system_prompt = system_prompt.replace("%PROJECT_TREE%", "");
                }
            }
        } else {
            system_prompt = system_prompt.replace("%PROJECT_TREE%", "");
        }
    }

    // %GIT_INFO% - Git repository information
    // Respects config.sections.git_info.enabled and max_chars
    if system_prompt.contains("%GIT_INFO%") {
        if include_project_info && config.sections.git_info.enabled {
            let project_dirs = get_project_dirs(app.gcx.clone()).await;
            let git_infos = gather_git_info(&project_dirs).await;
            let mut git_section = generate_git_info_prompt(&git_infos);
            if let Some(max_chars) = config.sections.git_info.max_chars {
                git_section = truncate_to_chars(&git_section, max_chars);
            }
            system_prompt = system_prompt.replace("%GIT_INFO%", &git_section);
        } else {
            system_prompt = system_prompt.replace("%GIT_INFO%", "");
        }
    }

    if system_prompt.contains("%WORKSPACE_INFO%") {
        if include_project_info {
            let (workspace_dirs, active_file_path) = workspace_files_info(&app).await;
            let info = _workspace_info(&workspace_dirs, &active_file_path).await;
            system_prompt = system_prompt.replace("%WORKSPACE_INFO%", &info);
        } else {
            system_prompt = system_prompt.replace("%WORKSPACE_INFO%", "");
        }
    }

    if system_prompt.contains("%AGENT_WORKTREE%") {
        let worktree_info = if let Some(tm) = task_meta {
            if let Some(ref card_id) = tm.card_id {
                match crate::tasks::storage::load_board(app.gcx.clone(), &tm.task_id).await {
                    Ok(board) => {
                        if let Some(card) = board.get_card(card_id) {
                            if let Some(ref worktree) = card.agent_worktree {
                                format!("## Your Working Directory\nYou are working in an isolated git worktree at:\n`{}`\n\nAll your file operations should be within this directory. Changes here don't affect the main repository until merged.", worktree)
                            } else {
                                String::new()
                            }
                        } else {
                            String::new()
                        }
                    }
                    Err(_) => String::new(),
                }
            } else {
                String::new()
            }
        } else {
            String::new()
        };
        system_prompt = system_prompt.replace("%AGENT_WORKTREE%", &worktree_info);
    }

    if system_prompt.contains("%KNOWLEDGE_INSTRUCTIONS%") {
        system_prompt = system_prompt.replace("%KNOWLEDGE_INSTRUCTIONS%", "");
    }

    if system_prompt.contains("%PROJECT_SUMMARY%") {
        system_prompt = system_prompt.replace("%PROJECT_SUMMARY%", "");
    }

    if system_prompt.contains("%SKILLS_INSTRUCTIONS%") {
        let has_activate = tool_names.contains("activate_skill");
        let has_deactivate = tool_names.contains("deactivate_skill");
        if has_activate || has_deactivate {
            let skills_text =
                build_skills_prompt_text(app.clone(), has_activate, has_deactivate).await;
            system_prompt = system_prompt.replace("%SKILLS_INSTRUCTIONS%", &skills_text);
        } else {
            system_prompt = system_prompt.replace("%SKILLS_INSTRUCTIONS%", "");
        }
    }

    if system_prompt.contains("%EXPLORE_FILE_EDIT_INSTRUCTIONS%") {
        let replacement =
            if tool_names.contains("create_textdoc") || tool_names.contains("update_textdoc") {
                "- Then use `*_textdoc()` tools to make changes.\n"
            } else {
                ""
            };

        system_prompt = system_prompt.replace("%EXPLORE_FILE_EDIT_INSTRUCTIONS%", replacement);
    }

    if system_prompt.contains("%AGENT_EXPLORATION_INSTRUCTIONS%") {
        system_prompt = system_prompt.replace(
            "%AGENT_EXPLORATION_INSTRUCTIONS%",
            super::prompt_snippets::AGENT_EXPLORATION_INSTRUCTIONS,
        );
    }

    if system_prompt.contains("%AGENT_EXECUTION_INSTRUCTIONS%") {
        let has_edit_tools =
            tool_names.contains("create_textdoc") || tool_names.contains("update_textdoc");
        let replacement = if has_edit_tools {
            super::prompt_snippets::AGENT_EXECUTION_INSTRUCTIONS
        } else {
            super::prompt_snippets::AGENT_EXECUTION_INSTRUCTIONS_NO_TOOLS
        };
        system_prompt = system_prompt.replace("%AGENT_EXECUTION_INSTRUCTIONS%", replacement);
    }

    if system_prompt.contains("%CD_INSTRUCTIONS%") {
        system_prompt =
            system_prompt.replace("%CD_INSTRUCTIONS%", super::prompt_snippets::CD_INSTRUCTIONS);
    }

    if system_prompt.contains("%SHELL_INSTRUCTIONS%") {
        system_prompt = system_prompt.replace(
            "%SHELL_INSTRUCTIONS%",
            super::prompt_snippets::SHELL_INSTRUCTIONS,
        );
    }

    if system_prompt.contains("%COMPRESS_HANDOFF_INSTRUCTIONS%") {
        let has_compress = tool_names.contains("compress_chat_probe")
            || tool_names.contains("compress_chat_apply");
        let has_handoff = tool_names.contains("handoff_to_mode");
        let replacement = if has_compress {
            super::prompt_snippets::COMPRESS_HANDOFF_INSTRUCTIONS
        } else if has_handoff {
            super::prompt_snippets::HANDOFF_ONLY_INSTRUCTIONS
        } else {
            ""
        };
        system_prompt = system_prompt.replace("%COMPRESS_HANDOFF_INSTRUCTIONS%", replacement);
    }

    if system_prompt.contains("%RICH_CONTENT_INSTRUCTIONS%") {
        system_prompt = system_prompt.replace(
            "%RICH_CONTENT_INSTRUCTIONS%",
            super::prompt_snippets::RICH_CONTENT_INSTRUCTIONS,
        );
    }

    system_prompt
}

async fn buddy_persona_block(app: AppState, mode_id: &str) -> String {
    let Some(snapshot) = app.buddy_event_sink.snapshot().await else {
        return String::new();
    };
    let mode_id = buddy_persona_cache_mode_id(mode_id);
    let archetype_id = snapshot.state.personality.archetype_id.clone();
    let identity_name = snapshot.state.identity.name.clone();
    let version = refact_buddy_core::state::persona_cache_version();
    let cache_key = (archetype_id, mode_id);
    let cache = buddy_persona_cache();
    let mut cache = cache.lock().await;
    if let Some(entry) = cache.get(&cache_key) {
        if entry.version == version && entry.identity_name == identity_name {
            return entry.rendered.clone();
        }
    }
    let rendered = refact_buddy_core::state::render_persona_block(&snapshot.state);
    cache.insert(
        cache_key,
        BuddyPersonaCacheEntry {
            version,
            identity_name,
            rendered: rendered.clone(),
        },
    );
    rendered
}

#[cfg(test)]
fn clear_buddy_persona_cache_for_tests() {
    if let Some(cache) = BUDDY_PERSONA_CACHE.get() {
        if let Ok(mut cache) = cache.try_lock() {
            cache.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_buddy_core::runtime_queue::RuntimeQueue;
    use refact_buddy_core::settings::BuddySettings;
    use crate::call_validation::{ChatContent, ChatMeta};
    use crate::tasks::types::{BoardCard, TaskBoard};
    use std::path::{Path, PathBuf};
    use tokio::sync::broadcast;

    struct ModePromptCase {
        label: &'static str,
        lookup_mode: &'static str,
        model_id: Option<&'static str>,
        cache_mode: &'static str,
    }

    const SPECIFIED_MODES: &[ModePromptCase] = &[
        ModePromptCase {
            label: "agent",
            lookup_mode: "agent",
            model_id: None,
            cache_mode: "agent",
        },
        ModePromptCase {
            label: "explore",
            lookup_mode: "explore",
            model_id: None,
            cache_mode: "explore",
        },
        ModePromptCase {
            label: "buddy",
            lookup_mode: "buddy",
            model_id: None,
            cache_mode: "buddy",
        },
        ModePromptCase {
            label: "task_planner",
            lookup_mode: "task_planner",
            model_id: None,
            cache_mode: "task_planner",
        },
        ModePromptCase {
            label: "task_agent",
            lookup_mode: "task_agent",
            model_id: None,
            cache_mode: "task_agent",
        },
        ModePromptCase {
            label: "setup",
            lookup_mode: "setup",
            model_id: None,
            cache_mode: "setup",
        },
        ModePromptCase {
            label: "learn",
            lookup_mode: "learn",
            model_id: None,
            cache_mode: "learn",
        },
        ModePromptCase {
            label: "plan",
            lookup_mode: "plan",
            model_id: None,
            cache_mode: "plan",
        },
        ModePromptCase {
            label: "quick_agent",
            lookup_mode: "quick_agent",
            model_id: None,
            cache_mode: "quick_agent",
        },
        ModePromptCase {
            label: "openai_agent",
            lookup_mode: "agent",
            model_id: Some("gpt-5"),
            cache_mode: "openai_agent",
        },
    ];

    fn prompt_test_lock() -> &'static tokio::sync::Mutex<()> {
        static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
    }

    async fn make_gcx_with_buddy() -> AppState {
        clear_buddy_persona_cache_for_tests();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let (tx, _) = broadcast::channel(16);
        let mut state = refact_buddy_core::state::default_buddy_state();
        state.identity.name = "Pixel".to_string();
        state.personality.archetype_id = "helper_sprite".to_string();
        state.personality.archetype_label = "Helper Sprite".to_string();
        state.personality.vibe = "Playful, quirky, helpful".to_string();
        state.personality.summary = "An energetic helper.".to_string();
        state.personality.prompt = "Use warm humor.".to_string();
        let service = crate::buddy::actor::BuddyService::new(
            std::env::temp_dir().join(format!("buddy-persona-test-{}", uuid::Uuid::new_v4())),
            state,
            BuddySettings::default(),
            Vec::new(),
            RuntimeQueue::new(),
            tx,
            None,
        );
        let buddy_arc = app.buddy.buddy.clone();
        *buddy_arc.lock().await = Some(service);
        app
    }

    async fn set_buddy_name(app: &AppState, name: &str) {
        let buddy_arc = app.buddy.buddy.clone();
        let mut lock = buddy_arc.lock().await;
        let service = lock.as_mut().unwrap();
        service.state.identity.name = name.to_string();
    }

    fn system_text(messages: &[ChatMessage]) -> &str {
        let Some(message) = messages.iter().find(|m| m.role == "system") else {
            panic!("system message not found");
        };
        match &message.content {
            ChatContent::SimpleText(text) => text,
            _ => panic!("system content must be simple text"),
        }
    }

    async fn task_memory_test_app(task_id: &str) -> (tempfile::TempDir, AppState, PathBuf) {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let temp = tempfile::tempdir().unwrap();
        {
            *gcx.documents_state.workspace_folders.lock().unwrap() =
                vec![temp.path().to_path_buf()];
        }
        let task_dir = temp.path().join(".refact/tasks").join(task_id);
        tokio::fs::create_dir_all(task_dir.join("memories"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(task_dir.join("documents"))
            .await
            .unwrap();
        let app = AppState::from_gcx(gcx).await;
        (temp, app, task_dir)
    }

    async fn write_task_memory(task_dir: &Path, name: &str, frontmatter: &str, body: &str) {
        tokio::fs::write(
            task_dir.join("memories").join(name),
            format!("---\n{}\n---\n\n{}", frontmatter.trim(), body),
        )
        .await
        .unwrap();
    }

    async fn write_task_document(task_dir: &Path, name: &str, frontmatter: &str, body: &str) {
        tokio::fs::write(
            task_dir.join("documents").join(name),
            format!("---\n{}\n---\n\n{}", frontmatter.trim(), body),
        )
        .await
        .unwrap();
    }

    async fn reset_task_briefing_tests() -> tokio::sync::MutexGuard<'static, ()> {
        let guard = prompt_test_lock().lock().await;
        clear_task_briefing_cache_for_tests().await;
        clear_task_briefing_test_state().await;
        guard
    }

    fn task_meta(task_id: &str, role: &str, card_id: Option<&str>) -> crate::chat::types::TaskMeta {
        crate::chat::types::TaskMeta {
            task_id: task_id.to_string(),
            role: role.to_string(),
            agent_id: None,
            card_id: card_id.map(|id| id.to_string()),
            planner_chat_id: None,
        }
    }

    fn board_card(id: &str, column: &str, depends_on: Vec<&str>) -> BoardCard {
        let now = chrono::Utc::now().to_rfc3339();
        BoardCard {
            id: id.to_string(),
            title: id.to_string(),
            column: column.to_string(),
            priority: "P1".to_string(),
            depends_on: depends_on.into_iter().map(str::to_string).collect(),
            instructions: String::new(),
            assignee: None,
            agent_chat_id: None,
            status_updates: Vec::new(),
            comments: vec![],
            final_report: None,
            final_report_structured: None,
            verifier_report: None,
            created_at: now,
            started_at: None,
            last_heartbeat_at: None,
            completed_at: None,
            agent_branch: None,
            agent_worktree: None,
            agent_worktree_name: None,
            ab_variants: None,
            team_members: vec![],
            target_files: Vec::new(),
            scope_guard_mode: Default::default(),
        }
    }

    async fn injected_task_memory_files(
        app: &AppState,
        task_id: &str,
        task_meta: Option<&crate::chat::types::TaskMeta>,
    ) -> Vec<ContextFile> {
        injected_task_context_files(app, task_id, task_meta, TASK_MEMORIES_CONTEXT_MARKER).await
    }

    async fn injected_task_context_files(
        app: &AppState,
        task_id: &str,
        task_meta: Option<&crate::chat::types::TaskMeta>,
        marker: &str,
    ) -> Vec<ContextFile> {
        let mut messages = vec![ChatMessage {
            role: "user".to_string(),
            content: ChatContent::SimpleText("hello".to_string()),
            ..Default::default()
        }];
        let mut stream_back_to_user = HasRagResults::new();
        inject_task_memories(
            app,
            &mut messages,
            &mut stream_back_to_user,
            Some(task_id.to_string()),
            task_meta,
        )
        .await
        .unwrap();
        let message = messages
            .iter()
            .find(|message| message.tool_call_id == marker)
            .unwrap();
        match &message.content {
            ChatContent::ContextFiles(files) => files.clone(),
            _ => panic!("task context must be context files"),
        }
    }

    fn combined_task_memory_content(files: &[ContextFile]) -> String {
        files
            .iter()
            .map(|file| file.file_content.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[tokio::test]
    async fn task_memory_injection_skips_archived_and_superseded() {
        let task_id = "task-memory-archived";
        let (_temp, app, task_dir) = task_memory_test_app(task_id).await;
        write_task_memory(
            &task_dir,
            "active.md",
            "created_at: 2026-05-22T00:00:03Z\nnamespace: task",
            "ACTIVE_MEMORY",
        )
        .await;
        write_task_memory(
            &task_dir,
            "archived.md",
            "created_at: 2026-05-22T00:00:02Z\nnamespace: task\nstatus: archived",
            "ARCHIVED_MEMORY",
        )
        .await;
        write_task_memory(
            &task_dir,
            "superseded.md",
            "created_at: 2026-05-22T00:00:01Z\nnamespace: task\nstatus: superseded",
            "SUPERSEDED_MEMORY",
        )
        .await;

        let meta = task_meta(task_id, "planner", None);
        let files = injected_task_memory_files(&app, task_id, Some(&meta)).await;
        let content = combined_task_memory_content(&files);

        assert!(content.contains("ACTIVE_MEMORY"));
        assert!(!content.contains("ARCHIVED_MEMORY"));
        assert!(!content.contains("SUPERSEDED_MEMORY"));
        assert!(content.contains(
            "Showing 1 of 3 memories (2 archived/superseded skipped, 0 dropped over budget)."
        ));
    }

    #[tokio::test]
    async fn task_memory_injection_keeps_pinned_memories_over_budget() {
        let task_id = "task-memory-pinned";
        let (_temp, app, task_dir) = task_memory_test_app(task_id).await;
        let large_body = "x".repeat(MAX_TASK_MEMORY_CONTENT_SIZE + 200);
        for idx in 0..10 {
            write_task_memory(
                &task_dir,
                &format!("bulk-{}.md", idx),
                &format!("created_at: 2026-05-22T00:00:{:02}Z\nnamespace: task", idx),
                &large_body,
            )
            .await;
        }
        write_task_memory(
            &task_dir,
            "pinned.md",
            "created_at: 2026-05-22T00:01:00Z\nnamespace: card:other\npinned: true",
            "PINNED_MEMORY",
        )
        .await;

        let meta = task_meta(task_id, "agents", Some("T-1"));
        let files = injected_task_memory_files(&app, task_id, Some(&meta)).await;
        let content = combined_task_memory_content(&files);

        assert!(content.contains("PINNED_MEMORY"));
        assert!(content.contains("dropped over budget"));
    }

    #[tokio::test]
    async fn task_memory_injection_agent_uses_card_global_task_and_dependencies() {
        let task_id = "task-memory-agent-scope";
        let (_temp, app, task_dir) = task_memory_test_app(task_id).await;
        crate::tasks::storage::save_board(
            app.gcx.clone(),
            task_id,
            &TaskBoard {
                cards: vec![
                    board_card("T-1", "done", vec![]),
                    board_card("T-2", "planned", vec![]),
                    board_card("T-3", "doing", vec!["T-1"]),
                ],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        write_task_memory(&task_dir, "global.md", "namespace: global", "GLOBAL_MEMORY").await;
        write_task_memory(&task_dir, "task.md", "namespace: task", "TASK_MEMORY").await;
        write_task_memory(
            &task_dir,
            "current.md",
            "namespace: card:T-3",
            "CURRENT_CARD_MEMORY",
        )
        .await;
        write_task_memory(
            &task_dir,
            "dependency.md",
            "namespace: card:T-1",
            "DEPENDENCY_MEMORY",
        )
        .await;
        write_task_memory(
            &task_dir,
            "other.md",
            "namespace: card:T-2",
            "OTHER_CARD_MEMORY",
        )
        .await;

        let meta = task_meta(task_id, "agents", Some("T-3"));
        let files = injected_task_memory_files(&app, task_id, Some(&meta)).await;
        let content = combined_task_memory_content(&files);

        assert!(content.contains("GLOBAL_MEMORY"));
        assert!(content.contains("TASK_MEMORY"));
        assert!(content.contains("CURRENT_CARD_MEMORY"));
        assert!(content.contains("DEPENDENCY_MEMORY"));
        assert!(!content.contains("OTHER_CARD_MEMORY"));
    }

    #[tokio::test]
    async fn task_memory_injection_planner_uses_doing_card_global_and_task() {
        let task_id = "task-memory-planner-scope";
        let (_temp, app, task_dir) = task_memory_test_app(task_id).await;
        crate::tasks::storage::save_board(
            app.gcx.clone(),
            task_id,
            &TaskBoard {
                cards: vec![
                    board_card("T-1", "doing", vec![]),
                    board_card("T-2", "planned", vec![]),
                ],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        write_task_memory(&task_dir, "global.md", "namespace: global", "GLOBAL_MEMORY").await;
        write_task_memory(&task_dir, "task.md", "namespace: task", "TASK_MEMORY").await;
        write_task_memory(
            &task_dir,
            "doing.md",
            "namespace: card:T-1",
            "DOING_CARD_MEMORY",
        )
        .await;
        write_task_memory(
            &task_dir,
            "planned.md",
            "namespace: card:T-2",
            "PLANNED_CARD_MEMORY",
        )
        .await;

        let meta = task_meta(task_id, "planner", None);
        let files = injected_task_memory_files(&app, task_id, Some(&meta)).await;
        let content = combined_task_memory_content(&files);

        assert!(content.contains("GLOBAL_MEMORY"));
        assert!(content.contains("TASK_MEMORY"));
        assert!(content.contains("DOING_CARD_MEMORY"));
        assert!(!content.contains("PLANNED_CARD_MEMORY"));
    }

    #[tokio::test]
    async fn task_memory_injection_budget_footer_counts_drops() {
        let task_id = "task-memory-budget";
        let (_temp, app, task_dir) = task_memory_test_app(task_id).await;
        let large_body = "y".repeat(MAX_TASK_MEMORY_CONTENT_SIZE + 200);
        for idx in 0..10 {
            write_task_memory(
                &task_dir,
                &format!("memory-{}.md", idx),
                &format!("created_at: 2026-05-22T00:00:{:02}Z\nnamespace: task", idx),
                &large_body,
            )
            .await;
        }

        let meta = task_meta(task_id, "planner", None);
        let files = injected_task_memory_files(&app, task_id, Some(&meta)).await;
        let content = combined_task_memory_content(&files);

        assert!(content.contains(
            "Showing 8 of 10 memories (0 archived/superseded skipped, 2 dropped over budget)."
        ));
    }

    #[tokio::test]
    async fn task_briefing_small_content_skips_briefing() {
        let _guard = reset_task_briefing_tests().await;
        let task_id = "task-briefing-small";
        let (_temp, app, task_dir) = task_memory_test_app(task_id).await;
        set_task_briefing_test_responses(
            task_id,
            vec![Ok("## Active Plans\n- should not be used".to_string())],
        )
        .await;
        write_task_memory(
            &task_dir,
            "small.md",
            "created_at: 2026-05-22T00:00:00Z\nnamespace: task",
            "SMALL",
        )
        .await;

        let meta = task_meta(task_id, "planner", None);
        let files = injected_task_memory_files(&app, task_id, Some(&meta)).await;
        let content = combined_task_memory_content(&files);

        assert!(content.contains("SMALL"));
        assert_eq!(task_briefing_test_calls().await, 0);
    }

    #[tokio::test]
    async fn task_briefing_cached_without_second_subchat_call() {
        let _guard = reset_task_briefing_tests().await;
        let task_id = "task-briefing-cache";
        let (_temp, app, task_dir) = task_memory_test_app(task_id).await;
        set_task_briefing_test_responses(
            task_id,
            vec![Ok("## Active Plans\n- Use cached plan [cache.md]\n\n## Key Decisions\n- Keep cache [cache.md]\n\n## Gotchas / Risks\n- None\n\n## Current State\n- Ready".to_string())],
        )
        .await;
        write_task_memory(
            &task_dir,
            "large.md",
            "created_at: 2026-05-22T00:00:00Z\nnamespace: task",
            &"CACHE_MEMORY ".repeat(300),
        )
        .await;

        let meta = task_meta(task_id, "planner", None);
        let first =
            injected_task_context_files(&app, task_id, Some(&meta), TASK_BRIEFING_CONTEXT_MARKER)
                .await;
        let second =
            injected_task_context_files(&app, task_id, Some(&meta), TASK_BRIEFING_CONTEXT_MARKER)
                .await;

        assert_eq!(task_briefing_test_calls().await, 1);
        assert!(combined_task_memory_content(&first).contains("Use cached plan"));
        assert_eq!(
            combined_task_memory_content(&first),
            combined_task_memory_content(&second)
        );
    }

    #[tokio::test]
    async fn task_briefing_failure_falls_back_to_raw() {
        let _guard = reset_task_briefing_tests().await;
        let task_id = "task-briefing-fallback";
        let (_temp, app, task_dir) = task_memory_test_app(task_id).await;
        set_task_briefing_test_responses(task_id, vec![Err("briefing unavailable".to_string())])
            .await;
        write_task_memory(
            &task_dir,
            "large.md",
            "created_at: 2026-05-22T00:00:00Z\nnamespace: task",
            &"FALLBACK_MEMORY ".repeat(250),
        )
        .await;

        let meta = task_meta(task_id, "planner", None);
        let files = injected_task_memory_files(&app, task_id, Some(&meta)).await;
        let content = combined_task_memory_content(&files);

        assert_eq!(task_briefing_test_calls().await, 1);
        assert!(content.contains("FALLBACK_MEMORY"));
        assert!(content.contains("Showing 1 of 1 memories"));
    }

    #[tokio::test]
    async fn task_briefing_renders_expected_sections_and_pinned_documents() {
        let _guard = reset_task_briefing_tests().await;
        let task_id = "task-briefing-render";
        let prompt = task_briefing_prompt("agents", "SOURCE");
        assert!(prompt.contains("## Active Plans"));
        assert!(prompt.contains("## Key Decisions"));
        assert!(prompt.contains("## Gotchas / Risks"));
        assert!(prompt.contains("## Current State"));
        let (_temp, app, task_dir) = task_memory_test_app(task_id).await;
        set_task_briefing_test_responses(
            task_id,
            vec![Ok("## Active Plans\n- Follow the rollout plan [plan.md]\n\n## Key Decisions\n- Use task documents [brief.md]\n\n## Gotchas / Risks\n- Watch cache invalidation [risk.md]\n\n## Current State\n- Implementation is in progress".to_string())],
        )
        .await;
        write_task_memory(
            &task_dir,
            "risk.md",
            "created_at: 2026-05-22T00:00:01Z\nnamespace: task\nkind: risk",
            &"Risk details ".repeat(180),
        )
        .await;
        write_task_document(
            &task_dir,
            "brief.md",
            "name: Brief\nslug: brief\nkind: brief\ncreated_at: 2026-05-22T00:00:02Z\nupdated_at: 2026-05-22T00:00:02Z\nauthor_role: planner\npinned: true\nversion: 1",
            &"Pinned task document ".repeat(80),
        )
        .await;

        let meta = task_meta(task_id, "agents", Some("T-1"));
        let files =
            injected_task_context_files(&app, task_id, Some(&meta), TASK_BRIEFING_CONTEXT_MARKER)
                .await;
        let content = combined_task_memory_content(&files);

        assert!(content.contains("## Active Plans"));
        assert!(content.contains("## Key Decisions"));
        assert!(content.contains("## Gotchas / Risks"));
        assert!(content.contains("## Current State"));
        assert!(content.contains("[brief.md]"));
    }

    #[tokio::test]
    async fn every_specified_mode_substitutes_buddy_personality_marker() {
        let _guard = prompt_test_lock().lock().await;
        let app = make_gcx_with_buddy().await;

        for case in SPECIFIED_MODES {
            let prompt = get_mode_system_prompt(app.clone(), case.lookup_mode, case.model_id).await;
            assert!(
                prompt.contains(BUDDY_PERSONALITY_MARKER),
                "mode missing marker: {}",
                case.label
            );
            let rendered = system_prompt_add_extra_instructions(
                app.clone(),
                prompt,
                HashSet::new(),
                &ChatMeta {
                    chat_mode: case.lookup_mode.to_string(),
                    include_project_info: false,
                    ..Default::default()
                },
                &None,
                case.cache_mode,
            )
            .await;

            assert!(rendered.contains("You are Pixel, a Helper Sprite"));
            assert!(!rendered.contains(BUDDY_PERSONALITY_MARKER));
        }
    }

    #[tokio::test]
    async fn personality_block_caches_per_archetype_and_mode() {
        let _guard = prompt_test_lock().lock().await;
        let app = make_gcx_with_buddy().await;
        let prompt = format!("before\n{}\nafter", BUDDY_PERSONALITY_MARKER);

        let first = system_prompt_add_extra_instructions(
            app.clone(),
            prompt.clone(),
            HashSet::new(),
            &ChatMeta::default(),
            &None,
            "agent",
        )
        .await;
        set_buddy_name(&app, "Nova").await;
        let cached_same_mode = system_prompt_add_extra_instructions(
            app.clone(),
            prompt.clone(),
            HashSet::new(),
            &ChatMeta::default(),
            &None,
            "agent",
        )
        .await;
        let uncached_other_mode = system_prompt_add_extra_instructions(
            app.clone(),
            prompt,
            HashSet::new(),
            &ChatMeta::default(),
            &None,
            "explore",
        )
        .await;

        assert!(first.contains("You are Pixel"));
        assert!(cached_same_mode.contains("You are Nova"));
        assert!(uncached_other_mode.contains("You are Nova"));
    }

    #[tokio::test]
    async fn persona_cache_invalidates_on_identity_name_change() {
        let _guard = prompt_test_lock().lock().await;
        let app = make_gcx_with_buddy().await;
        let prompt = format!("{}", BUDDY_PERSONALITY_MARKER);
        let first = system_prompt_add_extra_instructions(
            app.clone(),
            prompt.clone(),
            HashSet::new(),
            &ChatMeta::default(),
            &None,
            "agent",
        )
        .await;

        set_buddy_name(&app, "Nova").await;
        let second = system_prompt_add_extra_instructions(
            app.clone(),
            prompt,
            HashSet::new(),
            &ChatMeta::default(),
            &None,
            "agent",
        )
        .await;

        assert!(first.contains("You are Pixel"));
        assert!(second.contains("You are Nova"));
    }

    #[tokio::test]
    async fn personality_cache_invalidates_on_reroll() {
        let _guard = prompt_test_lock().lock().await;
        let app = make_gcx_with_buddy().await;
        let prompt = format!("{}", BUDDY_PERSONALITY_MARKER);
        let first = system_prompt_add_extra_instructions(
            app.clone(),
            prompt.clone(),
            HashSet::new(),
            &ChatMeta::default(),
            &None,
            "agent",
        )
        .await;

        set_buddy_name(&app, "Nova").await;
        refact_buddy_core::state::mark_persona_cache_dirty();
        let second = system_prompt_add_extra_instructions(
            app.clone(),
            prompt,
            HashSet::new(),
            &ChatMeta::default(),
            &None,
            "agent",
        )
        .await;

        assert!(first.contains("You are Pixel"));
        assert!(second.contains("You are Nova"));
    }

    #[tokio::test]
    async fn missing_buddy_state_renders_empty_marker_replacement() {
        let _guard = prompt_test_lock().lock().await;
        clear_buddy_persona_cache_for_tests();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let rendered = system_prompt_add_extra_instructions(
            app,
            format!("Alpha\n{}\nOmega", BUDDY_PERSONALITY_MARKER),
            HashSet::new(),
            &ChatMeta::default(),
            &None,
            "agent",
        )
        .await;

        assert!(!rendered.contains(BUDDY_PERSONALITY_MARKER));
        assert!(rendered.contains("Alpha\n\nOmega"));
    }

    #[tokio::test]
    async fn gather_and_inject_includes_personality_in_system_prompt() {
        let _guard = prompt_test_lock().lock().await;
        let app = make_gcx_with_buddy().await;
        let mut stream_back_to_user = HasRagResults::new();
        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: ChatContent::SimpleText("hello".to_string()),
            ..Default::default()
        }];

        let (messages, _) = prepend_the_right_system_prompt_and_maybe_more_initial_messages(
            app,
            messages,
            &ChatMeta {
                chat_mode: "agent".to_string(),
                include_project_info: false,
                ..Default::default()
            },
            &None,
            &mut stream_back_to_user,
            HashSet::new(),
            "agent",
            "",
        )
        .await;

        let content = system_text(&messages);
        assert!(content.contains("You are Pixel, a Helper Sprite"));
        assert!(!content.contains(BUDDY_PERSONALITY_MARKER));
    }
}

pub async fn prepend_the_right_system_prompt_and_maybe_more_initial_messages(
    app: AppState,
    mut messages: Vec<call_validation::ChatMessage>,
    chat_meta: &call_validation::ChatMeta,
    task_meta: &Option<crate::chat::types::TaskMeta>,
    stream_back_to_user: &mut HasRagResults,
    tool_names: HashSet<String>,
    mode_id: &str,
    model_id: &str,
) -> (Vec<call_validation::ChatMessage>, SkillsTrackingInfo) {
    if messages.is_empty() {
        tracing::error!("What's that? Messages list is empty");
        return (messages, SkillsTrackingInfo::default());
    }

    let have_system = messages
        .first()
        .map(|m| m.role == "system")
        .unwrap_or(false);
    let have_project_context = messages
        .iter()
        .any(|m| m.role == "context_file" && m.tool_call_id == PROJECT_CONTEXT_MARKER);

    if !have_system {
        let canonical_mode =
            canonical_mode_id(&chat_meta.chat_mode).unwrap_or_else(|_| "agent".to_string());
        match canonical_mode.as_str() {
            "configurator" => {
                crate::integrations::config_chat::mix_config_messages(
                    app.gcx.clone(),
                    &chat_meta,
                    &mut messages,
                    stream_back_to_user,
                )
                .await;
            }
            "setup" => {
                crate::integrations::setup_chat::mix_setup_messages(
                    app.gcx.clone(),
                    &chat_meta,
                    &mut messages,
                    stream_back_to_user,
                )
                .await;
            }
            _ => {
                let base_prompt =
                    get_mode_system_prompt(app.clone(), mode_id, Some(model_id)).await;
                let system_message_content = system_prompt_add_extra_instructions(
                    app.clone(),
                    base_prompt,
                    tool_names,
                    chat_meta,
                    task_meta,
                    mode_id,
                )
                .await;
                let msg = ChatMessage {
                    role: "system".to_string(),
                    content: ChatContent::SimpleText(system_message_content),
                    ..Default::default()
                };
                stream_back_to_user.push_in_json(serde_json::json!(msg));
                messages.insert(0, msg);
            }
        }
    }

    let mut skills_tracking = SkillsTrackingInfo::default();
    if chat_meta.include_project_info && !have_project_context {
        match gather_and_inject_system_context(&app, &mut messages, stream_back_to_user).await {
            Ok(info) => {
                skills_tracking = info;
            }
            Err(e) => {
                tracing::warn!("Failed to gather system context: {}", e);
            }
        }
    } else if !chat_meta.include_project_info {
        tracing::info!("Skipping project/system context injection (include_project_info=false)");
    }

    let canonical_chat_mode =
        canonical_mode_id(&chat_meta.chat_mode).unwrap_or_else(|_| "agent".to_string());
    if matches!(canonical_chat_mode.as_str(), "task_planner" | "task_agent") {
        let task_id_opt = task_meta
            .as_ref()
            .map(|m| m.task_id.clone())
            .or_else(|| infer_task_id_from_chat_id(&chat_meta.chat_id));
        match inject_task_memories(
            &app,
            &mut messages,
            stream_back_to_user,
            task_id_opt,
            task_meta.as_ref(),
        )
        .await
        {
            Ok(()) => {}
            Err(e) => {
                tracing::warn!("Failed to inject task memories: {}", e);
            }
        }
    }

    tracing::info!(
        "\n\nSYSTEM PROMPT MIXER chat_mode={:?}",
        chat_meta.chat_mode
    );
    (messages, skills_tracking)
}

const TASK_MEMORIES_CONTEXT_MARKER: &str = "task_memories_context";
const TASK_BRIEFING_CONTEXT_MARKER: &str = "task_briefing";
const MAX_TASK_MEMORY_CONTENT_SIZE: usize = 3000;
const MAX_TASK_MEMORIES_TOTAL_SIZE: usize = 25_000;
const MIN_TASK_BRIEFING_SOURCE_SIZE: usize = 2_000;
const MAX_TASK_BRIEFING_SIZE: usize = 5_000;

#[derive(Clone)]
struct TaskBriefingCacheEntry {
    briefing: String,
}

static TASK_BRIEFING_CACHE: OnceLock<AMutex<HashMap<(String, String), TaskBriefingCacheEntry>>> =
    OnceLock::new();

fn task_briefing_cache() -> &'static AMutex<HashMap<(String, String), TaskBriefingCacheEntry>> {
    TASK_BRIEFING_CACHE.get_or_init(|| AMutex::new(HashMap::new()))
}

#[cfg(test)]
#[derive(Default)]
struct TaskBriefingTestState {
    enabled_task_id: Option<String>,
    calls: usize,
    responses: std::collections::VecDeque<Result<String, String>>,
}

#[cfg(test)]
static TASK_BRIEFING_TEST_STATE: OnceLock<AMutex<TaskBriefingTestState>> = OnceLock::new();

#[cfg(test)]
fn task_briefing_test_state() -> &'static AMutex<TaskBriefingTestState> {
    TASK_BRIEFING_TEST_STATE.get_or_init(|| AMutex::new(TaskBriefingTestState::default()))
}

#[cfg(test)]
async fn task_briefing_test_response(task_id: &str) -> Result<String, String> {
    let mut state = task_briefing_test_state().lock().await;
    if state.enabled_task_id.as_deref() != Some(task_id) {
        return Err("task briefing subchat disabled in tests".to_string());
    }
    state.calls += 1;
    state
        .responses
        .pop_front()
        .unwrap_or_else(|| Err("task briefing test response missing".to_string()))
}

#[cfg(test)]
async fn set_task_briefing_test_responses(task_id: &str, responses: Vec<Result<String, String>>) {
    let mut state = task_briefing_test_state().lock().await;
    state.enabled_task_id = Some(task_id.to_string());
    state.calls = 0;
    state.responses = responses.into_iter().collect();
}

#[cfg(test)]
async fn clear_task_briefing_test_state() {
    if let Some(state) = TASK_BRIEFING_TEST_STATE.get() {
        let mut state = state.lock().await;
        *state = TaskBriefingTestState::default();
    }
}

#[cfg(test)]
async fn task_briefing_test_calls() -> usize {
    task_briefing_test_state().lock().await.calls
}

#[cfg(test)]
async fn clear_task_briefing_cache_for_tests() {
    if let Some(cache) = TASK_BRIEFING_CACHE.get() {
        cache.lock().await.clear();
    }
}

struct TaskMemoryForInjection {
    path: PathBuf,
    content: String,
    frontmatter: TaskMemoryFrontmatter,
    updated_at: String,
}

#[derive(Clone, Copy)]
enum TaskContextEntryKind {
    Memory,
    Document,
}

struct TaskContextEntry {
    path: PathBuf,
    content: String,
    kind: TaskContextEntryKind,
    updated_at: String,
    fingerprint_content: String,
}

struct TaskContextInjectionPlan {
    entries: Vec<TaskContextEntry>,
    footer: String,
    included_memories: usize,
    pinned_documents: usize,
    archived_skipped: usize,
    dropped_over_budget: usize,
    scope_skipped: usize,
}

fn split_task_memory_frontmatter(content: &str) -> Result<(Option<&str>, &str), String> {
    let delimiter_len = if content.starts_with("---\r\n") {
        5
    } else if content.starts_with("---\n") {
        4
    } else {
        return Ok((None, content));
    };

    let mut position = delimiter_len;
    while position < content.len() {
        let line_end = content[position..]
            .find('\n')
            .map(|offset| position + offset + 1)
            .unwrap_or(content.len());
        let line = &content[position..line_end];
        let trimmed = line.trim_end_matches(&['\r', '\n'][..]).trim();
        if trimmed == "---" {
            return Ok((
                Some(&content[delimiter_len..position]),
                &content[line_end..],
            ));
        }
        position = line_end;
    }

    Err("Invalid memory file: missing closing frontmatter delimiter".to_string())
}

fn frontmatter_string_value(mapping: &serde_yaml::Mapping, key: &str) -> Option<String> {
    mapping
        .get(&serde_yaml::Value::String(key.to_string()))
        .and_then(|value| match value {
            serde_yaml::Value::String(value) => Some(value.clone()),
            serde_yaml::Value::Number(value) => Some(value.to_string()),
            serde_yaml::Value::Bool(value) => Some(value.to_string()),
            _ => None,
        })
}

fn frontmatter_updated_at(frontmatter: &str) -> Option<String> {
    match serde_yaml::from_str::<serde_yaml::Value>(frontmatter).ok()? {
        serde_yaml::Value::Mapping(mapping) => frontmatter_string_value(&mapping, "updated_at")
            .or_else(|| frontmatter_string_value(&mapping, "created_at")),
        _ => None,
    }
}

fn parse_task_memory_for_injection(path: &PathBuf, content: &str) -> TaskMemoryForInjection {
    let (frontmatter, parsed) = match split_task_memory_frontmatter(content) {
        Ok((frontmatter, _)) => {
            let parsed = frontmatter
                .and_then(|text| TaskMemoryFrontmatter::from_yaml(text).ok())
                .unwrap_or_default();
            (frontmatter, parsed)
        }
        Err(err) => {
            tracing::warn!("Failed to parse task memory {}: {}", path.display(), err);
            (None, TaskMemoryFrontmatter::default())
        }
    };
    let updated_at = frontmatter
        .and_then(frontmatter_updated_at)
        .or_else(|| parsed.created_at.clone())
        .or_else(|| {
            path.file_name()
                .map(|name| name.to_string_lossy().to_string())
        })
        .unwrap_or_default();

    TaskMemoryForInjection {
        path: path.clone(),
        content: content.to_string(),
        frontmatter: parsed,
        updated_at,
    }
}

fn high_signal_memory_kind(kind: MemoryKind) -> bool {
    matches!(
        kind,
        MemoryKind::Decision
            | MemoryKind::Spec
            | MemoryKind::Gotcha
            | MemoryKind::Risk
            | MemoryKind::Handoff
            | MemoryKind::Brief
            | MemoryKind::Postmortem
    )
}

fn truncate_task_memory_content(content: &str) -> String {
    if content.len() > MAX_TASK_MEMORY_CONTENT_SIZE {
        format!(
            "{}\n\n[TRUNCATED]",
            content
                .chars()
                .take(MAX_TASK_MEMORY_CONTENT_SIZE)
                .collect::<String>()
        )
    } else {
        content.to_string()
    }
}

fn task_memory_context_file(path: &PathBuf, content: String, usefulness: f32) -> ContextFile {
    let line_count = content.lines().count().max(1);
    ContextFile {
        file_name: path.to_string_lossy().to_string(),
        file_content: content,
        line1: 1,
        line2: line_count,
        file_rev: None,
        symbols: vec![],
        gradient_type: -1,
        usefulness,
        skip_pp: true,
    }
}

fn task_memory_summary_context_file(content: String) -> ContextFile {
    task_memory_context_file(&PathBuf::from("(task memories summary)"), content, 50.0)
}

fn task_briefing_context_file(content: String) -> ContextFile {
    task_memory_context_file(&PathBuf::from("(task briefing)"), content, 100.0)
}

fn task_context_entry_context_file(entry: &TaskContextEntry) -> ContextFile {
    let usefulness = match entry.kind {
        TaskContextEntryKind::Memory => 95.0,
        TaskContextEntryKind::Document => 90.0,
    };
    task_memory_context_file(&entry.path, entry.content.clone(), usefulness)
}

fn sort_task_memory_bucket(bucket: &mut [TaskMemoryForInjection]) {
    bucket.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| b.path.cmp(&a.path))
    });
}

fn include_task_memory_bucket(
    bucket: Vec<TaskMemoryForInjection>,
    force_include: bool,
    entries: &mut Vec<TaskContextEntry>,
    total_size: &mut usize,
    included_count: &mut usize,
    dropped_over_budget: &mut usize,
) {
    for memory in bucket {
        let truncated_content = truncate_task_memory_content(&memory.content);
        let content_size = truncated_content.len();
        if !force_include && *total_size + content_size > MAX_TASK_MEMORIES_TOTAL_SIZE {
            *dropped_over_budget += 1;
            continue;
        }
        *total_size += content_size;
        *included_count += 1;
        entries.push(TaskContextEntry {
            path: memory.path,
            content: truncated_content,
            kind: TaskContextEntryKind::Memory,
            updated_at: memory.updated_at,
            fingerprint_content: memory.content,
        });
    }
}

fn frontmatter_bool_value(mapping: &serde_yaml::Mapping, key: &str) -> Option<bool> {
    mapping
        .get(&serde_yaml::Value::String(key.to_string()))
        .and_then(|value| match value {
            serde_yaml::Value::Bool(value) => Some(*value),
            serde_yaml::Value::String(value) => match value.as_str() {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            },
            _ => None,
        })
}

fn pinned_task_document_for_injection(path: PathBuf, content: String) -> Option<TaskContextEntry> {
    let (frontmatter, _) = split_task_memory_frontmatter(&content).ok()?;
    let frontmatter = frontmatter?;
    let mapping = match serde_yaml::from_str::<serde_yaml::Value>(frontmatter).ok()? {
        serde_yaml::Value::Mapping(mapping) => mapping,
        _ => return None,
    };
    if !frontmatter_bool_value(&mapping, "pinned").unwrap_or(false) {
        return None;
    }
    let updated_at = frontmatter_string_value(&mapping, "updated_at")
        .or_else(|| frontmatter_string_value(&mapping, "created_at"))
        .or_else(|| {
            path.file_name()
                .map(|name| name.to_string_lossy().to_string())
        })
        .unwrap_or_default();
    Some(TaskContextEntry {
        path,
        content: truncate_task_memory_content(&content),
        kind: TaskContextEntryKind::Document,
        updated_at,
        fingerprint_content: content,
    })
}

async fn load_pinned_task_documents(
    app: &AppState,
    task_id: &str,
) -> Result<Vec<TaskContextEntry>, String> {
    let task_dir = crate::tasks::storage::find_task_dir(app.gcx.clone(), task_id).await?;
    let documents_dir = task_dir.join("documents");
    if !documents_dir.exists() {
        return Ok(Vec::new());
    }
    let mut entries = tokio::fs::read_dir(&documents_dir)
        .await
        .map_err(|e| format!("failed to read {}: {}", documents_dir.display(), e))?;
    let mut documents = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| format!("failed to read {}: {}", documents_dir.display(), e))?
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if extension != "md" && extension != "mdx" {
            continue;
        }
        match tokio::fs::read_to_string(&path).await {
            Ok(content) => {
                if let Some(document) = pinned_task_document_for_injection(path, content) {
                    documents.push(document);
                }
            }
            Err(e) => {
                tracing::warn!("Failed to read task document file {:?}: {}", path, e);
            }
        }
    }
    documents.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| b.path.cmp(&a.path))
    });
    Ok(documents)
}

async fn build_task_context_injection_plan(
    app: &AppState,
    task_id: &str,
    task_meta: Option<&crate::chat::types::TaskMeta>,
) -> Result<Option<TaskContextInjectionPlan>, String> {
    let memories = app.tool_registry.load_task_memories(task_id).await?;
    let pinned_documents = load_pinned_task_documents(app, task_id).await?;
    if memories.is_empty() && pinned_documents.is_empty() {
        return Ok(None);
    }

    let board = crate::tasks::storage::load_board(app.gcx.clone(), task_id)
        .await
        .ok();
    let role = task_meta
        .map(|meta| meta.role.as_str())
        .unwrap_or("planner");
    let current_card_id = task_meta.and_then(|meta| meta.card_id.as_deref());
    let dependency_cards: HashSet<String> = if role == "agents" {
        board
            .as_ref()
            .and_then(|board| current_card_id.and_then(|card_id| board.get_card(card_id)))
            .map(|card| card.depends_on.iter().cloned().collect())
            .unwrap_or_default()
    } else {
        HashSet::new()
    };
    let doing_cards: HashSet<String> = board
        .as_ref()
        .map(|board| {
            board
                .cards
                .iter()
                .filter(|card| card.column == "doing")
                .map(|card| card.id.clone())
                .collect()
        })
        .unwrap_or_default();

    let mut always_bucket = Vec::new();
    let mut scoped_bucket = Vec::new();
    let mut less_relevant_bucket = Vec::new();
    let mut archived_skipped = 0usize;
    let mut scope_skipped = 0usize;

    for (path, content) in &memories {
        let memory = parse_task_memory_for_injection(path, content);
        if matches!(
            memory.frontmatter.status,
            MemoryStatus::Archived | MemoryStatus::Superseded
        ) {
            archived_skipped += 1;
            continue;
        }

        if memory.frontmatter.pinned || high_signal_memory_kind(memory.frontmatter.kind) {
            always_bucket.push(memory);
            continue;
        }

        let scope_relevant = match &memory.frontmatter.namespace {
            MemoryNamespace::Global | MemoryNamespace::Task => true,
            MemoryNamespace::Card(card_id) if role == "planner" => doing_cards.contains(card_id),
            MemoryNamespace::Card(card_id) if role == "agents" => {
                current_card_id == Some(card_id.as_str()) || dependency_cards.contains(card_id)
            }
            MemoryNamespace::Card(_) => false,
            MemoryNamespace::Agent(_) => false,
        };

        if scope_relevant {
            scoped_bucket.push(memory);
        } else if matches!(
            memory.frontmatter.kind,
            MemoryKind::Progress | MemoryKind::Freeform
        ) && !matches!(memory.frontmatter.namespace, MemoryNamespace::Card(_))
        {
            less_relevant_bucket.push(memory);
        } else {
            scope_skipped += 1;
        }
    }

    sort_task_memory_bucket(&mut always_bucket);
    sort_task_memory_bucket(&mut scoped_bucket);
    sort_task_memory_bucket(&mut less_relevant_bucket);

    let mut entries = Vec::new();
    let mut total_size = 0usize;
    let mut included_count = 0usize;
    let mut dropped_over_budget = 0usize;

    include_task_memory_bucket(
        always_bucket,
        true,
        &mut entries,
        &mut total_size,
        &mut included_count,
        &mut dropped_over_budget,
    );
    include_task_memory_bucket(
        scoped_bucket,
        false,
        &mut entries,
        &mut total_size,
        &mut included_count,
        &mut dropped_over_budget,
    );
    include_task_memory_bucket(
        less_relevant_bucket,
        false,
        &mut entries,
        &mut total_size,
        &mut included_count,
        &mut dropped_over_budget,
    );

    let pinned_document_count = pinned_documents.len();
    entries.extend(pinned_documents);

    let mut footer = format!(
        "Showing {} of {} memories ({} archived/superseded skipped, {} dropped over budget). Search more with task_mem_search().",
        included_count,
        memories.len(),
        archived_skipped,
        dropped_over_budget
    );
    if scope_skipped > 0 {
        footer.push_str(&format!(" {} outside scope skipped.", scope_skipped));
    }
    if pinned_document_count > 0 {
        footer.push_str(&format!(
            " {} pinned task document{} included.",
            pinned_document_count,
            if pinned_document_count == 1 { "" } else { "s" }
        ));
    }

    Ok(Some(TaskContextInjectionPlan {
        entries,
        footer,
        included_memories: included_count,
        pinned_documents: pinned_document_count,
        archived_skipped,
        dropped_over_budget,
        scope_skipped,
    }))
}

fn task_context_raw_files(plan: &TaskContextInjectionPlan) -> Vec<ContextFile> {
    let mut context_files = plan
        .entries
        .iter()
        .map(task_context_entry_context_file)
        .collect::<Vec<_>>();
    context_files.push(task_memory_summary_context_file(plan.footer.clone()));
    context_files
}

fn task_context_source_text(entries: &[TaskContextEntry]) -> String {
    entries
        .iter()
        .map(|entry| {
            let entry_type = match entry.kind {
                TaskContextEntryKind::Memory => "Memory",
                TaskContextEntryKind::Document => "Document",
            };
            format!(
                "### {} [{}]\n{}\n",
                entry_type,
                entry.path.to_string_lossy(),
                entry.content.trim()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn task_briefing_cache_hash(source: &str) -> String {
    let mut h = Sha256::new();
    h.update(source.as_bytes());
    hex::encode(h.finalize())
}

fn task_context_cache_material(entries: &[TaskContextEntry]) -> String {
    entries
        .iter()
        .map(|entry| {
            let entry_type = match entry.kind {
                TaskContextEntryKind::Memory => "Memory",
                TaskContextEntryKind::Document => "Document",
            };
            format!(
                "{}\n{}\n{}\n",
                entry_type,
                entry.path.to_string_lossy(),
                entry.fingerprint_content
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn task_briefing_prompt(role: &str, source: &str) -> String {
    format!(
        "Summarize the following project memories and task documents into a \"What you need to know\" briefing for the {} on this task.\n\nFormat:\n## Active Plans\n- bullet\n\n## Key Decisions\n- bullet (cite source [path])\n\n## Gotchas / Risks\n- bullet (cite source [path])\n\n## Current State\n- bullet\n\nKeep to 8 bullets total. Cite memory paths in [brackets] so the agent can cat them.\n\nMemories and documents:\n{}",
        role, source
    )
}

async fn run_task_briefing_subchat(
    app: &AppState,
    _task_id: &str,
    role: &str,
    source: &str,
) -> Result<String, String> {
    #[cfg(test)]
    {
        let _ = (app, role, source);
        let briefing = task_briefing_test_response(_task_id).await?;
        return Ok(crate::llm::safe_truncate(&briefing, MAX_TASK_BRIEFING_SIZE)
            .trim()
            .to_string());
    }

    #[cfg(not(test))]
    {
        let app = app.clone();
        let role = role.to_string();
        let source = source.to_string();
        tokio::task::spawn_blocking(move || -> Result<String, String> {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| format!("failed to start task briefing runtime: {}", e))?;
            runtime.block_on(async move {
                let caps =
                    crate::global_context::try_load_caps_quickly_if_not_present(app.gcx.clone(), 0)
                        .await
                        .map_err(|e| e.message.clone())?;
                let model = if !caps.defaults.chat_light_model.is_empty() {
                    caps.defaults.chat_light_model.clone()
                } else if !caps.defaults.chat_default_model.is_empty() {
                    caps.defaults.chat_default_model.clone()
                } else {
                    return Err("no model available for task briefing".to_string());
                };
                let model_rec = crate::caps::resolve_chat_model(caps, &model)?;
                let model_n_ctx = if model_rec.base.n_ctx > 0 {
                    model_rec.base.n_ctx
                } else {
                    16384
                };
                let messages = vec![ChatMessage::new(
                    "user".to_string(),
                    task_briefing_prompt(&role, &source),
                )];
                let config = crate::subchat::SubchatConfig {
                    tool_name: TASK_BRIEFING_CONTEXT_MARKER.to_string(),
                    stateful: false,
                    autonomous_no_confirm: false,
                    chat_id: None,
                    title: None,
                    parent_id: None,
                    link_type: None,
                    root_chat_id: None,
                    tools: crate::subchat::ToolsPolicy::None,
                    max_steps: 1,
                    prepend_system_prompt: false,
                    wrap_up: None,
                    task_meta: None,
                    worktree: None,
                    model,
                    mode: "NO_TOOLS".to_string(),
                    n_ctx: model_n_ctx,
                    max_new_tokens: 1536,
                    temperature: Some(0.0),
                    reasoning_effort: None,
                    parent_tool_call_id: None,
                    parent_subchat_tx: None,
                    abort_flag: None,
                    subchat_depth: 0,
                    buddy_meta: None,
                };
                let result = crate::subchat::run_subchat(app.gcx.clone(), messages, config).await?;
                let briefing = result
                    .messages
                    .iter()
                    .rev()
                    .find(|m| m.role == "assistant")
                    .map(|m| m.content.content_text_only())
                    .ok_or_else(|| {
                        "task briefing subchat produced no assistant message".to_string()
                    })?;
                Ok(crate::llm::safe_truncate(&briefing, MAX_TASK_BRIEFING_SIZE)
                    .trim()
                    .to_string())
            })
        })
        .await
        .map_err(|e| format!("task briefing worker failed: {}", e))?
    }
}

async fn task_briefing_for_plan(
    app: &AppState,
    task_id: &str,
    role: &str,
    plan: &TaskContextInjectionPlan,
) -> Result<Option<String>, String> {
    if plan.entries.is_empty() {
        return Ok(None);
    }
    let content_size = plan
        .entries
        .iter()
        .map(|entry| entry.content.len())
        .sum::<usize>();
    if content_size < MIN_TASK_BRIEFING_SOURCE_SIZE {
        return Ok(None);
    }
    let source = task_context_source_text(&plan.entries);
    let content_hash = task_briefing_cache_hash(&task_context_cache_material(&plan.entries));
    let cache_key = (task_id.to_string(), content_hash.clone());
    if let Some(entry) = task_briefing_cache().lock().await.get(&cache_key).cloned() {
        return Ok(Some(entry.briefing));
    }
    let prompt_source = source;
    let briefing = run_task_briefing_subchat(app, task_id, role, &prompt_source).await?;
    task_briefing_cache().lock().await.insert(
        cache_key,
        TaskBriefingCacheEntry {
            briefing: briefing.clone(),
        },
    );
    Ok(Some(briefing))
}

fn insert_task_context_message(
    messages: &mut Vec<ChatMessage>,
    stream_back_to_user: &mut HasRagResults,
    message: ChatMessage,
) -> usize {
    let insert_pos = messages
        .iter()
        .position(|m| {
            m.role == "user"
                || m.role == "assistant"
                || m.role == crate::chat::internal_roles::EVENT_ROLE
        })
        .unwrap_or(messages.len());
    stream_back_to_user.push_in_json(serde_json::json!(message));
    messages.insert(insert_pos, message);
    insert_pos
}

async fn gather_and_inject_system_context(
    app: &AppState,
    messages: &mut Vec<ChatMessage>,
    stream_back_to_user: &mut HasRagResults,
) -> Result<SkillsTrackingInfo, String> {
    let context = gather_system_context(app.gcx.clone(), false, 4).await?;

    if !context.instruction_files.is_empty() {
        match create_instruction_files_message(&context.instruction_files).await {
            Ok(instr_msg) => {
                let insert_pos = messages
                    .iter()
                    .position(|m| {
                        m.role == "user"
                            || m.role == "assistant"
                            || m.role == crate::chat::internal_roles::EVENT_ROLE
                    })
                    .unwrap_or(messages.len());

                stream_back_to_user.push_in_json(serde_json::json!(instr_msg));
                messages.insert(insert_pos, instr_msg);

                tracing::info!(
                    "Injected {} instruction files at position {}: {:?}",
                    context.instruction_files.len(),
                    insert_pos,
                    context
                        .instruction_files
                        .iter()
                        .map(|f| &f.file_name)
                        .collect::<Vec<_>>()
                );
            }
            Err(e) => {
                tracing::warn!("Failed to create instruction files message: {}", e);
            }
        }
    }

    if !context.memories.is_empty() {
        if let Some(memories_msg) = create_memories_message(&context.memories) {
            let insert_pos = messages
                .iter()
                .position(|m| {
                    m.role == "user"
                        || m.role == "assistant"
                        || m.role == crate::chat::internal_roles::EVENT_ROLE
                })
                .unwrap_or(messages.len());

            stream_back_to_user.push_in_json(serde_json::json!(memories_msg));
            messages.insert(insert_pos, memories_msg);

            tracing::info!(
                "Injected {} memories at position {}",
                context.memories.len(),
                insert_pos
            );
        }
    }

    let have_buddy_pulse = messages
        .iter()
        .any(|m| m.role == "context_file" && m.tool_call_id == BUDDY_PULSE_MARKER);
    if !have_buddy_pulse {
        if let Some(pulse_msg) = app.buddy_event_sink.build_pulse_message().await {
            let insert_pos = messages
                .iter()
                .position(|m| {
                    m.role == "user"
                        || m.role == "assistant"
                        || m.role == crate::chat::internal_roles::EVENT_ROLE
                })
                .unwrap_or(messages.len());
            stream_back_to_user.push_in_json(serde_json::json!(pulse_msg));
            messages.insert(insert_pos, pulse_msg);
            tracing::info!("Injected Buddy pulse at position {}", insert_pos);
        }
    }

    if !context.detected_environments.is_empty() {
        tracing::info!(
            "Detected {} environments: {:?}",
            context.detected_environments.len(),
            context
                .detected_environments
                .iter()
                .map(|e| &e.env_type)
                .collect::<Vec<_>>()
        );
    }

    let have_skills_context = messages
        .iter()
        .any(|m| m.role == "context_file" && m.tool_call_id == SKILLS_CONTEXT_MARKER);
    let skills_tracking = if !have_skills_context {
        let last_user_text = messages
            .iter()
            .rev()
            .find(|m| m.role == "user" || m.role == crate::chat::internal_roles::EVENT_ROLE)
            .and_then(|m| match &m.content {
                crate::call_validation::ChatContent::SimpleText(t) => Some(t.clone()),
                _ => None,
            })
            .unwrap_or_default();
        let (skills_msgs, tracking) =
            build_skills_context_messages_tracked(app.clone(), &last_user_text, None).await;
        for skills_msg in skills_msgs {
            let insert_pos = messages
                .iter()
                .position(|m| {
                    m.role == "user"
                        || m.role == "assistant"
                        || m.role == crate::chat::internal_roles::EVENT_ROLE
                })
                .unwrap_or(messages.len());
            stream_back_to_user.push_in_json(serde_json::json!(skills_msg));
            messages.insert(insert_pos, skills_msg);
        }
        tracking
    } else {
        SkillsTrackingInfo::default()
    };

    Ok(skills_tracking)
}

pub async fn inject_task_memories(
    app: &AppState,
    messages: &mut Vec<ChatMessage>,
    stream_back_to_user: &mut HasRagResults,
    task_id_opt: Option<String>,
    task_meta: Option<&crate::chat::types::TaskMeta>,
) -> Result<(), String> {
    let task_id = match task_id_opt {
        Some(id) => id,
        None => return Ok(()),
    };

    let Some(plan) = build_task_context_injection_plan(app, &task_id, task_meta).await? else {
        return Ok(());
    };

    let role = task_meta
        .map(|meta| meta.role.as_str())
        .unwrap_or("planner");
    let briefing = task_briefing_for_plan(app, &task_id, role, &plan).await;

    match briefing {
        Ok(Some(briefing)) => {
            let task_briefing_msg = ChatMessage {
                role: "context_file".to_string(),
                content: ChatContent::ContextFiles(vec![task_briefing_context_file(briefing)]),
                tool_call_id: TASK_BRIEFING_CONTEXT_MARKER.to_string(),
                ..Default::default()
            };
            let insert_pos =
                insert_task_context_message(messages, stream_back_to_user, task_briefing_msg);
            tracing::info!(
                "Injected task briefing at position {} for task {} from {} memories and {} pinned documents ({} archived/superseded skipped, {} dropped over budget, {} outside scope skipped)",
                insert_pos,
                task_id,
                plan.included_memories,
                plan.pinned_documents,
                plan.archived_skipped,
                plan.dropped_over_budget,
                plan.scope_skipped
            );
        }
        Ok(None) => {
            let task_memories_msg = ChatMessage {
                role: "context_file".to_string(),
                content: ChatContent::ContextFiles(task_context_raw_files(&plan)),
                tool_call_id: TASK_MEMORIES_CONTEXT_MARKER.to_string(),
                ..Default::default()
            };
            let insert_pos =
                insert_task_context_message(messages, stream_back_to_user, task_memories_msg);
            tracing::info!(
                "Injected {} task memories and {} pinned documents at position {} for task {} ({} archived/superseded skipped, {} dropped over budget, {} outside scope skipped)",
                plan.included_memories,
                plan.pinned_documents,
                insert_pos,
                task_id,
                plan.archived_skipped,
                plan.dropped_over_budget,
                plan.scope_skipped
            );
        }
        Err(e) => {
            tracing::warn!(
                "Task briefing failed for task {}, falling back to raw context: {}",
                task_id,
                e
            );
            let task_memories_msg = ChatMessage {
                role: "context_file".to_string(),
                content: ChatContent::ContextFiles(task_context_raw_files(&plan)),
                tool_call_id: TASK_MEMORIES_CONTEXT_MARKER.to_string(),
                ..Default::default()
            };
            let insert_pos =
                insert_task_context_message(messages, stream_back_to_user, task_memories_msg);
            tracing::info!(
                "Injected raw task context fallback at position {} for task {} from {} memories and {} pinned documents",
                insert_pos,
                task_id,
                plan.included_memories,
                plan.pinned_documents
            );
        }
    }

    Ok(())
}
