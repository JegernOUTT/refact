use std::sync::Arc;
use axum::Extension;
use axum::http::{Response, StatusCode};
use hyper::Body;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock as ARwLock;

use crate::call_validation::{ChatMessage, ChatMeta, is_agentic_mode_id, validate_mode_for_request};
use crate::custom_error::ScratchError;
use crate::caps::resolve_chat_model;
use crate::global_context::{try_load_caps_quickly_if_not_present, GlobalContext};
use crate::integrations::go_to_configuration_message;
use crate::agentic::generate_follow_up_message::generate_follow_up_message;
use crate::git::commit_info::{get_commit_information_from_current_changes, generate_commit_messages};
// use crate::http::routers::v1::git::GitCommitPost;

#[derive(Deserialize, Clone, Debug)]
pub struct LinksPost {
    messages: Vec<ChatMessage>,
    model_name: String,
    meta: ChatMeta,
}

#[derive(Default, Serialize, Deserialize, Debug)]
#[serde(rename_all = "kebab-case")]
enum LinkAction {
    #[default]
    PatchAll,
    FollowUp,
    Commit,
    GitInit,
    Goto,
    PostChat,
    RegenerateWithIncreasedContextSize,
}

#[derive(Default, Serialize, Debug)]
pub struct Link {
    link_action: LinkAction,
    link_text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    link_goto: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    link_summary_path: Option<String>,
    link_tooltip: String,
    #[serde(default, skip_serializing_if = "is_default_json_value")]
    link_payload: serde_json::Value,
}

fn is_default_json_value(value: &serde_json::Value) -> bool {
    value == &serde_json::Value::Null
}

fn last_message_assistant_without_tools_with_code_blocks(messages: &[ChatMessage]) -> bool {
    if let Some(m) = messages.last() {
        m.role == "assistant"
            && m.tool_calls.as_ref().map(|x| x.is_empty()).unwrap_or(true)
            && (m.content.content_text_only().contains("```")
                || m.content.content_text_only().contains("|"))
    } else {
        false
    }
}

fn last_message_stripped_assistant(messages: &[ChatMessage]) -> bool {
    if let Some(m) = messages.last() {
        m.role == "assistant" && m.finish_reason == Some("length".to_string())
    } else {
        false
    }
}

pub async fn handle_v1_links(
    Extension(gcx): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<LinksPost>(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;
    let mut links: Vec<Link> = Vec::new();
    let mut uncommited_changes_warning = String::new();

    let canonical_mode = validate_mode_for_request(gcx.clone(), &post.meta.chat_mode)
        .await
        .map_err(|e| {
            ScratchError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("Invalid chat mode: {}", e),
            )
        })?;

    tracing::info!("for links, canonical_mode == {:?}", canonical_mode);

    let integration_yaml_errors = if is_agentic_mode_id(&canonical_mode) {
        let (_integrations_map, integration_yaml_errors) =
            crate::integrations::running_integrations::load_integrations(
                gcx.clone(),
                &["**/*".to_string()],
            )
            .await;
        integration_yaml_errors
    } else {
        Vec::new()
    };
    if canonical_mode == "configurator" {
        if last_message_assistant_without_tools_with_code_blocks(&post.messages) {
            links.push(Link {
                link_action: LinkAction::FollowUp,
                link_text: "Looks alright! Save the generated config".to_string(),
                link_goto: None,
                link_summary_path: None,
                link_tooltip: format!(""),
                ..Default::default()
            });
        }
    }

    if canonical_mode == "setup" {
        if last_message_assistant_without_tools_with_code_blocks(&post.messages) {
            links.push(Link {
                link_action: LinkAction::FollowUp,
                link_text: "Looks alright! Save the generated setup artifacts".to_string(),
                link_goto: None,
                link_summary_path: None,
                link_tooltip: format!(""),
                ..Default::default()
            });
        }
    }

    // GIT uncommitted
    if is_agentic_mode_id(&canonical_mode) && post.messages.is_empty() {
        let commits_info = get_commit_information_from_current_changes(gcx.clone()).await;

        let mut commit_texts = Vec::new();
        for commit_info in &commits_info {
            let mut commit_text = format!("In project {}:\n", commit_info.get_project_name());

            if !commit_info.staged_changes.is_empty() {
                commit_text.push_str("Staged changes:\n");
                commit_text.push_str(
                    &commit_info
                        .staged_changes
                        .iter()
                        .take(2)
                        .map(|f| {
                            format!(
                                "{} {}",
                                f.status.initial(),
                                f.relative_path.to_string_lossy()
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
                commit_text.push('\n');
                if commit_info.staged_changes.len() > 2 {
                    commit_text.push_str(&format!(
                        "...{} files more\n",
                        commit_info.staged_changes.len() - 2
                    ));
                }
            }

            if !commit_info.unstaged_changes.is_empty() {
                commit_text.push_str("Unstaged changes:\n");
                commit_text.push_str(
                    &commit_info
                        .unstaged_changes
                        .iter()
                        .take(2)
                        .map(|f| {
                            format!(
                                "{} {}",
                                f.status.initial(),
                                f.relative_path.to_string_lossy()
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                );
                commit_text.push('\n');
                if commit_info.unstaged_changes.len() > 2 {
                    commit_text.push_str(&format!(
                        "...{} files more\n",
                        commit_info.unstaged_changes.len() - 2
                    ));
                }
            }

            commit_texts.push(commit_text);
        }
        if !commit_texts.is_empty() {
            if commit_texts.len() > 4 {
                let omitted_projects = commit_texts.len() - 3;
                commit_texts.truncate(3);
                commit_texts.push(format!("...{} projects more", omitted_projects));
            }
            uncommited_changes_warning = format!("You have uncommitted changes:\n```\n{}\n```\nIt's fine, but you might have a problem rolling back agent's changes.", commit_texts.join("\n"));
        }

        if false {
            for commit_with_msg in generate_commit_messages(gcx.clone(), commits_info).await {
                let all_changes = commit_with_msg
                    .staged_changes
                    .iter()
                    .chain(commit_with_msg.unstaged_changes.iter());
                let first_changes = all_changes
                    .clone()
                    .take(5)
                    .map(|f| {
                        format!(
                            "{} {}",
                            f.status.initial(),
                            f.relative_path.to_string_lossy()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let remaining_changes = all_changes.count().saturating_sub(5);

                let mut tooltip_message = format!(
                    "git commit -m \"{}{}\"\n{}",
                    commit_with_msg.commit_message.lines().next().unwrap_or(""),
                    if commit_with_msg.commit_message.lines().count() > 1 {
                        "..."
                    } else {
                        ""
                    },
                    first_changes,
                );
                if remaining_changes != 0 {
                    tooltip_message.push_str(&format!("\n...{} files more", remaining_changes));
                }
                links.push(Link {
                    link_action: LinkAction::Commit,
                    link_text: format!(
                        "Commit {} files in `{}`",
                        commit_with_msg.staged_changes.len()
                            + commit_with_msg.unstaged_changes.len(),
                        commit_with_msg.get_project_name()
                    ),
                    link_goto: Some("LINKS_AGAIN".to_string()),
                    link_summary_path: None,
                    link_tooltip: tooltip_message,
                    link_payload: serde_json::json!({ "commits": [commit_with_msg] }),
                });
            }
        }
    }

    if is_agentic_mode_id(&canonical_mode) {
        for failed_integr_name in failed_integration_names_after_last_user_message(&post.messages) {
            links.push(Link {
                link_action: LinkAction::Goto,
                link_text: format!("Configure {failed_integr_name}"),
                link_goto: Some(format!("SETTINGS:{failed_integr_name}")),
                link_summary_path: None,
                link_tooltip: format!(""),
                ..Default::default()
            })
        }

        // YAML problems
        for e in integration_yaml_errors {
            links.push(Link {
                link_action: LinkAction::Goto,
                link_text: format!(
                    "Syntax error in {}",
                    crate::nicer_logs::last_n_chars(&e.path, 20)
                ),
                link_goto: Some(format!("SETTINGS:{}", e.path)),
                link_summary_path: None,
                link_tooltip: format!("Error at line {}: {}", e.error_line, e.error_msg),
                ..Default::default()
            });
        }
    }

    // RegenerateWithIncreasedContextSize
    if last_message_stripped_assistant(&post.messages) {
        links.push(Link {
            link_action: LinkAction::RegenerateWithIncreasedContextSize,
            link_text: format!("Increase tokens limit and regenerate last message"),
            link_goto: None,
            link_summary_path: None,
            link_tooltip: format!(""),
            ..Default::default()
        });
        links.push(Link {
            link_action: LinkAction::FollowUp,
            link_text: "Complete the previous message from where it was left off".to_string(),
            link_goto: None,
            link_summary_path: None,
            link_tooltip: format!(""),
            ..Default::default()
        });
    }

    // Tool recommendations
    /* temporary remove project summary and recomended integrations    */

    // Follow-up
    let mut new_chat_suggestion = false;
    if canonical_mode != "explore"
        && links.is_empty()
        && post.messages.len() > 2
        && post
            .messages
            .last()
            .map(|x| x.role == "assistant")
            .unwrap_or(false)
    {
        let caps = try_load_caps_quickly_if_not_present(gcx.clone(), 0).await?;
        let model_id = match resolve_chat_model(caps.clone(), &caps.defaults.chat_light_model) {
            Ok(light_model) => light_model.base.id.clone(),
            Err(_) => post.model_name.clone(),
        };
        let follow_up_response = generate_follow_up_message(
            post.messages.clone(),
            gcx.clone(),
            &model_id,
            &post.meta.chat_id,
        )
        .await
        .map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Error generating follow-up message: {}", e),
            )
        })?;
        new_chat_suggestion = follow_up_response.topic_changed;
        for follow_up_message in follow_up_response.follow_ups {
            tracing::info!("follow-up {:?}", follow_up_message);
            links.push(Link {
                link_action: LinkAction::FollowUp,
                link_text: follow_up_message,
                link_goto: None,
                link_summary_path: None,
                link_tooltip: format!(""),
                ..Default::default()
            });
        }
    }

    match serde_json::to_string_pretty(&links) {
        Ok(serialized) => tracing::info!("generated links2\n{}", serialized),
        Err(err) => tracing::warn!("failed to serialize links for logging: {}", err),
    }

    let payload = serde_json::to_string_pretty(&serde_json::json!({
        "links": links,
        "uncommited_changes_warning": uncommited_changes_warning,
        "new_chat_suggestion": new_chat_suggestion
    }))
    .map_err(|e| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("response json error: {}", e),
        )
    })?;

    let response = Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(payload))
        .map_err(|e| {
            ScratchError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("response build error: {}", e),
            )
        })?;
    Ok(response)
}

fn failed_integration_names_after_last_user_message(messages: &[ChatMessage]) -> Vec<String> {
    let last_user_msg_index = messages.iter().rposition(|m| m.role == "user").unwrap_or(0);
    let tool_calls = messages[last_user_msg_index..]
        .iter()
        .filter(|m| m.role == "assistant")
        .filter_map(|m| m.tool_calls.as_ref())
        .flatten()
        .collect::<Vec<_>>();

    let mut result = Vec::new();
    for tool_call in tool_calls {
        if let Some(answer_text) = messages
            .iter()
            .find(|m| m.role == "tool" && m.tool_call_id == tool_call.id)
            .map(|m| m.content.content_text_only())
        {
            if answer_text.contains(&go_to_configuration_message(&tool_call.function.name)) {
                result.push(tool_call.function.name.clone());
            }
        }
    }
    result.sort();
    result.dedup();
    result
}
