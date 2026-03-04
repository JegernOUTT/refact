use std::sync::Arc;
use chrono::{DateTime, Utc, Duration};
use serde_json::Value;
use tokio::sync::RwLock as ARwLock;
use tokio::fs;
use tracing::{info, warn};
use walkdir::WalkDir;

use crate::call_validation::{ChatContent, ChatMessage};
use crate::chat::trajectories::extract_text_with_image_placeholders_from_json;
use crate::files_correction::get_project_dirs;
use crate::global_context::GlobalContext;
use crate::memories::{memories_add, create_frontmatter};
use crate::memories::extract_file_paths;
use crate::subchat::run_subchat_once;
use crate::yaml_configs::customization_registry::get_subagent_config;

const ABANDONED_THRESHOLD_HOURS: i64 = 2;
const CHECK_INTERVAL_SECS: u64 = 300;
const TRAJECTORIES_FOLDER: &str = ".refact/trajectories";
const SUBAGENT_ID: &str = "memo_extraction";

pub async fn trajectory_memos_background_task(gcx: Arc<ARwLock<GlobalContext>>) {
    loop {
        let shutdown_flag = gcx.read().await.shutdown_flag.clone();
        tokio::select! {
            _ = tokio::time::sleep(tokio::time::Duration::from_secs(CHECK_INTERVAL_SECS)) => {}
            _ = async {
                while !shutdown_flag.load(std::sync::atomic::Ordering::SeqCst) {
                    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
                }
            } => {
                tracing::info!("Trajectory memos: shutdown detected, stopping");
                return;
            }
        }

        if let Err(e) = process_abandoned_trajectories(gcx.clone()).await {
            warn!("trajectory_memos: error processing trajectories: {}", e);
        }
    }
}

async fn process_abandoned_trajectories(gcx: Arc<ARwLock<GlobalContext>>) -> Result<(), String> {
    let project_dirs = get_project_dirs(gcx.clone()).await;
    if project_dirs.is_empty() {
        return Ok(());
    }

    let now = Utc::now();
    let threshold = now - Duration::hours(ABANDONED_THRESHOLD_HOURS);

    for workspace_root in project_dirs {
        let trajectories_dir = workspace_root.join(TRAJECTORIES_FOLDER);
        if !trajectories_dir.exists() {
            continue;
        }

        for entry in WalkDir::new(&trajectories_dir)
            .max_depth(1)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if !path.is_file() || path.extension().map(|e| e != "json").unwrap_or(true) {
                continue;
            }

            match process_single_trajectory(gcx.clone(), path.to_path_buf(), &threshold).await {
                Ok(true) => info!("trajectory_memos: extracted memos from {}", path.display()),
                Ok(false) => {}
                Err(e) => warn!(
                    "trajectory_memos: failed to process {}: {}",
                    path.display(),
                    e
                ),
            }
        }
    }

    Ok(())
}

async fn process_single_trajectory(
    gcx: Arc<ARwLock<GlobalContext>>,
    path: std::path::PathBuf,
    threshold: &DateTime<Utc>,
) -> Result<bool, String> {
    let content = fs::read_to_string(&path).await.map_err(|e| e.to_string())?;
    let mut trajectory: Value = serde_json::from_str(&content).map_err(|e| e.to_string())?;

    if trajectory
        .get("memo_extracted")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return Ok(false);
    }

    let updated_at = trajectory
        .get("updated_at")
        .and_then(|v| v.as_str())
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc));

    let is_abandoned = match updated_at {
        Some(dt) => dt < *threshold,
        None => false,
    };

    if !is_abandoned {
        return Ok(false);
    }

    let messages = trajectory
        .get("messages")
        .and_then(|v| v.as_array())
        .ok_or("No messages")?;

    if messages.len() < 10 {
        return Ok(false);
    }

    let trajectory_id = trajectory
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    let root_chat_id = trajectory
        .get("root_chat_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| trajectory_id.clone());
    let current_title = trajectory
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("Untitled")
        .to_string();

    let is_title_generated = trajectory
        .get("extra")
        .and_then(|e| e.get("isTitleGenerated"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let chat_messages = build_chat_messages(messages);

    let extraction = extract_memos_and_meta(
        gcx.clone(),
        chat_messages,
        &current_title,
        is_title_generated,
    )
    .await?;

    let traj_obj = trajectory.as_object_mut().ok_or("Invalid trajectory")?;

    if let Some(ref meta) = extraction.meta {
        traj_obj.insert("overview".to_string(), Value::String(meta.overview.clone()));
        if is_title_generated && !meta.title.is_empty() {
            traj_obj.insert("title".to_string(), Value::String(meta.title.clone()));
            info!(
                "trajectory_memos: updated title '{}' -> '{}' for {}",
                current_title, meta.title, trajectory_id
            );
        }
    }

    let memo_title = extraction
        .meta
        .as_ref()
        .filter(|_| is_title_generated)
        .map(|m| m.title.clone())
        .unwrap_or(current_title);

    for memo in extraction.memos {
        let frontmatter = create_frontmatter(
            Some(&format!("[{}] {}", memo.memo_type, memo_title)),
            &[memo.memo_type.clone(), "trajectory".to_string()],
            &[],
            &[],
            "memory",
        );

        let mut frontmatter = frontmatter;
        frontmatter.source_chat_id = Some(root_chat_id.clone());
        frontmatter.source_tool = Some("memo_extraction".to_string());
        frontmatter.source_trajectory_id = Some(trajectory_id.clone());
        frontmatter.summary = Some(memo.content.lines().take(3).collect::<Vec<_>>().join(" "));
        frontmatter.description = memo
            .content
            .lines()
            .skip(1)
            .find(|l| !l.trim().is_empty())
            .map(|l| l.trim().to_string());
        frontmatter.related_files = extract_file_paths(&memo.content);

        let content_with_source = format!(
            "{}\n\n---\nSource: trajectory `{}`",
            memo.content, trajectory_id
        );

        if let Err(e) = memories_add(gcx.clone(), &frontmatter, &content_with_source).await {
            warn!("trajectory_memos: failed to save memo: {}", e);
        }
    }

    traj_obj.insert("memo_extracted".to_string(), Value::Bool(true));

    let tmp_path = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(&trajectory).map_err(|e| e.to_string())?;
    fs::write(&tmp_path, &json)
        .await
        .map_err(|e| e.to_string())?;
    fs::rename(&tmp_path, &path)
        .await
        .map_err(|e| e.to_string())?;

    Ok(true)
}

fn build_chat_messages(messages: &[Value]) -> Vec<ChatMessage> {
    let msgs: Vec<ChatMessage> = messages
        .iter()
        .filter_map(|msg| {
            let role = msg.get("role").and_then(|v| v.as_str())?;
            if role != "user" && role != "assistant" {
                return None;
            }

            let content = msg.get("content")
                .and_then(extract_text_with_image_placeholders_from_json)?;

            if content.trim().is_empty() {
                return None;
            }

            Some(ChatMessage {
                role: role.to_string(),
                content: ChatContent::SimpleText(content.chars().take(3000).collect()),
                ..Default::default()
            })
        })
        .collect();

    // Drop leading assistant messages — validate_chat_history requires the first message
    // to be 'user' or 'system'. This can happen when a subchat trajectory starts with a
    // system message (filtered above) followed by an assistant message.
    let start = msgs.iter().position(|m| m.role == "user").unwrap_or(msgs.len());
    msgs[start..].to_vec()
}

struct ExtractedMemo {
    memo_type: String,
    content: String,
}

struct TrajectoryMeta {
    overview: String,
    title: String,
}

struct ExtractionResult {
    meta: Option<TrajectoryMeta>,
    memos: Vec<ExtractedMemo>,
}

async fn extract_memos_and_meta(
    gcx: Arc<ARwLock<GlobalContext>>,
    mut messages: Vec<ChatMessage>,
    current_title: &str,
    is_title_generated: bool,
) -> Result<ExtractionResult, String> {
    let subagent_config = get_subagent_config(gcx.clone(), SUBAGENT_ID, None)
        .await
        .ok_or_else(|| format!("subagent config '{}' not found", SUBAGENT_ID))?;

    let extraction_prompt = subagent_config.messages.user_template
        .as_ref()
        .ok_or_else(|| format!("messages.user_template not defined for subagent '{}'", SUBAGENT_ID))?;

    let title_hint = if is_title_generated {
        format!("\n\nNote: The current title \"{}\" was auto-generated. Please provide a better descriptive title.", current_title)
    } else {
        String::new()
    };

    messages.push(ChatMessage {
        role: "user".to_string(),
        content: ChatContent::SimpleText(format!("{}{}", extraction_prompt, title_hint)),
        ..Default::default()
    });

    let result = run_subchat_once(gcx, SUBAGENT_ID, messages)
        .await
        .map_err(|e| e.to_string())?;

    let response_text = result
        .messages
        .last()
        .and_then(|m| match &m.content {
            ChatContent::SimpleText(t) => Some(t.clone()),
            _ => None,
        })
        .unwrap_or_default();

    let mut meta: Option<TrajectoryMeta> = None;
    let mut memos: Vec<ExtractedMemo> = Vec::new();

    for line in response_text.lines() {
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }

        let parsed: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if let (Some(overview), Some(title)) = (
            parsed.get("overview").and_then(|v| v.as_str()),
            parsed.get("title").and_then(|v| v.as_str()),
        ) {
            meta = Some(TrajectoryMeta {
                overview: overview.to_string(),
                title: title.to_string(),
            });
            continue;
        }

        if let (Some(memo_type), Some(content)) = (
            parsed.get("type").and_then(|v| v.as_str()),
            parsed.get("content").and_then(|v| v.as_str()),
        ) {
            if memos.len() < 10 {
                memos.push(ExtractedMemo {
                    memo_type: memo_type.to_string(),
                    content: content.to_string(),
                });
            }
        }
    }

    Ok(ExtractionResult { meta, memos })
}
