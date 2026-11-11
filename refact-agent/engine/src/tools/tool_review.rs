#[path = "tool_review/schema.rs"]
mod schema;
#[cfg(test)]
#[path = "tool_review/smoke_tests.rs"]
mod smoke_tests;

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use refact_review_core::{Review, Side};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::review::authoring::{self, comments, open, read, write, Target};
use crate::review::remote;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ReviewToolKind {
    Open,
    Read,
    Tour,
    Finding,
    Source,
    Reply,
    ImportComments,
    Outbox,
    PrepareExecution,
    RecordEvidence,
}

impl ReviewToolKind {
    pub const ALL: [ReviewToolKind; 10] = [
        ReviewToolKind::Open,
        ReviewToolKind::Read,
        ReviewToolKind::Tour,
        ReviewToolKind::Finding,
        ReviewToolKind::Source,
        ReviewToolKind::Reply,
        ReviewToolKind::ImportComments,
        ReviewToolKind::Outbox,
        ReviewToolKind::PrepareExecution,
        ReviewToolKind::RecordEvidence,
    ];
}

pub struct ToolReview(pub ReviewToolKind);

impl ToolReview {
    pub fn name(&self) -> &'static str {
        match self.0 {
            ReviewToolKind::Open => "review_open",
            ReviewToolKind::Read => "review_read",
            ReviewToolKind::Tour => "review_tour",
            ReviewToolKind::Finding => "review_finding",
            ReviewToolKind::Source => "review_source",
            ReviewToolKind::Reply => "review_reply",
            ReviewToolKind::ImportComments => "review_import_comments",
            ReviewToolKind::Outbox => "review_outbox",
            ReviewToolKind::PrepareExecution => "review_prepare_execution",
            ReviewToolKind::RecordEvidence => "review_record_evidence",
        }
    }
}

type Args = HashMap<String, Value>;
type Done = Result<(bool, Vec<ContextEnum>), String>;

fn text(args: &Args, name: &str) -> Option<String> {
    args.get(name)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.trim().is_empty())
}

fn flag(args: &Args, name: &str) -> bool {
    args.get(name).and_then(Value::as_bool).unwrap_or(false)
}

fn parsed<T: DeserializeOwned>(value: Option<&Value>) -> Option<T> {
    value
        .filter(|value| !value.is_null())
        .and_then(|value| serde_json::from_value(value.clone()).ok())
}

fn items<'a>(args: &'a Args, name: &str) -> impl Iterator<Item = &'a Value> {
    args.get(name)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

fn side(value: Option<&Value>) -> Side {
    parsed(value).unwrap_or(Side::Head)
}

/// Declares only what this result quotes, plus the review's guarded provenance.
fn done(target: Option<&Target>, call_id: &str, value: Value, quoted: &[String]) -> Done {
    let mut message = ChatMessage {
        role: "tool".into(),
        tool_call_id: call_id.into(),
        content: ChatContent::SimpleText(value.to_string()),
        ..Default::default()
    };
    if let Some(target) = target {
        attach(&mut message, target, &target.review, quoted)?;
    }
    Ok((false, vec![ContextEnum::ChatMessage(message)]))
}

fn attach(
    message: &mut ChatMessage,
    target: &Target,
    review: &Review,
    quoted: &[String],
) -> Result<(), String> {
    message.extra.insert(
        "review".into(),
        json!({"id":review.id,"revision":review.revision,"candidate_id":review.current_candidate_id}),
    );
    let mut records = crate::privacy::records::declared_file_records(
        &target.app.gcx,
        authoring::origin_paths(target, quoted),
    )?;
    records.extend(crate::review::service::guarded_records(review));
    crate::privacy::records::merge_records(message, records);
    Ok(())
}

fn saved(target: &Target, review: Review, call_id: &str, value: Value, quoted: &[String]) -> Done {
    let mut message = ChatMessage {
        role: "tool".into(),
        tool_call_id: call_id.into(),
        content: ChatContent::SimpleText(value.to_string()),
        ..Default::default()
    };
    attach(&mut message, target, &review, quoted)?;
    Ok((false, vec![ContextEnum::ChatMessage(message)]))
}

async fn read_review(ccx: &authoring::Ccx, call_id: &str, args: &Args) -> Done {
    let member = text(args, "member");
    let Some(target) = authoring::target(ccx, member.as_deref()).await? else {
        return done(
            None,
            call_id,
            json!({"review":null,"next":authoring::NOT_OPEN}),
            &[],
        );
    };
    if let Some(path) = text(args, "path") {
        let (value, path) = read::file(
            &target,
            &path,
            text(args, "lines").as_deref(),
            side(args.get("side")),
            flag(args, "diff"),
        )
        .await?;
        return done(Some(&target), call_id, value, &[path]);
    }
    let quoted = read::quoted_paths(&target.review);
    let value = if let Some(id) = text(args, "page") {
        read::page(&target.review, &id)?
    } else if let Some(id) = text(args, "finding") {
        read::finding(&target.review, &id)?
    } else if let Some(id) = text(args, "thread") {
        read::thread(&target.review, &id)?
    } else {
        read::overview(&target).await?
    };
    done(Some(&target), call_id, value, &quoted)
}

async fn open_review(ccx: &authoring::Ccx, call_id: &str, args: &Args) -> Done {
    let stack = items(args, "stack")
        .map(|entry| open::StackEntry {
            pr: entry["pr"].as_u64(),
            branch: entry["branch"].as_str().unwrap_or_default().to_string(),
            head: entry["head"].as_str().map(str::to_string),
            base: entry["base"].as_str().map(str::to_string),
        })
        .collect();
    let (review, value) = open::open(
        ccx,
        open::OpenInput {
            repository: text(args, "repository"),
            title: text(args, "title"),
            target: text(args, "target"),
            base: text(args, "base"),
            head: text(args, "head"),
            local_changes: flag(args, "local_changes"),
            stack,
            focus: text(args, "focus"),
        },
        call_id,
    )
    .await?;
    let target = authoring::require(ccx, None).await?;
    let changed: Vec<String> = authoring::candidate(&review)
        .map(|candidate| {
            authoring::changed_files(candidate)
                .iter()
                .map(|(path, _)| (*path).to_string())
                .collect()
        })
        .unwrap_or_default();
    saved(&target, review, call_id, value, &changed)
}

async fn outbox(ccx: &authoring::Ccx, call_id: &str, args: &Args) -> Done {
    let mut target = authoring::require(ccx, text(args, "member").as_deref()).await?;
    let source = text(args, "source").unwrap_or_else(|| remote::SOURCE.into());
    for id in items(args, "request").filter_map(Value::as_str) {
        target.review = comments::request(&target, &source, id).await?;
    }
    for item in items(args, "published") {
        target.review = comments::published(
            &target,
            &source,
            item["id"].as_str().unwrap_or_default(),
            item["remote_id"].as_str().unwrap_or_default(),
            None,
            item["url"].as_str().map(str::to_string),
        )
        .await?;
    }
    let sent = if flag(args, "send") {
        Some(remote::sync(&mut target, remote::Direction::PUSH).await)
    } else {
        None
    };
    crate::review::chat_bridge::invalidate(&target.app, &target.review).await;
    let mut value = comments::waiting(&target.review)?;
    if let Some(report) = sent {
        value["sent"] = json!(report);
    }
    let quoted = read::quoted_paths(&target.review);
    done(Some(&target), call_id, value, &quoted)
}

fn remote_comment(item: &Value) -> comments::RemoteComment {
    let string = |name: &str| item[name].as_str().map(str::to_string);
    comments::RemoteComment {
        id: string("id").unwrap_or_default(),
        author: string("author").unwrap_or_default(),
        body: string("body").unwrap_or_default(),
        path: string("path"),
        line: item["line"].as_u64().map(|line| line as u32),
        start_line: item["start_line"].as_u64().map(|line| line as u32),
        side: side(item.get("side")),
        reply_to: string("reply_to"),
        url: string("url"),
        created_at_ms: string("created_at")
            .and_then(|time| chrono::DateTime::parse_from_rfc3339(&time).ok())
            .map(|time| time.timestamp_millis()),
    }
}

async fn evidence(kind: ReviewToolKind, ccx: &authoring::Ccx, call_id: &str, args: &Args) -> Done {
    let target = authoring::require(ccx, None).await?;
    if kind == ReviewToolKind::PrepareExecution {
        let material =
            crate::review::evidence::prepare_execution(ccx.clone(), &format!("tool:{call_id}"))
                .await?;
        let value = json!({
            "material_id": material.id,
            "next": "Start the run now, wait for it to finish, then call review_record_evidence with its process_id and this material_id.",
        });
        return done(Some(&target), call_id, value, &[]);
    }
    let changed = crate::review::evidence::record_prepared_process_evidence(
        ccx.clone(),
        &target.review.id,
        &text(args, "process_id").unwrap_or_default(),
        &text(args, "summary").unwrap_or_default(),
        parsed(args.get("retained_kind"))
            .unwrap_or(refact_review_core::MaterialKind::Investigation),
        text(args, "prepared_material_id").as_deref(),
    )
    .await?;
    crate::review::chat_bridge::invalidate(&target.app, &changed).await;
    let recorded = changed.evidence.last().map(|item| {
        let exit_code = match &item.source {
            refact_review_core::EvidenceSource::Executed {
                exit_code, ..
            } => *exit_code,
            _ => None,
        };
        json!({"id":item.id,"exit_code":exit_code})
    });
    let value = json!({
        "evidence": recorded,
        "use": "Cite it on a tour page with <refact-evidence ref=\"…\"></refact-evidence>.",
    });
    saved(&target, changed, call_id, value, &[])
}

#[async_trait]
impl Tool for ToolReview {
    fn tool_description(&self) -> ToolDesc {
        let (display_name, description, input_schema) = schema::spec(self.0);
        ToolDesc {
            name: self.name().into(),
            display_name: display_name.into(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel: self.0 == ReviewToolKind::Read,
            description: description.into(),
            input_schema,
            output_schema: None,
            annotations: None,
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<Mutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        schema::validate(self.name(), &schema::spec(self.0).2, &json!(args))?;
        let member = text(args, "member");
        match self.0 {
            ReviewToolKind::Open => open_review(&ccx, tool_call_id, args).await,
            ReviewToolKind::Read => read_review(&ccx, tool_call_id, args).await,
            ReviewToolKind::Tour => {
                let target = authoring::require(&ccx, member.as_deref()).await?;
                let edits = items(args, "edits")
                    .map(|edit| {
                        (
                            edit["old"].as_str().unwrap_or_default().to_string(),
                            edit["new"].as_str().unwrap_or_default().to_string(),
                        )
                    })
                    .collect();
                let (review, value, quoted) = write::tour_page(
                    &ccx,
                    &target,
                    write::PageInput {
                        id: text(args, "id").unwrap_or_default(),
                        title: text(args, "title"),
                        summary: text(args, "summary"),
                        html: text(args, "html"),
                        edits,
                    },
                    tool_call_id,
                )
                .await?;
                saved(&target, review, tool_call_id, value, &quoted)
            }
            ReviewToolKind::Finding => {
                let target = authoring::require(&ccx, member.as_deref()).await?;
                let (review, value, quoted) = write::finding(
                    &ccx,
                    &target,
                    write::FindingInput {
                        id: text(args, "id"),
                        title: text(args, "title"),
                        body: text(args, "body"),
                        path: text(args, "path"),
                        lines: text(args, "lines"),
                        side: parsed(args.get("side")),
                        kind: parsed(args.get("kind")),
                        severity: parsed(args.get("severity")),
                        confidence: parsed(args.get("confidence")),
                        assessment: parsed(args.get("assessment")),
                        disposition: parsed(args.get("status")),
                    },
                    tool_call_id,
                )
                .await?;
                saved(&target, review, tool_call_id, value, &quoted)
            }
            ReviewToolKind::Source => {
                let target = authoring::require(&ccx, member.as_deref()).await?;
                let (review, value, quoted) = write::source(
                    &ccx,
                    &target,
                    write::SourceInput {
                        id: text(args, "id"),
                        summary: text(args, "summary").unwrap_or_default(),
                        url: text(args, "url"),
                        path: text(args, "path"),
                        lines: text(args, "lines"),
                    },
                    tool_call_id,
                )
                .await?;
                saved(&target, review, tool_call_id, value, &quoted)
            }
            ReviewToolKind::Reply => {
                let thread = text(args, "thread").unwrap_or_default();
                let target = match member.as_deref() {
                    Some(_) => authoring::require(&ccx, member.as_deref()).await?,
                    None => authoring::thread_home(&ccx, &thread).await?,
                };
                let (review, value) = write::reply(
                    &ccx,
                    &target,
                    &thread,
                    text(args, "body").unwrap_or_default(),
                    tool_call_id,
                )
                .await?;
                saved(&target, review, tool_call_id, value, &[])
            }
            ReviewToolKind::ImportComments => {
                let mut target = authoring::require(&ccx, member.as_deref()).await?;
                let value = if args.get("comments").is_some_and(Value::is_array) {
                    comments::import(
                        &target,
                        text(args, "source").unwrap_or_else(|| remote::SOURCE.into()),
                        items(args, "comments").map(remote_comment).collect(),
                    )
                    .await?
                } else {
                    json!(remote::sync(&mut target, remote::Direction::PULL).await)
                };
                done(Some(&target), tool_call_id, value, &[])
            }
            ReviewToolKind::Outbox => outbox(&ccx, tool_call_id, args).await,
            ReviewToolKind::PrepareExecution | ReviewToolKind::RecordEvidence => {
                evidence(self.0, &ccx, tool_call_id, args).await
            }
        }
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_advertises_a_plain_object_schema_without_runtime_bookkeeping() {
        for kind in ReviewToolKind::ALL {
            let description = ToolReview(kind).tool_description();
            let schema = &description.input_schema;
            assert_eq!(schema["type"], "object", "{}", description.name);
            assert_eq!(schema["additionalProperties"], false);
            for unsupported in ["oneOf", "anyOf", "allOf"] {
                assert!(schema.get(unsupported).is_none(), "{}", description.name);
            }
            let text = schema.to_string();
            for runtime_owned in [
                "expected_revision",
                "request_key",
                "content_hash",
                "candidate_id",
            ] {
                assert!(
                    !text.contains(runtime_owned),
                    "{}: {runtime_owned}",
                    description.name
                );
            }
            assert_eq!(
                description.clone().into_openai_style(true)["function"]["parameters"],
                description.input_schema
            );
        }
    }

    #[test]
    fn rejected_arguments_name_every_bad_parameter_and_never_echo_values() {
        let schema = schema::spec(ReviewToolKind::Open).2;
        let error = schema::validate(
            "review_open",
            &schema,
            &json!({"action":"SECRET","base":7,"stack":[{"pr":0,"head":"x"}]}),
        )
        .unwrap_err();
        for expected in [
            "review_open was not run",
            "`action` is not a parameter here",
            "parameters are: ",
            "`base` must be of type string",
            "`stack[0].branch` is required",
            "`stack[0].pr` must be at least 1",
        ] {
            assert!(error.contains(expected), "{expected}: {error}");
        }
        assert!(!error.contains("SECRET"));
        let finding = schema::spec(ReviewToolKind::Finding).2;
        let error = schema::validate("review_finding", &finding, &json!({"severity":"urgent"}))
            .unwrap_err();
        assert!(error.contains("`severity` must be one of: info, low, medium, high, critical"));
        assert!(
            schema::validate("review_finding", &finding, &json!({"id":null,"title":"t"})).is_ok()
        );
    }
}
