use std::sync::Arc;
use serde::{Deserialize, Serialize};

use crate::call_validation::ChatMessage;
use crate::global_context::GlobalContext;
use crate::subchat::run_subchat_once;
use crate::yaml_configs::customization_registry::get_subagent_config;

use refact_knowledge_graph::kg_structs::KnowledgeDoc;

const KG_ENRICH_SUBAGENT_ID: &str = "kg_enrich";
const KG_DEPRECATE_SUBAGENT_ID: &str = "kg_deprecate";

#[derive(Debug, Serialize, Deserialize)]
pub struct EnrichmentResult {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub filenames: Vec<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub links: Vec<String>,
    #[serde(default)]
    pub review_after_days: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeprecationDecision {
    #[serde(default)]
    pub target_id: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub confidence: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeprecationResult {
    #[serde(default)]
    pub deprecate: Vec<DeprecationDecision>,
    #[serde(default)]
    pub keep: Vec<String>,
}

pub async fn enrich_knowledge_metadata(
    gcx: Arc<GlobalContext>,
    content: &str,
    entities: &[String],
    candidate_files: &[String],
    candidate_docs: &[(String, String)],
) -> Result<EnrichmentResult, String> {
    let content = content.to_string();
    let entities = entities.to_vec();
    let candidate_files = candidate_files.to_vec();
    let candidate_docs = candidate_docs.to_vec();
    let gcx2 = gcx.clone();
    crate::buddy::workflows::buddy_wrap_workflow(
        crate::app_state::AppState::from_gcx(gcx).await,
        "kg_enrich",
        "📚",
        12,
        |_: &EnrichmentResult| "Knowledge updated".to_string(),
        move || async move {
            let subagent_config = get_subagent_config(gcx2.clone(), KG_ENRICH_SUBAGENT_ID, None)
                .await
                .ok_or_else(|| format!("subagent config '{}' not found", KG_ENRICH_SUBAGENT_ID))?;

            let enrichment_template =
                subagent_config
                    .messages
                    .user_template
                    .as_ref()
                    .ok_or_else(|| {
                        format!(
                            "messages.user_template not defined for subagent '{}'",
                            KG_ENRICH_SUBAGENT_ID
                        )
                    })?;

            let entities_str = entities.join(", ");
            let files_str = candidate_files
                .iter()
                .take(20)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n");
            let docs_str = candidate_docs
                .iter()
                .take(10)
                .map(|(id, title)| format!("- {}: {}", id, title))
                .collect::<Vec<_>>()
                .join("\n");

            let prompt = enrichment_template
                .replace("{content}", &content.chars().take(2000).collect::<String>())
                .replace("{entities}", &entities_str)
                .replace("{candidate_files}", &files_str)
                .replace("{candidate_docs}", &docs_str);

            let messages = vec![ChatMessage::new("user".to_string(), prompt)];

            let result = run_subchat_once(gcx2, KG_ENRICH_SUBAGENT_ID, messages).await?;

            let response = result
                .messages
                .last()
                .map(|m| m.content.content_text_only())
                .unwrap_or_default();

            let json_start = response.find('{').unwrap_or(0);
            let json_end = response.rfind('}').map(|i| i + 1).unwrap_or(response.len());
            let json_str = &response[json_start..json_end];

            serde_json::from_str(json_str)
                .map_err(|e| format!("Failed to parse enrichment JSON: {}", e))
        },
    )
    .await
}

pub async fn check_deprecation(
    gcx: Arc<GlobalContext>,
    new_doc_title: &str,
    new_doc_tags: &[String],
    new_doc_files: &[String],
    new_doc_snippet: &str,
    candidates: &[&KnowledgeDoc],
) -> Result<DeprecationResult, String> {
    if candidates.is_empty() {
        return Ok(DeprecationResult {
            deprecate: vec![],
            keep: vec![],
        });
    }
    let title = new_doc_title.to_string();
    let tags = new_doc_tags.to_vec();
    let files = new_doc_files.to_vec();
    let snippet = new_doc_snippet.to_string();
    let candidates: Vec<KnowledgeDoc> = candidates.iter().map(|d| (*d).clone()).collect();
    let gcx2 = gcx.clone();
    crate::buddy::workflows::buddy_wrap_workflow(
        crate::app_state::AppState::from_gcx(gcx).await,
        "kg_deprecate",
        "🗑",
        5,
        |_: &DeprecationResult| "Knowledge entry deprecated".to_string(),
        move || async move {
            let subagent_config = get_subagent_config(gcx2.clone(), KG_DEPRECATE_SUBAGENT_ID, None)
                .await
                .ok_or_else(|| {
                    format!("subagent config '{}' not found", KG_DEPRECATE_SUBAGENT_ID)
                })?;

            let deprecation_template =
                subagent_config
                    .messages
                    .user_template
                    .as_ref()
                    .ok_or_else(|| {
                        format!(
                            "messages.user_template not defined for subagent '{}'",
                            KG_DEPRECATE_SUBAGENT_ID
                        )
                    })?;

            let candidates_str = candidates
                .iter()
                .map(|doc| {
                    let id = doc
                        .frontmatter
                        .id
                        .clone()
                        .unwrap_or_else(|| doc.path.to_string_lossy().to_string());
                    let doc_title = doc.frontmatter.title.clone().unwrap_or_default();
                    let doc_tags = doc.frontmatter.tags.join(", ");
                    let doc_files = doc.frontmatter.filenames.join(", ");
                    let snippet: String = doc.content.chars().take(300).collect();
                    format!(
                        "ID: {}\nTitle: {}\nTags: {}\nFiles: {}\nSnippet: {}\n---",
                        id, doc_title, doc_tags, doc_files, snippet
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");

            let prompt = deprecation_template
                .replace("{new_title}", &title)
                .replace("{new_tags}", &tags.join(", "))
                .replace("{new_files}", &files.join(", "))
                .replace(
                    "{new_snippet}",
                    &snippet.chars().take(500).collect::<String>(),
                )
                .replace("{candidates}", &candidates_str);

            let messages = vec![ChatMessage::new("user".to_string(), prompt)];

            let result = run_subchat_once(gcx2, KG_DEPRECATE_SUBAGENT_ID, messages).await?;

            let response = result
                .messages
                .last()
                .map(|m| m.content.content_text_only())
                .unwrap_or_default();

            let json_start = response.find('{').unwrap_or(0);
            let json_end = response.rfind('}').map(|i| i + 1).unwrap_or(response.len());
            let json_str = &response[json_start..json_end];

            serde_json::from_str(json_str)
                .map_err(|e| format!("Failed to parse deprecation JSON: {}", e))
        },
    )
    .await
}
