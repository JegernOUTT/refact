//! Tool surface for structured disagreement records.
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use refact_work_core::store::records::{
    now_ms, AdjudicationVerdict, DisagreementPosition, DisagreementRecord, DisagreementStatus,
    EvidenceKind, EvidenceRef,
};
use refact_work_core::{DisagreementFiling, RuntimeKind, WorkKey};

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::coordination::disagreement::{
    blocks_completion, completion_gate, ledger_for, render, render_all, store_resolution,
    DisagreementLedger, NO_STORE,
};

pub struct ToolDisagreementFile {
    pub config_path: String,
}

pub struct ToolDisagreementList {
    pub config_path: String,
}

pub struct ToolDisagreementAdjudicate {
    pub config_path: String,
}

#[derive(Deserialize)]
struct EvidenceArg {
    kind: EvidenceKind,
    reference: String,
}

impl From<EvidenceArg> for EvidenceRef {
    fn from(arg: EvidenceArg) -> Self {
        EvidenceRef::new(arg.kind, arg.reference)
    }
}

#[derive(Deserialize)]
struct PositionArg {
    position_id: String,
    author: String,
    #[serde(default = "first_generation")]
    run_generation: i64,
    statement: String,
    #[serde(default)]
    evidence: Vec<EvidenceArg>,
}

fn first_generation() -> i64 {
    1
}

#[derive(Deserialize)]
struct FileArgs {
    record_id: String,
    claim: String,
    subject: String,
    subject_revision: String,
    positions: Vec<PositionArg>,
}

#[derive(Deserialize)]
struct ListArgs {
    #[serde(default)]
    record_id: Option<String>,
    #[serde(default)]
    status: Option<DisagreementStatus>,
}

#[derive(Deserialize)]
struct AdjudicateArgs {
    record_id: String,
    current_revision: String,
    #[serde(default)]
    adjudicator: Option<String>,
    #[serde(default)]
    resolved_position: Option<String>,
    #[serde(default)]
    decisive_evidence: Option<EvidenceArg>,
    #[serde(default)]
    rationale: Option<String>,
    #[serde(default)]
    unresolvable_reason: Option<String>,
}

/// `runtime:id` as `WorkKey` prints it; a bare id is a background agent.
pub(crate) fn parse_author(raw: &str) -> WorkKey {
    match raw.split_once(':') {
        Some((runtime, id)) => match RuntimeKind::parse(runtime) {
            Some(runtime) if !id.is_empty() => WorkKey::new(runtime, id),
            _ => WorkKey::background_agent(raw),
        },
        None => WorkKey::background_agent(raw),
    }
}

fn parse<T: serde::de::DeserializeOwned>(
    args: &HashMap<String, Value>,
    tool_name: &str,
) -> Result<T, String> {
    serde_json::from_value(json!(args)).map_err(|err| format!("{tool_name}: {err}"))
}

fn tool_message(tool_call_id: &str, content: String) -> ContextEnum {
    ContextEnum::ChatMessage(ChatMessage {
        role: "tool".to_string(),
        content: ChatContent::SimpleText(content),
        tool_calls: None,
        tool_call_id: tool_call_id.to_string(),
        ..Default::default()
    })
}

async fn open_ledger(ccx: &Arc<AMutex<AtCommandsContext>>) -> Result<DisagreementLedger, String> {
    let app = ccx.lock().await.app.clone();
    ledger_for(&app).await.ok_or_else(|| NO_STORE.to_string())
}

async fn caller_id(ccx: &Arc<AMutex<AtCommandsContext>>) -> String {
    ccx.lock().await.chat_id.clone()
}

fn evidence_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["kind", "reference"],
        "properties": {
            "kind": {
                "type": "string",
                "enum": EvidenceKind::ALL.map(|kind| kind.as_str()),
                "description": "evidence for an id in the evidence ledger; otherwise the kind of source being referenced"
            },
            "reference": {"type": "string", "minLength": 1}
        }
    })
}

fn builtin_source(config_path: &str) -> crate::tools::tools_description::ToolSource {
    crate::tools::tools_description::ToolSource {
        source_type: crate::tools::tools_description::ToolSourceType::Builtin,
        config_path: config_path.to_string(),
    }
}

#[async_trait]
impl crate::tools::tools_description::Tool for ToolDisagreementFile {
    fn tool_description(&self) -> crate::tools::tools_description::ToolDesc {
        crate::tools::tools_description::ToolDesc {
            name: "disagreement_file".to_string(),
            display_name: "File Disagreement".to_string(),
            source: builtin_source(&self.config_path),
            experimental: false,
            allow_parallel: false,
            description: "Record that two runs reached opposite conclusions about the same claim. \
                Both positions are stored with their author, run generation, evidence and the \
                revision each argued against. A record_id already filed is returned unchanged — \
                filing never overwrites an argument someone else already made."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["record_id", "claim", "subject", "subject_revision", "positions"],
                "properties": {
                    "record_id": {"type": "string", "minLength": 1, "description": "Stable id for this disagreement"},
                    "claim": {"type": "string", "minLength": 1, "description": "The claim in dispute"},
                    "subject": {"type": "string", "minLength": 1, "description": "What the claim is about: a path, spec id or document reference"},
                    "subject_revision": {"type": "string", "minLength": 1, "description": "Revision of the subject both sides argued against"},
                    "positions": {
                        "type": "array",
                        "minItems": 2,
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["position_id", "author", "statement"],
                            "properties": {
                                "position_id": {"type": "string", "minLength": 1},
                                "author": {"type": "string", "minLength": 1, "description": "Author work key, e.g. background_agent:<id>"},
                                "run_generation": {"type": "integer", "minimum": 1},
                                "statement": {"type": "string", "minLength": 1},
                                "evidence": {"type": "array", "items": evidence_schema()}
                            }
                        }
                    }
                }
            }),
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
        let parsed: FileArgs = parse(args, "disagreement_file")?;
        let ledger = open_ledger(&ccx).await?;
        let at_ms = now_ms();
        let record = DisagreementRecord {
            project: ledger.project().clone(),
            record_id: parsed.record_id,
            claim: parsed.claim,
            subject: parsed.subject,
            positions: parsed
                .positions
                .into_iter()
                .map(|position| DisagreementPosition {
                    position_id: position.position_id,
                    author: parse_author(&position.author),
                    run_generation: position.run_generation,
                    statement: position.statement,
                    evidence: position
                        .evidence
                        .into_iter()
                        .map(EvidenceRef::from)
                        .collect(),
                    asserted_at_ms: at_ms,
                    subject_revision: parsed.subject_revision.clone(),
                })
                .collect(),
            status: DisagreementStatus::Open,
            adjudication: None,
            open_reason: None,
            filed_at_ms: at_ms,
            updated_at_ms: at_ms,
        };
        record.validate().map_err(|refusal| refusal.to_string())?;
        let output = match ledger.file(&record).await.map_err(|err| err.to_string())? {
            DisagreementFiling::Filed(filed) => {
                format!("Disagreement filed.\n\n{}", render(&filed))
            }
            DisagreementFiling::Exists(existing) => format!(
                "A disagreement is already on record under `{}` and was left untouched.\n\n{}",
                existing.record_id,
                render(&existing)
            ),
            DisagreementFiling::OpenLimit(max) => format!(
                "Refused: this project already holds {max} open disagreement(s), the configured \
                 maximum. Settle one before filing another."
            ),
        };
        Ok((false, vec![tool_message(tool_call_id, output)]))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl crate::tools::tools_description::Tool for ToolDisagreementList {
    fn tool_description(&self) -> crate::tools::tools_description::ToolDesc {
        crate::tools::tools_description::ToolDesc {
            name: "disagreement_list".to_string(),
            display_name: "List Disagreements".to_string(),
            source: builtin_source(&self.config_path),
            experimental: false,
            allow_parallel: true,
            description: "Read recorded disagreements. Both positions are shown, including the one \
                an adjudication rejected. Reports whether open disagreements block completion under \
                the current policy; reading changes nothing."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "record_id": {"type": "string", "minLength": 1, "description": "Read one record instead of the project's list"},
                    "status": {
                        "type": "string",
                        "enum": DisagreementStatus::ALL.map(|status| status.as_str()),
                        "description": "Only records in this state"
                    }
                }
            }),
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
        let parsed: ListArgs = parse(args, "disagreement_list")?;
        let ledger = open_ledger(&ccx).await?;
        let records = match parsed.record_id {
            Some(record_id) => ledger
                .get(&record_id)
                .await
                .map_err(|err| err.to_string())?
                .into_iter()
                .collect(),
            None => ledger
                .list(parsed.status)
                .await
                .map_err(|err| err.to_string())?,
        };
        let gate = completion_gate(&records, blocks_completion());
        Ok((
            false,
            vec![tool_message(tool_call_id, render_all(&records, &gate))],
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl crate::tools::tools_description::Tool for ToolDisagreementAdjudicate {
    fn tool_description(&self) -> crate::tools::tools_description::ToolDesc {
        crate::tools::tools_description::ToolDesc {
            name: "disagreement_adjudicate".to_string(),
            display_name: "Adjudicate Disagreement".to_string(),
            source: builtin_source(&self.config_path),
            experimental: false,
            allow_parallel: false,
            description: "Settle a disagreement on the evidence presented. The decisive evidence \
                must be evidence the winning position itself presented, and the rejected position \
                is kept. If neither side has decisive evidence, pass unresolvable_reason and the \
                record stays open. If the subject moved since the positions were argued, the record \
                becomes stale and must be re-argued."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["record_id", "current_revision"],
                "properties": {
                    "record_id": {"type": "string", "minLength": 1},
                    "current_revision": {"type": "string", "minLength": 1, "description": "Revision of the subject as it stands now"},
                    "adjudicator": {"type": "string", "minLength": 1, "description": "Who or what is deciding. Defaults to the calling chat"},
                    "resolved_position": {"type": "string", "minLength": 1, "description": "position_id the evidence supports"},
                    "decisive_evidence": evidence_schema(),
                    "rationale": {"type": "string", "minLength": 1, "description": "Why that evidence decides it"},
                    "unresolvable_reason": {"type": "string", "minLength": 1, "description": "Why neither side has decisive evidence. Leaves the record open"}
                }
            }),
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
        let parsed: AdjudicateArgs = parse(args, "disagreement_adjudicate")?;
        let adjudicator = match parsed.adjudicator {
            Some(adjudicator) => adjudicator,
            None => caller_id(&ccx).await,
        };
        let verdict = match (
            parsed.resolved_position,
            parsed.decisive_evidence,
            parsed.unresolvable_reason,
        ) {
            (Some(position_id), Some(evidence), reason) => AdjudicationVerdict::Decided {
                position_id,
                decisive_evidence: evidence.into(),
                rationale: parsed
                    .rationale
                    .or(reason)
                    .ok_or_else(|| "a decision needs a rationale".to_string())?,
            },
            (None, None, Some(reason)) => AdjudicationVerdict::Unresolvable { reason },
            _ => {
                return Err(
                    "pass resolved_position with decisive_evidence and rationale, or \
                            unresolvable_reason alone"
                        .to_string(),
                )
            }
        };
        let ledger = open_ledger(&ccx).await?;
        let mut record = ledger
            .get(&parsed.record_id)
            .await
            .map_err(|err| err.to_string())?
            .ok_or_else(|| format!("no disagreement `{}` is on record", parsed.record_id))?;
        let outcome = record.resolve(&adjudicator, verdict, &parsed.current_revision, now_ms());
        store_resolution(&ledger, &record)
            .await
            .map_err(|err| err.to_string())?;
        let output = match outcome {
            Ok(status) => format!(
                "Disagreement `{}` is now {}.\n\n{}",
                record.record_id,
                status.as_str(),
                render(&record)
            ),
            Err(refusal) => format!("Not adjudicated: {refusal}.\n\n{}", render(&record)),
        };
        Ok((false, vec![tool_message(tool_call_id, output)]))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::tools_description::Tool;

    fn args(items: &[(&str, Value)]) -> HashMap<String, Value> {
        items
            .iter()
            .map(|(key, value)| ((*key).to_string(), value.clone()))
            .collect()
    }

    fn output_text(result: (bool, Vec<ContextEnum>)) -> String {
        match result.1.into_iter().next().unwrap() {
            ContextEnum::ChatMessage(message) => match message.content {
                ChatContent::SimpleText(text) => text,
                _ => panic!("expected text output"),
            },
            _ => panic!("expected chat message"),
        }
    }

    async fn ccx(
        cache: &std::path::Path,
        config: &std::path::Path,
    ) -> Arc<AMutex<AtCommandsContext>> {
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            cache.to_path_buf(),
            config.to_path_buf(),
        )
        .await;
        Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                crate::app_state::AppState::from_gcx(gcx).await,
                4096,
                20,
                false,
                vec![],
                "reviewer-chat".to_string(),
                None,
                "model".to_string(),
                None,
                None,
            )
            .await,
        ))
    }

    fn filing(record_id: &str, claim: &str, positions: Value) -> HashMap<String, Value> {
        args(&[
            ("record_id", json!(record_id)),
            ("claim", json!(claim)),
            ("subject", json!("src/chat/session.rs")),
            ("subject_revision", json!("sha-abc")),
            ("positions", positions),
        ])
    }

    fn two_positions() -> Value {
        json!([
            {
                "position_id": "held",
                "author": "background_agent:agent-a",
                "run_generation": 2,
                "statement": "the guard outlives the await",
                "evidence": [{"kind": "file", "reference": "src/chat/session.rs:210"}]
            },
            {
                "position_id": "dropped",
                "author": "trajectory:chat-b",
                "statement": "the guard is dropped first",
                "evidence": []
            }
        ])
    }

    #[tokio::test]
    async fn the_disagreement_tool_surface_files_reads_and_adjudicates_without_losing_a_side() {
        let cache = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let ccx = ccx(cache.path(), config.path()).await;
        let app = ccx.lock().await.app.clone();
        assert!(
            ledger_for(&app).await.is_some(),
            "the tool surface needs a coordination store to exercise"
        );

        let filed = output_text(
            ToolDisagreementFile {
                config_path: String::new(),
            }
            .tool_execute(
                ccx.clone(),
                &"call-file".to_string(),
                &filing(
                    "d-tool",
                    "the lock is held across the await",
                    two_positions(),
                ),
            )
            .await
            .unwrap(),
        );
        assert!(filed.contains("Disagreement filed."), "{filed}");
        assert!(filed.contains("position `held`"), "{filed}");
        assert!(filed.contains("position `dropped`"), "{filed}");

        let second = output_text(
            ToolDisagreementFile {
                config_path: String::new(),
            }
            .tool_execute(
                ccx.clone(),
                &"call-file-again".to_string(),
                &filing(
                    "d-tool",
                    "there was never a dispute",
                    json!([
                        {"position_id": "mine", "author": "agent-c", "statement": "only my view"},
                        {"position_id": "also-mine", "author": "agent-c", "statement": "and this one"}
                    ]),
                ),
            )
            .await
            .unwrap(),
        );
        assert!(second.contains("left untouched"), "{second}");
        assert!(
            second.contains("the lock is held across the await"),
            "a later writer must not replace the filed claim: {second}"
        );
        assert!(!second.contains("position `mine`"), "{second}");

        let adjudicated = output_text(
            ToolDisagreementAdjudicate {
                config_path: String::new(),
            }
            .tool_execute(
                ccx.clone(),
                &"call-adjudicate".to_string(),
                &args(&[
                    ("record_id", json!("d-tool")),
                    ("current_revision", json!("sha-abc")),
                    ("resolved_position", json!("held")),
                    (
                        "decisive_evidence",
                        json!({"kind": "file", "reference": "src/chat/session.rs:210"}),
                    ),
                    ("rationale", json!("the line shows the guard scope")),
                ]),
            )
            .await
            .unwrap(),
        );
        assert!(adjudicated.contains("is now adjudicated"), "{adjudicated}");
        assert!(
            adjudicated.contains("file:src/chat/session.rs:210"),
            "the decisive evidence must be cited: {adjudicated}"
        );
        assert!(
            adjudicated.contains("position `dropped`"),
            "the rejected side must stay readable: {adjudicated}"
        );

        let listed = output_text(
            ToolDisagreementList {
                config_path: String::new(),
            }
            .tool_execute(
                ccx.clone(),
                &"call-list".to_string(),
                &args(&[("record_id", json!("d-tool"))]),
            )
            .await
            .unwrap(),
        );
        assert!(listed.contains("position `dropped`"), "{listed}");
        assert!(listed.contains("stay readable"), "{listed}");
        assert!(listed.contains("0 open disagreement(s)"), "{listed}");
    }

    #[tokio::test]
    async fn the_disagreement_tool_refuses_a_verdict_it_cannot_ground_in_evidence() {
        let cache = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        let ccx = ccx(cache.path(), config.path()).await;
        ToolDisagreementFile {
            config_path: String::new(),
        }
        .tool_execute(
            ccx.clone(),
            &"call-file".to_string(),
            &filing("d-refuse", "the lock is held", two_positions()),
        )
        .await
        .unwrap();

        let borrowed = output_text(
            ToolDisagreementAdjudicate {
                config_path: String::new(),
            }
            .tool_execute(
                ccx.clone(),
                &"call-borrowed".to_string(),
                &args(&[
                    ("record_id", json!("d-refuse")),
                    ("current_revision", json!("sha-abc")),
                    ("resolved_position", json!("dropped")),
                    (
                        "decisive_evidence",
                        json!({"kind": "file", "reference": "src/chat/session.rs:210"}),
                    ),
                    ("rationale", json!("borrowed from the other side")),
                ]),
            )
            .await
            .unwrap(),
        );
        assert!(borrowed.contains("Not adjudicated"), "{borrowed}");
        assert!(borrowed.contains("never presented"), "{borrowed}");

        let unresolvable = output_text(
            ToolDisagreementAdjudicate {
                config_path: String::new(),
            }
            .tool_execute(
                ccx.clone(),
                &"call-open".to_string(),
                &args(&[
                    ("record_id", json!("d-refuse")),
                    ("current_revision", json!("sha-abc")),
                    ("unresolvable_reason", json!("neither side ran the case")),
                ]),
            )
            .await
            .unwrap(),
        );
        assert!(unresolvable.contains("is now open"), "{unresolvable}");
        assert!(
            unresolvable.contains("no decisive evidence"),
            "{unresolvable}"
        );

        let moved = output_text(
            ToolDisagreementAdjudicate {
                config_path: String::new(),
            }
            .tool_execute(
                ccx.clone(),
                &"call-moved".to_string(),
                &args(&[
                    ("record_id", json!("d-refuse")),
                    ("current_revision", json!("sha-moved")),
                    ("resolved_position", json!("held")),
                    (
                        "decisive_evidence",
                        json!({"kind": "file", "reference": "src/chat/session.rs:210"}),
                    ),
                    ("rationale", json!("still looks right")),
                ]),
            )
            .await
            .unwrap(),
        );
        assert!(moved.contains("Not adjudicated"), "{moved}");
        assert!(moved.contains("stale"), "{moved}");
        assert!(!moved.contains("[adjudicated]"), "{moved}");
    }
}
