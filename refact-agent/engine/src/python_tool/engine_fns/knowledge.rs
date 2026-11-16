use std::collections::HashSet;

use futures::future::BoxFuture;
use serde_json::{json, Value};

use super::common::{failed, limit, limit_schema, redact, to_value};
use super::mask::item;
use super::{previewed, EngineClass, EngineCtx, EngineError, EngineFn, EngineOutput, Preview};

const KINDS: &[&str] = &[
    "doc",
    "doc_trajectory",
    "doc_code",
    "doc_decision",
    "doc_deprecated",
    "doc_archived",
    "tag",
    "file",
    "entity",
];

pub fn functions() -> Vec<EngineFn> {
    vec![
        EngineFn {
            name: "knowledge.graph",
            summary: "The project knowledge graph: memory documents, tags, referenced files, entities, the links between them and counts. Trajectory memories only when asked for in kinds.",
            class: EngineClass::Read,
            input: || {
                json!({"type":"object","additionalProperties":false,"properties":{
                    "include_content":{"type":"boolean","description":"Include each memory's text (redacted); false by default."},
                    "kinds":{"type":"array","minItems":1,"uniqueItems":true,"items":{"enum":KINDS},"description":"Node types to return; every type except doc_trajectory by default."},
                    "limit":limit_schema()
                }})
            },
            output: || Some(json!({"type":"object","required":["nodes","edges","stats","total","truncated"],"properties":{"nodes":{"type":"array","items":{"type":"object","properties":{"id":{"type":"string"},"node_type":{"type":"string"},"label":{"type":"string"},"title":{"type":"string"},"content":{"type":"string"},"tags":{"type":"array"},"created":{"type":"string"},"file_path":{"type":"string"},"kind":{"type":"string"}}}},"edges":{"type":"array","items":{"type":"object","properties":{"source":{"type":"string"},"target":{"type":"string"},"edge_type":{"enum":["tagged_with","references_file","mentions","links_to","superseded_by"]}}}},"stats":{"type":"object"},"total":{"type":"integer","description":"nodes of the requested kinds"},"truncated":{"type":"boolean"}}})),
            handler: graph,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "knowledge.relink",
            summary: "Recompute automatic links between all active memories after the user approves; links are only added, never removed.",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"properties":{}}),
            output: || Some(json!({"type":"object","required":["docs_scanned","docs_updated","links_added"],"properties":{"docs_scanned":{"type":"integer"},"docs_updated":{"type":"integer"},"links_added":{"type":"integer"}}})),
            handler: relink,
            preview: Some(relink_preview),
            timeout: Some(std::time::Duration::from_secs(120)),
        },
        EngineFn {
            name: "knowledge.enrichment_preview",
            summary: "Preview of the wand rewrite: which memories auto-enrichment would attach to a text and how it would rewrite it. Costs one llm() call of this run's budget; the text goes to the enrichment model.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"required":["text"],"properties":{"text":{"type":"string","minLength":1,"description":"Cut to 2000 characters."}}}),
            output: || Some(json!({"type":"object","required":["query_used","rewritten_text","items"],"properties":{"query_used":{"type":"string"},"rewritten_text":{"type":"string"},"items":{"type":"array","items":{"type":"object","properties":{"kind":{"type":"string"},"label":{"type":"string"},"file_path":{"type":"string"},"line1":{"type":"integer"},"line2":{"type":"integer"},"content":{"type":"string"}}}}}})),
            handler: enrichment_preview,
            preview: None,
            timeout: Some(std::time::Duration::from_secs(180)),
        },
        EngineFn {
            name: "knowledge.update",
            summary: "Edit one memory (title, content, tags, kind, referenced files, links, status) after the user approves. Replacing the content is destructive: the engine keeps no copy.",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"required":["file_path"],"properties":{"file_path":{"type":"string","minLength":1,"description":"A memory file inside .refact/knowledge, as knowledge.graph returns it."},"title":{"type":"string","minLength":1},"content":{"type":"string"},"tags":{"type":"array","items":{"type":"string"}},"kind":{"type":"string","minLength":1},"filenames":{"type":"array","items":{"type":"string"}},"links":{"type":"array","items":{"type":"string"}},"status":{"enum":crate::http::routers::v1::knowledge_ops::VALID_STATUSES},"auto_link":{"type":"boolean","description":"Add automatic links; true by default."}}}),
            output: || Some(json!({"type":"object","required":["file_path","updated_fields"],"properties":{"file_path":{"type":"string"},"updated_fields":{"type":"array","items":{"type":"string"}}}})),
            handler: update,
            preview: Some(update_preview),
            timeout: Some(std::time::Duration::from_secs(120)),
        },
        EngineFn {
            name: "knowledge.delete",
            summary: "Archive a memory (archive=true, the default) or delete its file permanently (archive=false), after the user approves.",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"required":["file_path"],"properties":{"file_path":{"type":"string","minLength":1},"archive":{"type":"boolean","description":"true (default) archives; false deletes the file."}}}),
            output: || Some(json!({"type":"object","required":["file_path","archived","deleted"],"properties":{"file_path":{"type":"string"},"archived":{"type":"boolean"},"deleted":{"type":"boolean"}}})),
            handler: delete,
            preview: Some(delete_preview),
            timeout: None,
        },
    ]
}

const OVERWRITE_WARNING: &str =
    "The memory's previous text is overwritten and the engine keeps no copy.";
const DELETE_WARNING: &str =
    "The memory file is removed from disk and cannot be restored by the engine.";

/// The run's privacy records and this chat's history must be allowed to reach `destination`.
pub(super) async fn egress(
    ctx: &EngineCtx,
    destination: refact_privacy::Destination,
    what: &str,
) -> Result<(), EngineError> {
    ctx.clear_for(&destination)
        .await
        .map_err(|refusal| EngineError::Permission(format!("{what} was not sent: {refusal}")))
}

/// A memory is read by every chat of the project, whatever model it uses.
async fn shared_memory_egress(ctx: &EngineCtx, what: &str) -> Result<(), EngineError> {
    egress(
        ctx,
        refact_privacy::Destination {
            id: refact_privacy::DestinationId("project_knowledge".into()),
            kind: refact_privacy::DestinationKind::Provider,
            display_name: "project knowledge (read by every chat)".into(),
        },
        what,
    )
    .await
}

fn enrichment_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    use std::sync::atomic::Ordering;
    Box::pin(async move {
        let text = args["text"].as_str().unwrap_or_default();
        if text.trim().is_empty() {
            return Err(EngineError::Type("text must not be empty".into()));
        }
        let query: String = text
            .chars()
            .take(crate::knowledge::enrichment::MAX_QUERY_LENGTH)
            .collect();
        let budget = ctx.llm.clone().ok_or_else(|| {
            EngineError::Permission(
                "knowledge.enrichment_preview needs llm() calls and python_max_llm_calls is 0"
                    .into(),
            )
        })?;
        let used = budget.input_tokens.load(Ordering::Acquire)
            + budget.output_tokens.load(Ordering::Acquire);
        if used >= budget.max_tokens {
            return Err(EngineError::Permission(format!(
                "llm token budget exhausted ({used} of {} tokens used in this run)",
                budget.max_tokens
            )));
        }
        let config = crate::knowledge::enrichment::enrichment_config(ctx.app.gcx.clone())
            .await
            .map_err(EngineError::Failed)?;
        egress(
            ctx,
            crate::privacy::records::provider_destination(&config.model),
            "the text",
        )
        .await?;
        budget
            .started
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < budget.max_calls).then_some(n + 1)
            })
            .map_err(|_| {
                EngineError::Permission(format!(
                    "llm call budget exhausted ({} calls per run)",
                    budget.max_calls
                ))
            })?;
        budget.calls.fetch_add(1, Ordering::AcqRel);
        budget
            .models
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(config.model.clone());
        let usage: crate::python_tool::llm::UsageSink = Default::default();
        let result = crate::subchat::with_stream_usage(
            usage.clone(),
            crate::knowledge::enrichment::model_gather_and_rewrite(ctx.app.gcx.clone(), &query),
        )
        .await;
        let reported = usage.lock().unwrap_or_else(|e| e.into_inner()).clone();
        budget.input_tokens.fetch_add(
            (reported.prompt_tokens
                + reported.cache_read_tokens.unwrap_or(0)
                + reported.cache_creation_tokens.unwrap_or(0)) as u64,
            Ordering::AcqRel,
        );
        budget
            .output_tokens
            .fetch_add(reported.completion_tokens as u64, Ordering::AcqRel);
        let (rewritten, items) = result.map_err(EngineError::Failed)?;
        let paths: Vec<std::path::PathBuf> = items
            .iter()
            .map(|i| std::path::PathBuf::from(&i.context_file.file_name))
            .collect();
        let rows: Vec<Value> = items
            .iter()
            .map(|i| {
                json!({
                    "kind": i.kind,
                    "label": i.label,
                    "file_path": i.context_file.file_name,
                    "line1": i.context_file.line1,
                    "line2": i.context_file.line2,
                    "content": redact(&i.context_file.file_content),
                })
            })
            .collect();
        let mut output = EngineOutput::new(
            json!({"query_used":query,"rewritten_text":rewritten,"items":rows}),
            format!("{} memories would be attached", rows.len()),
        );
        if !paths.is_empty() {
            let records = crate::privacy::records::declared_file_records(&ctx.app.gcx, paths)
                .map_err(failed)?;
            output.privacy.push(json!({ "files": records }));
        }
        Ok(output)
    })
}

/// Fields of `knowledge.update` in card order.
const UPDATE_FIELDS: &[&str] = &[
    "title",
    "content",
    "tags",
    "kind",
    "filenames",
    "links",
    "status",
];

#[derive(Clone)]
struct MemoryPlan {
    path: std::path::PathBuf,
    frontmatter: crate::knowledge_graph::KnowledgeFrontmatter,
    body: String,
}

async fn memory_path(ctx: &EngineCtx, args: &Value) -> Result<std::path::PathBuf, EngineError> {
    use crate::http::routers::v1::knowledge_ops::{get_knowledge_root, validate_knowledge_path};
    let root = get_knowledge_root(&ctx.app.gcx)
        .await
        .map_err(|e| EngineError::Failed(e.message))?;
    validate_knowledge_path(
        std::path::Path::new(args["file_path"].as_str().unwrap_or_default()),
        &root,
    )
    .await
    .map_err(|e| EngineError::Type(format!("file_path: {}", e.message)))
}

async fn memory_text(path: &std::path::Path) -> Result<String, EngineError> {
    tokio::fs::read_to_string(path)
        .await
        .map_err(|e| EngineError::Failed(format!("cannot read the memory: {e}")))
}

/// The plan state of a memory file: a digest of its text, never the text.
fn memory_state(text: &str) -> Value {
    json!({ "memory": super::digest(text.as_bytes()) })
}

/// Frontmatter fields an update can change, as the card shows them.
const PLANNED_FIELDS: &[&str] = &[
    "title",
    "tags",
    "kind",
    "filenames",
    "links",
    "status",
    "deprecated_at",
    "superseded_by",
];

fn update_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        if !UPDATE_FIELDS.iter().any(|f| args.get(*f).is_some()) {
            return Err(EngineError::Type(format!(
                "knowledge.update needs at least one of {}",
                UPDATE_FIELDS.join(", ")
            )));
        }
        let path = memory_path(ctx, args).await?;
        let text = memory_text(&path).await?;
        if UPDATE_FIELDS
            .iter()
            .any(|f| *f != "status" && args.get(*f).is_some())
        {
            shared_memory_egress(ctx, "the new memory text").await?;
        }
        let string = |key: &str| args.get(key).and_then(Value::as_str).map(String::from);
        let list = |key: &str| {
            args.get(key).and_then(Value::as_array).map(|items| {
                items
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
        };
        let post = crate::http::routers::v1::knowledge_ops::UpdateMemoryPost {
            file_path: path.to_string_lossy().into_owned(),
            title: string("title"),
            content: string("content"),
            tags: list("tags"),
            kind: string("kind"),
            filenames: list("filenames"),
            links: list("links"),
            status: string("status"),
            auto_link: args.get("auto_link").and_then(Value::as_bool),
        };
        let (old_frontmatter, start) = crate::knowledge_graph::KnowledgeFrontmatter::parse(&text);
        let (frontmatter, body) = crate::http::routers::v1::knowledge_ops::plan_memory_update(
            ctx.app.gcx.clone(),
            &path,
            &text,
            post,
        )
        .await
        .map_err(|e| EngineError::Type(e.message))?;
        let (old, new) = (
            serde_json::to_value(&old_frontmatter).map_err(failed)?,
            serde_json::to_value(&frontmatter).map_err(failed)?,
        );
        let shown = |value: &Value| match value {
            Value::Null => None,
            Value::Array(items) if items.is_empty() => None,
            other => Some(other.clone()),
        };
        let mut items = Vec::new();
        for field in PLANNED_FIELDS {
            if old[*field] != new[*field] {
                items.push(item(field, shown(&old[*field]), shown(&new[*field])));
            }
        }
        let old_body = text.get(start..).unwrap_or("").trim();
        if args.get("content").is_some() && old_body != body.trim() {
            items.push(item(
                "content",
                (!old_body.is_empty()).then(|| json!(old_body)),
                Some(json!(body.trim())),
            ));
        }
        if items.is_empty() {
            return Ok(Preview {
                title: "edit a memory".into(),
                items,
                scope: Some(knowledge_scope(ctx)),
                class: EngineClass::Change,
                warning: None,
                plan: super::Plan::nothing(),
            });
        }
        if old["updated"] != new["updated"] {
            items.push(item(
                "updated",
                shown(&old["updated"]),
                shown(&new["updated"]),
            ));
        }
        let replaces_body = items.iter().any(|i| i.label == "content");
        Ok(Preview {
            title: "edit a memory".into(),
            items,
            scope: Some(knowledge_scope(ctx)),
            class: if replaces_body {
                EngineClass::Destructive
            } else {
                EngineClass::Change
            },
            warning: replaces_body.then(|| OVERWRITE_WARNING.to_string()),
            plan: super::Plan::new(
                memory_state(&text),
                MemoryPlan {
                    path,
                    frontmatter,
                    body,
                },
            ),
        })
    })
}

fn update<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let path = memory_path(ctx, args).await?;
        let current = memory_text(&path).await?;
        let plan: MemoryPlan = ctx.take_plan("knowledge.update", &memory_state(&current))?;
        let fields: Vec<&str> = UPDATE_FIELDS
            .iter()
            .copied()
            .filter(|f| args.get(*f).is_some())
            .collect();
        let frontmatter = plan.frontmatter;
        crate::memories::rewrite_memory_document(
            ctx.app.gcx.clone(),
            &plan.path,
            &frontmatter,
            &plan.body,
        )
        .await
        .map_err(EngineError::Failed)?;
        let title = frontmatter.title.unwrap_or_default();
        Ok(EngineOutput::new(
            json!({"file_path":plan.path.to_string_lossy(),"updated_fields":fields}),
            format!("updated memory {title}"),
        ))
    })
}

fn delete_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let path = memory_path(ctx, args).await?;
        let text = memory_text(&path).await?;
        let archive = args.get("archive").and_then(Value::as_bool) != Some(false);
        let (frontmatter, _) = crate::knowledge_graph::KnowledgeFrontmatter::parse(&text);
        if archive && (frontmatter.is_archived() || frontmatter.is_deprecated()) {
            return Ok(Preview {
                title: "archive a memory".into(),
                items: Vec::new(),
                scope: Some(knowledge_scope(ctx)),
                class: EngineClass::Change,
                warning: None,
                plan: super::Plan::nothing(),
            });
        }
        let graph = crate::knowledge_graph::build_knowledge_graph(ctx.app.gcx.clone()).await;
        let incoming = graph
            .docs
            .values()
            .filter(|doc| {
                doc.path != path
                    && frontmatter
                        .id
                        .as_ref()
                        .is_some_and(|id| doc.frontmatter.links.iter().any(|link| link == id))
            })
            .count();
        let title = frontmatter.title.clone().unwrap_or_else(|| {
            path.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
        let status = frontmatter
            .status
            .clone()
            .unwrap_or_else(|| "active".into());
        let outcome = if archive { "archived" } else { "deleted" };
        Ok(Preview {
            title: format!("{} a memory", if archive { "archive" } else { "delete" }),
            items: vec![
                item(
                    "memory",
                    Some(json!(format!("{title} ({status})"))),
                    Some(json!(outcome)),
                ),
                item(
                    "incoming links",
                    Some(json!(format!("{incoming} docs link here"))),
                    Some(json!(if incoming == 0 { "none" } else { "dangling" })),
                ),
            ],
            scope: Some(knowledge_scope(ctx)),
            class: if archive {
                EngineClass::Change
            } else {
                EngineClass::Destructive
            },
            warning: (!archive).then(|| DELETE_WARNING.to_string()),
            plan: previewed::plan(memory_state(&text)),
        })
    })
}

fn delete<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let path = memory_path(ctx, args).await?;
        let text = memory_text(&path).await?;
        previewed::take(ctx, "knowledge.delete", &memory_state(&text))?;
        let archive = args.get("archive").and_then(Value::as_bool) != Some(false);
        let title = crate::knowledge_graph::KnowledgeFrontmatter::parse(&text)
            .0
            .title
            .unwrap_or_default();
        let changed = crate::http::routers::v1::knowledge_ops::archive_or_delete_memory(
            ctx.app.gcx.clone(),
            &path,
            archive,
        )
        .await
        .map_err(|e| EngineError::Failed(e.message))?;
        Ok(EngineOutput::new(
            json!({"file_path":path.to_string_lossy(),"archived":archive && changed,"deleted":!archive}),
            format!("{} {title}", if archive { "archived" } else { "deleted" }),
        ))
    })
}

fn graph<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let include_content = args.get("include_content").and_then(Value::as_bool) == Some(true);
        let kinds: Vec<String> = match args.get("kinds").and_then(Value::as_array) {
            Some(kinds) => kinds
                .iter()
                .filter_map(|k| k.as_str().map(String::from))
                .collect(),
            None => KINDS
                .iter()
                .filter(|k| **k != "doc_trajectory")
                .map(|k| k.to_string())
                .collect(),
        };
        let limit = limit(args);
        let gcx = ctx.app.gcx.clone();
        let full = to_value(
            crate::http::routers::v1::knowledge_graph::knowledge_graph_json(
                gcx.clone(),
                include_content,
            )
            .await,
        )?;
        let filter = super::common::PathFilter::new(ctx).await?;
        let workspace = gcx
            .documents_state
            .workspace_folders
            .lock()
            .unwrap()
            .first()
            .cloned();
        let file_allowed = |id: &str| {
            let Some(path) = id.strip_prefix("file:") else {
                return true;
            };
            let path = std::path::Path::new(path);
            let absolute = if path.is_absolute() {
                Some(path.to_path_buf())
            } else {
                workspace.as_ref().map(|root| root.join(path))
            };
            absolute.is_some_and(|p| filter.allows(&p.to_string_lossy()))
        };
        let nodes: Vec<Value> = full["nodes"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|n| {
                n["node_type"]
                    .as_str()
                    .is_some_and(|t| kinds.iter().any(|k| k == t))
                    && n["id"].as_str().is_some_and(|id| file_allowed(id))
            })
            .collect();
        let total = nodes.len();
        let mut nodes: Vec<Value> = nodes.into_iter().take(limit).collect();
        let kept: HashSet<String> = nodes
            .iter()
            .filter_map(|n| n["id"].as_str().map(String::from))
            .collect();
        let edges: Vec<Value> = full["edges"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|e| {
                [&e["source"], &e["target"]]
                    .iter()
                    .all(|end| end.as_str().is_some_and(|id| kept.contains(id)))
            })
            .collect();
        let mut doc_paths = Vec::new();
        for node in &mut nodes {
            if let Some(content) = node["content"].as_str() {
                node["content"] = json!(redact(content));
            }
            if let Some(path) = node["file_path"].as_str() {
                doc_paths.push(std::path::PathBuf::from(path));
            }
        }
        let mut output = EngineOutput::new(Value::Null, "");
        if !doc_paths.is_empty() {
            crate::privacy::load_privacy_if_needed(gcx.clone()).await;
            let records =
                crate::privacy::records::declared_file_records(&gcx, doc_paths).map_err(|e| {
                    EngineError::Failed(format!(
                        "privacy records of the memories could not be made: {e}"
                    ))
                })?;
            output.privacy.push(json!({"files":to_value(records)?}));
        }
        let stats = &full["stats"];
        output.text = format!(
            "{} docs ({} active), {} tags, {} edges",
            stats["doc_count"], stats["active_docs"], stats["tag_count"], stats["edge_count"]
        );
        output.data = json!({"nodes":nodes,"edges":edges,"stats":stats,"total":total,"truncated":total > limit});
        Ok(output)
    })
}

async fn active_docs(ctx: &EngineCtx) -> usize {
    crate::knowledge_graph::build_knowledge_graph(ctx.app.gcx.clone())
        .await
        .docs
        .values()
        .filter(|d| {
            d.frontmatter.is_active() && d.frontmatter.kind.as_deref() != Some("trajectory")
        })
        .count()
}

fn knowledge_scope(ctx: &EngineCtx) -> String {
    let root = ctx
        .app
        .gcx
        .documents_state
        .workspace_folders
        .lock()
        .unwrap()
        .first()
        .map(|root| root.to_string_lossy().into_owned())
        .unwrap_or_default();
    format!("project knowledge ({root})")
}

fn relink_preview<'a>(
    ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let active = active_docs(ctx).await;
        if active == 0 {
            return Err(EngineError::Type(
                "there are no active memories to relink".into(),
            ));
        }
        let approved = previewed::plan(json!(active));
        Ok(Preview {
            plan: approved,
            title: "relink memories".into(),
            items: vec![item(
                "active memories",
                Some(json!(active.to_string())),
                Some(json!("links recomputed (only additions)")),
            )],
            scope: Some(knowledge_scope(ctx)),
            class: EngineClass::Change,
            warning: None,
        })
    })
}

fn relink<'a>(
    ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        previewed::take(ctx, "knowledge.relink", &json!(active_docs(ctx).await))?;
        let stats =
            crate::http::routers::v1::knowledge_ops::auto_link_all_memories(ctx.app.gcx.clone())
                .await
                .map_err(failed)?;
        let text = format!(
            "relinked: {} docs updated, {} links added",
            stats.docs_updated, stats.links_added
        );
        Ok(EngineOutput::new(to_value(&stats)?, text))
    })
}
