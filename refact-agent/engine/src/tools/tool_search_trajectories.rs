use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::chat::trajectory_search::{
    query_terms, search_saved_trajectories, RefreshStats, SearchLimits, TrajectoryHit,
};
use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, json_schema_from_params,
};

pub struct ToolSearchTrajectories {
    pub config_path: String,
}

pub const NO_MATCHING_TRAJECTORIES: &str = "No matching trajectories found.";

fn hist_search_preview_notice(shown: usize, total: usize, limit: usize) -> String {
    format!(
        "\n⚠️ showing {} of {} preview characters (limit: hist_search_preview_chars = {}). 💡 Raise hist_search_preview_chars in trajectory settings or call get_trajectory_context() to expand.",
        shown, total, limit
    )
}

fn coverage_notes(stats: &RefreshStats) -> String {
    let mut notes = String::new();
    if stats.pending > 0 {
        notes.push_str(&format!(
            "\n⚠️ {} changed trajectories were not indexed yet (per-search read limit); repeat the search to include them.",
            stats.pending
        ));
    }
    if stats.over_cap > 0 {
        notes.push_str(&format!(
            "\n⚠️ {} older trajectories are beyond the search cap and were not searched.",
            stats.over_cap
        ));
    }
    notes
}

pub fn format_search_output(
    hits: &[TrajectoryHit],
    stats: &RefreshStats,
    preview_chars: usize,
) -> String {
    let Some(top_score) = hits.first().map(|hit| hit.score) else {
        return format!("{}{}", NO_MATCHING_TRAJECTORIES, coverage_notes(stats));
    };
    let mut result = format!("Found {} relevant trajectories:\n\n", hits.len());
    for hit in hits {
        result.push_str("───────────────────────────────────────\n");
        result.push_str(&format!("📁 {}\n", hit.trajectory_id));
        result.push_str(&format!("📌 {}\n", hit.title));
        result.push_str(&format!(
            "⭐ Relevance: {:.0}%\n",
            hit.score / top_score * 100.0
        ));
        result.push_str(&format!(
            "📍 Messages: {}-{}\n",
            hit.message_start, hit.message_end
        ));
        result.push('\n');
        let total_chars = hit.snippet.chars().count();
        let preview: String = hit.snippet.chars().take(preview_chars).collect();
        result.push_str(&preview);
        if total_chars > preview_chars {
            result.push_str(&hist_search_preview_notice(
                preview_chars,
                total_chars,
                preview_chars,
            ));
        }
        result.push_str("\n\n");
    }
    result.push_str("───────────────────────────────────────\n");
    result.push_str(
        "💡 Use get_trajectory_context(trajectory_id, message_start, message_end) to expand.\n",
    );
    result.push_str("\nNote: these are keyword matches and may be unrelated.");
    result.push_str(&coverage_notes(stats));
    result
}

#[async_trait]
impl Tool for ToolSearchTrajectories {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "search_trajectories".to_string(),
            display_name: "Search Trajectories".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Keyword search over saved past chat trajectories (exact words, not semantic meaning). Returns matching trajectory IDs with message ranges that can be expanded using get_trajectory_context.".to_string(),
            input_schema: json_schema_from_params(&[("query", "string", "Keywords to find in past conversations."), ("top_n", "string", "Maximum number of trajectories to return (default: 5).")], &["query"]),
            output_schema: None,
            annotations: None,
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let gcx = ccx.lock().await.app.gcx.clone();

        let query = match args.get("query") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => return Err(format!("argument `query` is not a string: {:?}", v)),
            None => return Err("argument `query` is missing".to_string()),
        };

        let top_n: usize = match args.get("top_n") {
            Some(Value::String(s)) => s.parse().unwrap_or(5),
            Some(Value::Number(n)) => n.as_u64().unwrap_or(5) as usize,
            _ => 5,
        };

        let terms = query_terms(&query);
        let dirs = crate::chat::trajectories::get_all_trajectories_dirs(gcx).await;
        let (hits, stats) =
            search_saved_trajectories(dirs, terms, top_n, SearchLimits::default()).await?;
        let output = format_search_output(
            &hits,
            &stats,
            crate::runtime_settings::current().hist_search_preview_chars,
        );

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(output),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            })],
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[cfg(test)]
#[path = "tool_search_trajectories_tests.rs"]
mod tests;
