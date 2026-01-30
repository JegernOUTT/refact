use std::collections::HashSet;
use std::fs;
use std::sync::Arc;
use std::path::PathBuf;
use tokio::sync::RwLock as ARwLock;

use crate::call_validation;
use crate::files_correction::get_project_dirs;
use crate::global_context::GlobalContext;
use crate::http::http_post_json;
use crate::http::routers::v1::system_prompt::{PrependSystemPromptPost, PrependSystemPromptResponse};
use crate::integrations::docker::docker_container_manager::docker_container_get_host_lsp_port_to_connect;
use crate::scratchpads::scratchpad_utils::HasRagResults;
use super::system_context::{
    self, create_instruction_files_message, create_memories_message, gather_system_context,
    generate_git_info_prompt, gather_git_info, PROJECT_CONTEXT_MARKER,
};
use crate::call_validation::{ChatMessage, ChatContent, ChatMode, ContextFile};
use crate::tasks::storage::infer_task_id_from_chat_id;
use crate::tools::tool_task_memory::load_task_memories;
use crate::yaml_configs::customization_registry::{get_mode_config, map_legacy_mode_to_id};

pub async fn get_mode_system_prompt(
    gcx: Arc<ARwLock<GlobalContext>>,
    mode_id: &str,
    model_id: Option<&str>,
) -> String {
    let mode_id = map_legacy_mode_to_id(mode_id);

    match get_mode_config(gcx, mode_id, model_id).await {
        Some(mode_config) => mode_config.prompt,
        None => {
            tracing::warn!("Mode '{}' not found, using empty prompt", mode_id);
            String::new()
        }
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

pub async fn dig_for_project_summarization_file(
    gcx: Arc<ARwLock<GlobalContext>>,
) -> (bool, Option<String>) {
    match crate::files_correction::get_active_project_path(gcx.clone()).await {
        Some(active_project_path) => {
            let summary_path = active_project_path
                .join(".refact")
                .join("project_summary.yaml");
            if !summary_path.exists() {
                (false, Some(summary_path.to_string_lossy().to_string()))
            } else {
                (true, Some(summary_path.to_string_lossy().to_string()))
            }
        }
        None => {
            tracing::info!("No projects found, project summarization is not relevant.");
            (false, None)
        }
    }
}

async fn _read_project_summary(summary_path: String) -> Option<String> {
    match fs::read_to_string(summary_path) {
        Ok(content) => {
            if let Ok(yaml) = serde_yaml::from_str::<serde_yaml::Value>(&content) {
                if let Some(project_summary) = yaml.get("project_summary") {
                    match project_summary {
                        serde_yaml::Value::String(s) => Some(s.clone()),
                        _ => {
                            tracing::error!("'project_summary' is not a string in YAML file.");
                            None
                        }
                    }
                } else {
                    tracing::error!("Key 'project_summary' not found in YAML file.");
                    None
                }
            } else {
                tracing::error!("Failed to parse project summary YAML file.");
                None
            }
        }
        Err(e) => {
            tracing::error!("Failed to read project summary file: {}", e);
            None
        }
    }
}

pub async fn system_prompt_add_extra_instructions(
    gcx: Arc<ARwLock<GlobalContext>>,
    system_prompt: String,
    tool_names: HashSet<String>,
    chat_meta: &call_validation::ChatMeta,
    task_meta: &Option<crate::chat::types::TaskMeta>,
) -> String {
    let include_project_info = chat_meta.include_project_info;
    async fn workspace_files_info(
        gcx: &Arc<ARwLock<GlobalContext>>,
    ) -> (Vec<String>, Option<PathBuf>) {
        let gcx_locked = gcx.read().await;
        let documents_state = &gcx_locked.documents_state;
        let dirs_locked = documents_state.workspace_folders.lock().unwrap();
        let workspace_dirs = dirs_locked
            .clone()
            .into_iter()
            .map(|x| x.to_string_lossy().to_string())
            .collect();
        let active_file_path = documents_state.active_file_path.clone();
        (workspace_dirs, active_file_path)
    }

    let mut system_prompt = system_prompt.clone();

    // New: %SYSTEM_INFO% - OS, datetime, username, architecture
    if system_prompt.contains("%SYSTEM_INFO%") {
        let system_info = system_context::SystemInfo::gather();
        system_prompt = system_prompt.replace("%SYSTEM_INFO%", &system_info.to_prompt_string());
    }

    // New: %ENVIRONMENT_INFO% - Detected environments and usage instructions
    if system_prompt.contains("%ENVIRONMENT_INFO%") {
        if include_project_info {
            let project_dirs = get_project_dirs(gcx.clone()).await;
            let environments = system_context::detect_environments(&project_dirs).await;
            let env_instructions = system_context::generate_environment_instructions(&environments);
            system_prompt = system_prompt.replace("%ENVIRONMENT_INFO%", &env_instructions);
        } else {
            system_prompt = system_prompt.replace("%ENVIRONMENT_INFO%", "");
        }
    }

    // New: %PROJECT_CONFIGS% - Detected project configuration files
    if system_prompt.contains("%PROJECT_CONFIGS%") {
        if include_project_info {
            let project_dirs = get_project_dirs(gcx.clone()).await;
            let configs = system_context::find_project_configs(&project_dirs).await;
            if !configs.is_empty() {
                let config_list = configs
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

    if system_prompt.contains("%PROJECT_TREE%") {
        if include_project_info {
            match system_context::generate_compact_project_tree(gcx.clone(), 4).await {
                Ok(tree) if !tree.is_empty() => {
                    let tree_section = format!("## Project Structure\n```\n{}```", tree);
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

    if system_prompt.contains("%GIT_INFO%") {
        if include_project_info {
            let project_dirs = get_project_dirs(gcx.clone()).await;
            let git_infos = gather_git_info(&project_dirs).await;
            let git_section = generate_git_info_prompt(&git_infos);
            system_prompt = system_prompt.replace("%GIT_INFO%", &git_section);
        } else {
            system_prompt = system_prompt.replace("%GIT_INFO%", "");
        }
    }

    if system_prompt.contains("%WORKSPACE_INFO%") {
        if include_project_info {
            let (workspace_dirs, active_file_path) = workspace_files_info(&gcx).await;
            let info = _workspace_info(&workspace_dirs, &active_file_path).await;
            system_prompt = system_prompt.replace("%WORKSPACE_INFO%", &info);
        } else {
            system_prompt = system_prompt.replace("%WORKSPACE_INFO%", "");
        }
    }

    if system_prompt.contains("%AGENT_WORKTREE%") {
        let worktree_info = if let Some(tm) = task_meta {
            if let Some(ref card_id) = tm.card_id {
                match crate::tasks::storage::load_board(gcx.clone(), &tm.task_id).await {
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
        if include_project_info {
            let cfg = crate::yaml_configs::customization_loader::load_customization_compiled_in();
            let knowledge_instructions = cfg
                .get("KNOWLEDGE_INSTRUCTIONS_META")
                .map(|x| x.as_str().unwrap_or("").to_string())
                .unwrap_or("".to_string());
            system_prompt =
                system_prompt.replace("%KNOWLEDGE_INSTRUCTIONS%", &knowledge_instructions);
        } else {
            system_prompt = system_prompt.replace("%KNOWLEDGE_INSTRUCTIONS%", "");
        }
    }

    if system_prompt.contains("%PROJECT_SUMMARY%") {
        if include_project_info {
            let (exists, summary_path_option) =
                dig_for_project_summarization_file(gcx.clone()).await;
            if exists {
                if let Some(summary_path) = summary_path_option {
                    if let Some(project_info) = _read_project_summary(summary_path).await {
                        system_prompt = system_prompt.replace("%PROJECT_SUMMARY%", &project_info);
                    } else {
                        system_prompt = system_prompt.replace("%PROJECT_SUMMARY%", "");
                    }
                }
            } else {
                system_prompt = system_prompt.replace("%PROJECT_SUMMARY%", "");
            }
        } else {
            system_prompt = system_prompt.replace("%PROJECT_SUMMARY%", "");
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
        let cfg = crate::yaml_configs::customization_loader::load_customization_compiled_in();
        let replacement = cfg
            .get("AGENT_EXPLORATION_INSTRUCTIONS")
            .and_then(|x| x.as_str())
            .unwrap_or("- Call available tools to find relevant files.\n");
        system_prompt = system_prompt.replace("%AGENT_EXPLORATION_INSTRUCTIONS%", replacement);
    }

    if system_prompt.contains("%AGENT_EXECUTION_INSTRUCTIONS%") {
        let has_edit_tools =
            tool_names.contains("create_textdoc") || tool_names.contains("update_textdoc");
        let replacement = if has_edit_tools {
            let cfg = crate::yaml_configs::customization_loader::load_customization_compiled_in();
            cfg.get("AGENT_EXECUTION_INSTRUCTIONS")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string()
        } else {
            "  - Propose the changes to the user
    - the suspected root cause
    - the exact files/functions to modify or create
    - the new or updated tests to add
    - the expected outcome and success criteria
"
            .to_string()
        };
        system_prompt = system_prompt.replace("%AGENT_EXECUTION_INSTRUCTIONS%", &replacement);
    }

    if system_prompt.contains("%CD_INSTRUCTIONS%") {
        let cfg = crate::yaml_configs::customization_loader::load_customization_compiled_in();
        let replacement = cfg
            .get("CD_INSTRUCTIONS")
            .and_then(|x| x.as_str())
            .unwrap_or("You might receive additional instructions that start with 💿. Those are not coming from the user, they are programmed to help you operate well and they are always in English. Answer in the language the user has asked the question.");
        system_prompt = system_prompt.replace("%CD_INSTRUCTIONS%", replacement);
    }

    if system_prompt.contains("%SHELL_INSTRUCTIONS%") {
        let cfg = crate::yaml_configs::customization_loader::load_customization_compiled_in();
        let replacement = cfg
            .get("SHELL_INSTRUCTIONS")
            .and_then(|x| x.as_str())
            .unwrap_or("");
        system_prompt = system_prompt.replace("%SHELL_INSTRUCTIONS%", replacement);
    }

    system_prompt
}

pub async fn prepend_the_right_system_prompt_and_maybe_more_initial_messages(
    gcx: Arc<ARwLock<GlobalContext>>,
    mut messages: Vec<call_validation::ChatMessage>,
    chat_meta: &call_validation::ChatMeta,
    task_meta: &Option<crate::chat::types::TaskMeta>,
    stream_back_to_user: &mut HasRagResults,
    tool_names: HashSet<String>,
    mode_id: &str,
    model_id: &str,
) -> Vec<call_validation::ChatMessage> {
    if messages.is_empty() {
        tracing::error!("What's that? Messages list is empty");
        return messages;
    }

    let have_system = messages
        .first()
        .map(|m| m.role == "system")
        .unwrap_or(false);
    let have_project_context = messages
        .iter()
        .any(|m| m.role == "context_file" && m.tool_call_id == PROJECT_CONTEXT_MARKER);

    let is_inside_container = gcx.read().await.cmdline.inside_container;
    if chat_meta.chat_remote && !is_inside_container {
        messages = match prepend_system_prompt_and_maybe_more_initial_messages_from_remote(
            gcx.clone(),
            &messages,
            chat_meta,
            stream_back_to_user,
        )
        .await
        {
            Ok(messages_from_remote) => messages_from_remote,
            Err(e) => {
                tracing::error!("prepend_the_right_system_prompt_and_maybe_more_initial_messages_from_remote: {}", e);
                messages
            }
        };
        return messages;
    }

    if !have_system {
        match chat_meta.chat_mode {
            ChatMode::EXPLORE
            | ChatMode::AGENT
            | ChatMode::NO_TOOLS
            | ChatMode::TASK_PLANNER
            | ChatMode::TASK_AGENT => {
                let base_prompt = get_mode_system_prompt(gcx.clone(), mode_id, Some(model_id)).await;
                let system_message_content = system_prompt_add_extra_instructions(
                    gcx.clone(),
                    base_prompt,
                    tool_names,
                    chat_meta,
                    task_meta,
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
            ChatMode::CONFIGURE => {
                crate::integrations::config_chat::mix_config_messages(
                    gcx.clone(),
                    &chat_meta,
                    &mut messages,
                    stream_back_to_user,
                )
                .await;
            }
            ChatMode::PROJECT_SUMMARY => {
                crate::integrations::project_summary_chat::mix_project_summary_messages(
                    gcx.clone(),
                    &chat_meta,
                    &mut messages,
                    stream_back_to_user,
                )
                .await;
            }
        }
    }

    if chat_meta.include_project_info && !have_project_context {
        match gather_and_inject_system_context(&gcx, &mut messages, stream_back_to_user).await {
            Ok(()) => {}
            Err(e) => {
                tracing::warn!("Failed to gather system context: {}", e);
            }
        }
    } else if !chat_meta.include_project_info {
        tracing::info!("Skipping project/system context injection (include_project_info=false)");
    }

    if matches!(
        chat_meta.chat_mode,
        ChatMode::TASK_PLANNER | ChatMode::TASK_AGENT
    ) {
        match inject_task_memories(&gcx, &mut messages, stream_back_to_user, &chat_meta.chat_id)
            .await
        {
            Ok(()) => {}
            Err(e) => {
                tracing::warn!("Failed to inject task memories: {}", e);
            }
        }
    }

    tracing::info!(
        "\n\nSYSTEM PROMPT MIXER chat_mode={:?}\n{:#?}",
        chat_meta.chat_mode,
        messages
    );
    messages
}

const TASK_MEMORIES_CONTEXT_MARKER: &str = "task_memories_context";
const MAX_TASK_MEMORY_CONTENT_SIZE: usize = 3000;
const MAX_TASK_MEMORIES_TOTAL_SIZE: usize = 80_000;

async fn gather_and_inject_system_context(
    gcx: &Arc<ARwLock<GlobalContext>>,
    messages: &mut Vec<ChatMessage>,
    stream_back_to_user: &mut HasRagResults,
) -> Result<(), String> {
    let context = gather_system_context(gcx.clone(), false, 4).await?;

    if !context.instruction_files.is_empty() {
        match create_instruction_files_message(&context.instruction_files).await {
            Ok(instr_msg) => {
                let insert_pos = messages
                    .iter()
                    .position(|m| m.role == "user" || m.role == "assistant")
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
                .position(|m| m.role == "user" || m.role == "assistant")
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

    Ok(())
}

pub async fn inject_task_memories(
    gcx: &Arc<ARwLock<GlobalContext>>,
    messages: &mut Vec<ChatMessage>,
    stream_back_to_user: &mut HasRagResults,
    chat_id: &str,
) -> Result<(), String> {
    let task_id = match infer_task_id_from_chat_id(chat_id) {
        Some(id) => id,
        None => return Ok(()),
    };

    let memories = load_task_memories(gcx.clone(), &task_id).await?;
    if memories.is_empty() {
        return Ok(());
    }

    let mut context_files: Vec<ContextFile> = Vec::new();
    let mut total_size = 0;
    let mut included_count = 0;
    let mut skipped_count = 0;

    for (path, content) in &memories {
        if total_size >= MAX_TASK_MEMORIES_TOTAL_SIZE {
            skipped_count += 1;
            continue;
        }

        let truncated_content = if content.len() > MAX_TASK_MEMORY_CONTENT_SIZE {
            format!(
                "{}\n\n[TRUNCATED]",
                content
                    .chars()
                    .take(MAX_TASK_MEMORY_CONTENT_SIZE)
                    .collect::<String>()
            )
        } else {
            content.clone()
        };

        let line_count = truncated_content.lines().count().max(1);
        total_size += truncated_content.len();
        included_count += 1;

        context_files.push(ContextFile {
            file_name: path.to_string_lossy().to_string(),
            file_content: truncated_content,
            line1: 1,
            line2: line_count,
            file_rev: None,
            symbols: vec![],
            gradient_type: -1,
            usefulness: 95.0,
            skip_pp: true,
        });
    }

    if context_files.is_empty() {
        return Ok(());
    }

    if skipped_count > 0 {
        context_files.push(ContextFile {
            file_name: "(task memories summary)".to_string(),
            file_content: format!(
                "Note: {} task memories included, {} omitted due to size limits. Use task_memories_get() to retrieve all.",
                included_count,
                skipped_count
            ),
            line1: 1,
            line2: 1,
            file_rev: None,
            symbols: vec![],
            gradient_type: -1,
            usefulness: 50.0,
            skip_pp: true,
        });
    }

    let task_memories_msg = ChatMessage {
        role: "context_file".to_string(),
        content: ChatContent::ContextFiles(context_files),
        tool_call_id: TASK_MEMORIES_CONTEXT_MARKER.to_string(),
        ..Default::default()
    };

    let insert_pos = messages
        .iter()
        .position(|m| m.role == "user" || m.role == "assistant")
        .unwrap_or(messages.len());

    stream_back_to_user.push_in_json(serde_json::json!(task_memories_msg));
    messages.insert(insert_pos, task_memories_msg);

    tracing::info!(
        "Injected {} task memories at position {} for task {} ({} skipped)",
        included_count,
        insert_pos,
        task_id,
        skipped_count
    );

    Ok(())
}

pub async fn prepend_system_prompt_and_maybe_more_initial_messages_from_remote(
    gcx: Arc<ARwLock<GlobalContext>>,
    messages: &[call_validation::ChatMessage],
    chat_meta: &call_validation::ChatMeta,
    stream_back_to_user: &mut HasRagResults,
) -> Result<Vec<call_validation::ChatMessage>, String> {
    let post = PrependSystemPromptPost {
        messages: messages.to_vec(),
        chat_meta: chat_meta.clone(),
    };

    let port =
        docker_container_get_host_lsp_port_to_connect(gcx.clone(), &chat_meta.chat_id).await?;
    let url =
        format!("http://localhost:{port}/v1/prepend-system-prompt-and-maybe-more-initial-messages");
    let response: PrependSystemPromptResponse = http_post_json(&url, &post).await?;

    for msg in response.messages_to_stream_back {
        stream_back_to_user.push_in_json(msg);
    }

    Ok(response.messages)
}
