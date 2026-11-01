use refact_buddy_core::autonomous_workflows::workflow_deliverable_contract;
use refact_buddy_core::operator_types::{
    EvidenceRef, NativeSourceIdentity, NativeSourceKind, ProjectIdentity, WorkerIdentity,
};
use serde::{Deserialize, Serialize};

use super::jobs::autonomous_chats::redact_and_cap_text;

pub(crate) const CANDIDATE_BINDINGS: &str = "workflow_candidate_bindings";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeCandidateBinding {
    pub schema_version: u16,
    pub run_id: String,
    pub workflow_id: String,
    pub project: ProjectIdentity,
    pub worker: WorkerIdentity,
    pub mandate_id: String,
    pub owner_epoch: u64,
    pub candidate_chat_id: String,
    pub worktree_id: String,
    pub candidate_version: String,
    pub requested_by: NativeSourceIdentity,
    pub control_id: String,
    pub admitted_at: String,
}

impl NativeCandidateBinding {
    pub(crate) async fn load(
        runtime: &super::outcome_runtime::OutcomeRuntime,
        run_id: &str,
    ) -> Result<Option<Self>, String> {
        let run_id = run_id.to_string();
        runtime
            .with_store(move |store| store.get_document::<Self>(CANDIDATE_BINDINGS, &run_id))
            .await
            .map(|stored| stored.map(|s| s.value))
            .map_err(|e| e.to_string())
    }

    pub(crate) async fn save(
        &self,
        runtime: &super::outcome_runtime::OutcomeRuntime,
    ) -> Result<(), String> {
        let value = self.clone();
        runtime
            .with_store(move |store| {
                store.put_document(CANDIDATE_BINDINGS, &value.run_id, &value, None)
            })
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

/// The deliberately small, non-executable proposal vocabulary accepted from an autonomous
/// report.  This is not an ActionSpec and cannot carry adapter arguments, authority, grants, or
/// a version chosen by the model.  The caller supplies all of those from the owning resource.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "family", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum WorkflowProposal {
    MarkdownExtension {
        kind: MarkdownProposalKind,
        name: String,
        #[serde(default)]
        description: Option<String>,
        body: String,
    },
    IsolatedCandidateCheckpoint {
        worktree_id: String,
        chat_id: String,
    },
    /// Names one memory lifecycle operation the engine itself already produced and stored.
    /// The model cannot author the operation, its targets, or its payload.
    MemoryLifecycleSelection { op_id: String },
    /// Names one already-registered catalog model for one already-known default slot.
    DefaultModelSelection {
        slot: ProposedModelSlot,
        model: String,
    },
    /// One of a fixed set of non-authority Buddy preferences.
    BuddyPreference { preference: ProposedPreference },
    /// Human-readable description text for an existing project customization file. No tool,
    /// model, permission, or confirmation field can be reached through this vocabulary.
    CustomizationDescription {
        kind: ProposedCustomizationKind,
        id: String,
        description: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MarkdownProposalKind {
    Skill,
    Command,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProposedModelSlot {
    Chat,
    ChatLight,
    ChatThinking,
    ChatBuddy,
}

impl ProposedModelSlot {
    fn native(self) -> crate::providers::operator_defaults::DefaultSlot {
        use crate::providers::operator_defaults::DefaultSlot;
        match self {
            ProposedModelSlot::Chat => DefaultSlot::Chat,
            ProposedModelSlot::ChatLight => DefaultSlot::ChatLight,
            ProposedModelSlot::ChatThinking => DefaultSlot::ChatThinking,
            ProposedModelSlot::ChatBuddy => DefaultSlot::ChatBuddy,
        }
    }
}

/// The complete set of settings an autonomous report may propose. Everything that carries
/// authority, autonomy, budget, or observation scope is deliberately absent and is additionally
/// rejected by the native Buddy settings owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(tag = "setting", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ProposedPreference {
    QuietMode { enabled: bool },
    HumorEnabled { enabled: bool },
    DailyDigestHour { hour: u8 },
}

impl ProposedPreference {
    fn change(self) -> (String, serde_json::Value) {
        match self {
            ProposedPreference::QuietMode { enabled } => {
                ("quiet_mode".to_string(), serde_json::json!(enabled))
            }
            ProposedPreference::HumorEnabled { enabled } => {
                ("humor_enabled".to_string(), serde_json::json!(enabled))
            }
            ProposedPreference::DailyDigestHour { hour } => {
                ("daily_digest_hour".to_string(), serde_json::json!(hour))
            }
        }
    }

    fn is_wellformed(&self) -> bool {
        match self {
            ProposedPreference::DailyDigestHour { hour } => *hour <= 23,
            _ => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProposedCustomizationKind {
    Modes,
    Subagents,
}

impl ProposedCustomizationKind {
    fn native(self) -> &'static str {
        match self {
            ProposedCustomizationKind::Modes => "modes",
            ProposedCustomizationKind::Subagents => "subagents",
        }
    }
}

/// Parse only one bounded JSON proposal. Model prose is otherwise retained solely as prose.
/// A fenced JSON block is accepted for readability, but malformed/extra fields are rejected.
/// Only explicit proposal families whose native owner can bind and preview a candidate may cross
/// from prose into a prepared action.
/// This instruction is deliberately declarative: the fenced object remains an untrusted proposal
/// until the engine validates, previews, attaches, and receives an explicit user grant.
///
/// `native_candidate` is the id of an isolated worktree the engine has already found registered
/// to this exact report chat. It is never minted here: when there is none, the candidate
/// vocabulary is simply not offered and the deliverable stays an honest read-only assessment.
pub(crate) fn local_proposal_protocol_instruction(
    workflow_id: &str,
    native_candidate: Option<&str>,
) -> Option<String> {
    use refact_buddy_core::autonomous_workflows::{workflow_action_family, WorkflowActionFamily};

    let preamble = "If and only if your assessment supports it, end your response with exactly one fenced JSON object and no prose inside or after that fence:\n";
    let epilogue = "\nThe fenced JSON is a proposal only: it grants no authority, attaches no adapter arguments, applies nothing, verifies nothing, and cannot execute. The engine re-validates every field against its own owner before anything becomes a reviewable action.";
    let body = match workflow_action_family(workflow_id) {
        WorkflowActionFamily::MarkdownProjectApply => concat!(
            "```json\n",
            "{\"family\":\"markdown_extension\",\"kind\":\"skill\",\"name\":\"example_skill\",\"description\":\"Optional description (max 500 chars)\",\"body\":\"Markdown body (1-2500 chars)\"}\n",
            "```\n",
            "Schema: family must be markdown_extension; kind is skill or command; name is 1-80 ASCII letters, digits, _ or -; description is optional and at most 500 chars; body is non-empty and at most 2500 chars."
        ).to_string(),
        WorkflowActionFamily::IsolatedCandidateCheckpoint => {
            // Only emitted when the caller has already found a native, run-bound isolated
            // checkout. Parsing it is still insufficient: attachment repeats that lookup.
            let worktree_id = native_candidate?;
            format!(
                "```json\n{{\"family\":\"isolated_candidate_checkpoint\",\"worktree_id\":\"{worktree_id}\",\"chat_id\":\"this_report_chat_id\"}}\n```\nA native isolated candidate is already registered to this exact report chat. Use only worktree id {worktree_id} and this report chat id. This creates no worktree, edits nothing, and does not checkpoint, merge, or land."
            )
        }
        WorkflowActionFamily::MemoryLifecycle => concat!(
            "```json\n",
            "{\"family\":\"memory_lifecycle_selection\",\"op_id\":\"existing_engine_operation_id\"}\n",
            "```\n",
            "Schema: op_id must be the id of a memory lifecycle operation the engine already recorded and that is still awaiting review. You cannot author a memory operation, its targets, or its content here; you may only point at one that already exists."
        ).to_string(),
        WorkflowActionFamily::DefaultModelSelect => concat!(
            "```json\n",
            "{\"family\":\"default_model_selection\",\"slot\":\"chat\",\"model\":\"provider/model-id\"}\n",
            "```\n",
            "Schema: slot is chat, chat_light, chat_thinking, or chat_buddy; model must be an exact provider/model id that is already enabled in this installation's catalog. The engine re-reads the current default itself; an unknown or disabled model is refused rather than proposed."
        ).to_string(),
        WorkflowActionFamily::BuddySettingsPatch => concat!(
            "```json\n",
            "{\"family\":\"buddy_preference\",\"preference\":{\"setting\":\"quiet_mode\",\"enabled\":true}}\n",
            "```\n",
            "Schema: preference.setting is quiet_mode (enabled: bool), humor_enabled (enabled: bool), or daily_digest_hour (hour: 0-23). No autonomy, permission, budget, or observation setting can be proposed."
        ).to_string(),
        WorkflowActionFamily::CustomizationFileApply => concat!(
            "```json\n",
            "{\"family\":\"customization_description\",\"kind\":\"modes\",\"id\":\"existing_id\",\"description\":\"Replacement description (max 500 chars)\"}\n",
            "```\n",
            "Schema: kind is modes or subagents; id must name a customization file that already exists in this project; description is human-readable text only. Tools, models, permissions, and confirmation settings cannot be reached through this vocabulary."
        ).to_string(),
        WorkflowActionFamily::SavedEvidenceOnly { .. } => return None,
    };
    Some(format!("{preamble}{body}{epilogue}"))
}

pub(crate) fn parse_local_proposal(workflow_id: &str, report: &str) -> Option<WorkflowProposal> {
    use refact_buddy_core::autonomous_workflows::{workflow_action_family, WorkflowActionFamily};
    // The published assessment adds its own coverage/provenance text and is capped at 8 KiB.
    // Keep the embedded structured proposal comfortably below that cap so attachment can prove
    // it came from the immutable, readable artifact rather than an in-memory report.
    if report.len() > 2_500 {
        return None;
    }
    let candidate = report.trim();
    // The protocol permits an explanatory assessment before the final fenced object, but never
    // prose after it. Do not scrape arbitrary JSON out of a report: that would make an incidental
    // example executable-looking proposal material.
    let candidate = if candidate.starts_with('{') {
        candidate
    } else if candidate.starts_with("```json\n") {
        candidate
            .strip_prefix("```json\n")
            .and_then(|text| text.strip_suffix("\n```"))?
    } else {
        let fence_start = candidate.rfind("\n```json\n")?;
        let (prose, fenced) = candidate.split_at(fence_start);
        if prose.trim().is_empty() {
            return None;
        }
        fenced
            .strip_prefix("\n```json\n")
            .and_then(|text| text.strip_suffix("\n```"))?
    };
    let proposal: WorkflowProposal = serde_json::from_str(candidate).ok()?;
    match (workflow_action_family(workflow_id), &proposal) {
        (
            WorkflowActionFamily::MarkdownProjectApply,
            WorkflowProposal::MarkdownExtension {
                name,
                description,
                body,
                ..
            },
        ) if valid_extension_name(name)
            && body.trim().len() <= 2_500
            && !body.trim().is_empty()
            && description
                .as_deref()
                .map(|s| s.len() <= 500)
                .unwrap_or(true)
            && proposal_text_is_safe(name)
            && description
                .as_deref()
                .map(proposal_text_is_safe)
                .unwrap_or(true)
            && proposal_text_is_safe(body) =>
        {
            Some(proposal)
        }
        (
            WorkflowActionFamily::IsolatedCandidateCheckpoint,
            WorkflowProposal::IsolatedCandidateCheckpoint {
                worktree_id,
                chat_id,
            },
        ) if valid_identifier(worktree_id, 128) && valid_identifier(chat_id, 128) => Some(proposal),
        (
            WorkflowActionFamily::MemoryLifecycle,
            WorkflowProposal::MemoryLifecycleSelection { op_id },
        ) if valid_operation_identifier(op_id) => Some(proposal),
        (
            WorkflowActionFamily::DefaultModelSelect,
            WorkflowProposal::DefaultModelSelection { model, .. },
        ) if valid_model_identifier(model) => Some(proposal),
        (
            WorkflowActionFamily::BuddySettingsPatch,
            WorkflowProposal::BuddyPreference { preference },
        ) if preference.is_wellformed() => Some(proposal),
        (
            WorkflowActionFamily::CustomizationFileApply,
            WorkflowProposal::CustomizationDescription {
                id, description, ..
            },
        ) if valid_customization_id(id)
            && !description.trim().is_empty()
            && description.len() <= 500
            && proposal_text_is_safe(description) =>
        {
            Some(proposal)
        }
        _ => None,
    }
}

/// Proposed content is retained as readable report text if it would need redaction or truncation,
/// but may not cross the typed draft boundary in that case.
fn proposal_text_is_safe(text: &str) -> bool {
    redact_and_cap_text(text, 2_500) == text
}

fn valid_extension_name(name: &str) -> bool {
    valid_identifier(name, 80)
}

fn valid_identifier(value: &str, max_len: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

/// Engine-minted memory operation ids are structured (`source:kind:hash`), so the separator is
/// permitted, but nothing that could traverse a path or carry prose is.
fn valid_operation_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.'))
}

fn valid_model_identifier(value: &str) -> bool {
    let mut parts = value.split('/');
    let (Some(instance), Some(id), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    value.len() <= 120 && valid_identifier_with_dots(instance) && valid_identifier_with_dots(id)
}

fn valid_identifier_with_dots(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

/// Mirrors `customization_service::validate_id`; the owner re-checks it during binding.
fn valid_customization_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
}

pub(crate) async fn attach_local_proposal(
    app: crate::app_state::AppState,
    runtime: &super::outcome_runtime::OutcomeRuntime,
    work: super::operator_store::Stored<refact_buddy_core::operator_types::WorkItem>,
    source_run_id: &str,
    source_report: &str,
    proposal: WorkflowProposal,
) -> Result<String, String> {
    use refact_buddy_core::operator_types::{
        ActionAvailability, ActionCapabilities, ActionSpec, AuthorityClass, ConfirmationClass,
        ResourceRef,
    };
    use super::resource_execution::persist_draft;
    use super::resources::extensions::{MarkdownDraft, MarkdownKind};
    let project = runtime.project();
    if work.value.project != project
        || source_run_id.trim().is_empty()
        || source_report.trim().is_empty()
    {
        return Err("proposal work belongs to another project".into());
    }
    let run = runtime
        .get_run_outcome(source_run_id.to_string())
        .await
        .map_err(|error| error.to_string())?
        .ok_or("saved workflow outcome unavailable for proposal")?;
    if run.value.project != project || run.value.work_id != work.value.id {
        return Err("proposal run does not belong to this work item".into());
    }
    let workflow_id = run
        .value
        .workflow
        .as_deref()
        .ok_or("saved workflow run has no proposal family")?;
    if parse_local_proposal(workflow_id, source_report).as_ref() != Some(&proposal) {
        return Err("source report does not contain this exact proposal".into());
    }
    let artifact_id = work
        .value
        .current_artifact_id
        .clone()
        .ok_or("proposal artifact missing")?;
    let artifact_revision = work
        .value
        .current_artifact_revision
        .ok_or("proposal revision missing")?;
    let artifact = runtime
        .get_artifact_revision(artifact_id.clone(), Some(artifact_revision))
        .await
        .map_err(|error| error.to_string())?
        .ok_or("saved proposal artifact unavailable")?;
    if artifact.value.project != project
        || !artifact
            .value
            .source_run_ids
            .iter()
            .any(|id| id == source_run_id)
        || !artifact.value.body.contains(source_report.trim())
    {
        return Err("saved proposal artifact does not match this workflow report".into());
    }
    match proposal {
        WorkflowProposal::MarkdownExtension {
            kind,
            name,
            description,
            body,
        } => {
            if !valid_extension_name(&name)
                || body.is_empty()
                || body.len() > 2_500
                || !proposal_text_is_safe(&name)
                || !description
                    .as_deref()
                    .map(proposal_text_is_safe)
                    .unwrap_or(true)
                || !proposal_text_is_safe(&body)
            {
                return Err("local proposal invalid".into());
            }
            let family = match kind {
                MarkdownProposalKind::Skill => "skills",
                MarkdownProposalKind::Command => "commands",
            };
            let relative = if family == "skills" {
                format!("skills/{name}/SKILL.md")
            } else {
                format!("commands/{name}.md")
            };
            let before = crate::ext::operator_service::snapshot(&project, &relative)
                .map_err(|e| format!("{e:?}"))?;
            let version = crate::ext::operator_service::version(&before);
            let draft = MarkdownDraft {
                project: project.clone(),
                kind: match kind {
                    MarkdownProposalKind::Skill => MarkdownKind::Skill,
                    MarkdownProposalKind::Command => MarkdownKind::Command,
                },
                name: name.clone(),
                description,
                body,
            };
            let reference = proposal_reference(
                &artifact_id,
                artifact_revision,
                &artifact.value.body,
                &draft,
            )?;
            persist_draft(runtime, reference.clone(), draft)
                .await
                .map_err(|e| format!("{e:?}"))?;
            let spec = ActionSpec {
                schema_version: 1,
                action_id: reference.clone(),
                kind: "markdown_project_apply".into(),
                adapter: "extensions".into(),
                project: project.clone(),
                resource: ResourceRef {
                    resource_kind: "native_markdown".into(),
                    resource_id: format!("{family}/{name}"),
                    version: version.clone(),
                },
                expected_version: version,
                parameter_ref: Some(reference),
                availability: ActionAvailability::Available,
                confirmation: ConfirmationClass::UserConfirmationRequired,
                authority_class: AuthorityClass::Mutating,
                capabilities: ActionCapabilities {
                    can_preview: true,
                    can_execute: true,
                    can_verify: true,
                    can_recover: true,
                    can_cancel: false,
                },
            };
            attach_prepared_proposal_action(
                app,
                runtime,
                work,
                artifact_id,
                artifact_revision,
                spec,
            )
            .await
        }
        WorkflowProposal::IsolatedCandidateCheckpoint {
            worktree_id,
            chat_id,
        } => {
            // An engine-minted binding (native supervision admission) is the production path:
            // the candidate belongs to the supervised worker chat, not to this report chat.
            // Without a binding the only remaining origin is a report chat that itself owns
            // the isolated checkout; that path never links a foreign chat.
            let candidate = match NativeCandidateBinding::load(runtime, source_run_id).await? {
                Some(binding) => {
                    if binding.run_id != source_run_id || binding.workflow_id != workflow_id {
                        return Err("candidate binding does not belong to this workflow run".into());
                    }
                    bound_candidate(app.clone(), runtime, &binding, &worktree_id, &chat_id).await?
                }
                None => {
                    if chat_id != source_run_id {
                        return Err("candidate chat is not this report run".into());
                    }
                    native_bound_candidate(app.clone(), runtime, source_run_id, &worktree_id)
                        .await?
                }
            };
            let draft = super::resources::project_code::ProjectCodeDraft {
                project: project.clone(),
                worktree_id: candidate.id.clone(),
                chat_id,
            };
            let version = candidate.version;
            let reference = proposal_reference(
                &artifact_id,
                artifact_revision,
                &artifact.value.body,
                &draft,
            )?;
            persist_draft(runtime, reference.clone(), draft)
                .await
                .map_err(|e| format!("{e:?}"))?;
            let spec = ActionSpec {
                schema_version: 1,
                action_id: reference.clone(),
                kind: "checkpoint_isolated_candidate".into(),
                adapter: "project_code".into(),
                project: project.clone(),
                resource: ResourceRef {
                    resource_kind: "worktree".into(),
                    resource_id: candidate.id,
                    version: version.clone(),
                },
                expected_version: version,
                parameter_ref: Some(reference),
                availability: ActionAvailability::Available,
                confirmation: ConfirmationClass::UserConfirmationRequired,
                authority_class: AuthorityClass::Mutating,
                capabilities: ActionCapabilities {
                    can_preview: true,
                    can_execute: true,
                    can_verify: true,
                    can_recover: false,
                    can_cancel: false,
                },
            };
            attach_prepared_proposal_action(
                app,
                runtime,
                work,
                artifact_id,
                artifact_revision,
                spec,
            )
            .await
        }
        WorkflowProposal::MemoryLifecycleSelection { op_id } => {
            // The operation itself is engine-produced: the model may only point at one that is
            // already stored and still awaiting review. Nothing here authors memory content.
            let operation = engine_produced_memory_operation(app.clone(), &project, &op_id).await?;
            let expected_digest = memory_operation_digest(&operation)?;
            if !run.value.evidence.iter().any(|evidence| {
                evidence.source.source_kind == NativeSourceKind::Workflow
                    && evidence.source.source_id == format!("memory-lifecycle:{op_id}")
                    && evidence.digest.as_deref() == Some(expected_digest.as_str())
            }) {
                return Err("memory operation was not retained as evidence for this run".into());
            }
            let resource_id = "memory".to_string();
            let draft = super::resources::memory::MemoryDraft {
                project: project.clone(),
                resource_id: resource_id.clone(),
                operation,
            };
            let version = memory_current_version(app.clone(), &project, &resource_id).await?;
            let reference = proposal_reference(
                &artifact_id,
                artifact_revision,
                &artifact.value.body,
                &draft,
            )?;
            persist_draft(runtime, reference.clone(), draft)
                .await
                .map_err(|e| format!("{e:?}"))?;
            let spec = ActionSpec {
                schema_version: 1,
                action_id: reference.clone(),
                kind: "memory_lifecycle".into(),
                adapter: "memory".into(),
                project: project.clone(),
                resource: ResourceRef {
                    resource_kind: "memory".into(),
                    resource_id,
                    version: version.clone(),
                },
                expected_version: version,
                parameter_ref: Some(reference),
                availability: ActionAvailability::Available,
                confirmation: ConfirmationClass::UserConfirmationRequired,
                authority_class: AuthorityClass::Mutating,
                capabilities: ActionCapabilities {
                    can_preview: true,
                    can_execute: true,
                    can_verify: true,
                    can_recover: false,
                    can_cancel: false,
                },
            };
            attach_prepared_proposal_action(
                app,
                runtime,
                work,
                artifact_id,
                artifact_revision,
                spec,
            )
            .await
        }
        WorkflowProposal::DefaultModelSelection { slot, model } => {
            use crate::providers::operator_defaults as owner;
            use refact_core::provider_types::DefaultsScope;
            let scope = DefaultsScope::Project;
            // The catalog owner decides whether this model exists and is enabled here; an
            // unreviewed or disabled id is refused rather than turned into a proposal.
            owner::validate_candidate(&app, &model).await?;
            let root = owner::scope_root(&app, &project, scope).await?;
            let before = owner::defaults_snapshot(&root, scope).await?;
            let version = super::resources::settings::version(&before);
            let expected_selected = current_default_model(&before, slot.native());
            if expected_selected.as_deref() == Some(model.as_str()) {
                return Err("proposed model is already the current default".into());
            }
            // Confirm the owner can actually produce the exact patch before anything is stored.
            owner::patch(&before, slot.native(), &expected_selected, &model)?;
            let draft = super::resources::default_models::DefaultModelDraft {
                project: project.clone(),
                scope,
                slot: slot.native(),
                expected_selected,
                selected_model: model,
            };
            let reference = proposal_reference(
                &artifact_id,
                artifact_revision,
                &artifact.value.body,
                &draft,
            )?;
            persist_draft(runtime, reference.clone(), draft)
                .await
                .map_err(|e| format!("{e:?}"))?;
            let spec = ActionSpec {
                schema_version: 1,
                action_id: reference.clone(),
                kind: "default_model_select".into(),
                adapter: "default_models".into(),
                project: project.clone(),
                resource: ResourceRef {
                    resource_kind: "default_models".into(),
                    resource_id: "project/default_models".into(),
                    version: version.clone(),
                },
                expected_version: version,
                parameter_ref: Some(reference),
                availability: ActionAvailability::Available,
                confirmation: ConfirmationClass::UserConfirmationRequired,
                authority_class: AuthorityClass::Mutating,
                capabilities: ActionCapabilities {
                    can_preview: true,
                    can_execute: true,
                    can_verify: true,
                    can_recover: true,
                    can_cancel: false,
                },
            };
            attach_prepared_proposal_action(
                app,
                runtime,
                work,
                artifact_id,
                artifact_revision,
                spec,
            )
            .await
        }
        WorkflowProposal::BuddyPreference { preference } => {
            if !preference.is_wellformed() {
                return Err("proposed preference is out of range".into());
            }
            let root = std::path::PathBuf::from(&project.canonical_root);
            let before = crate::buddy::settings::settings_snapshot(&root).await?;
            let version = super::resources::settings::version(&before);
            let (key, value) = preference.change();
            let draft = super::resources::settings::BuddySettingsDraft {
                project: project.clone(),
                changes: std::collections::BTreeMap::from([(key, value)]),
            };
            let reference = proposal_reference(
                &artifact_id,
                artifact_revision,
                &artifact.value.body,
                &draft,
            )?;
            persist_draft(runtime, reference.clone(), draft)
                .await
                .map_err(|e| format!("{e:?}"))?;
            let spec = ActionSpec {
                schema_version: 1,
                action_id: reference.clone(),
                kind: "buddy_settings_patch".into(),
                adapter: "buddy_settings".into(),
                project: project.clone(),
                resource: ResourceRef {
                    resource_kind: "settings".into(),
                    resource_id: "project/buddy".into(),
                    version: version.clone(),
                },
                expected_version: version,
                parameter_ref: Some(reference),
                availability: ActionAvailability::Available,
                confirmation: ConfirmationClass::UserConfirmationRequired,
                authority_class: AuthorityClass::Mutating,
                capabilities: ActionCapabilities {
                    can_preview: true,
                    can_execute: true,
                    can_verify: true,
                    can_recover: true,
                    can_cancel: false,
                },
            };
            attach_prepared_proposal_action(
                app,
                runtime,
                work,
                artifact_id,
                artifact_revision,
                spec,
            )
            .await
        }
        WorkflowProposal::CustomizationDescription {
            kind,
            id,
            description,
        } => {
            if !valid_customization_id(&id)
                || description.trim().is_empty()
                || description.len() > 500
                || !proposal_text_is_safe(&description)
            {
                return Err("proposed customization description invalid".into());
            }
            let native_kind = kind.native();
            let relative = format!("{native_kind}/{id}.yaml");
            // An absent file has no current version to bind, so nothing is created here.
            let before = crate::ext::operator_service::snapshot(&project, &relative)
                .map_err(|e| format!("{e:?}"))?
                .ok_or("proposed customization file does not exist in this project")?;
            let version = crate::yaml_configs::customization_service::version(&before);
            let draft = super::resources::customization::CustomizationDraft {
                kind: native_kind.into(),
                id: id.clone(),
                // Only the description field is mentioned; the owner merges it over the stored
                // document and rejects any widening of tools, models, or permissions.
                config: serde_json::json!({ "id": id, "description": description }),
            };
            let reference = proposal_reference(
                &artifact_id,
                artifact_revision,
                &artifact.value.body,
                &draft,
            )?;
            persist_draft(runtime, reference.clone(), draft)
                .await
                .map_err(|e| format!("{e:?}"))?;
            let spec = ActionSpec {
                schema_version: 1,
                action_id: reference.clone(),
                kind: super::resources::customization::ACTION_KIND.into(),
                adapter: super::resources::customization::ADAPTER_ID.into(),
                project: project.clone(),
                resource: ResourceRef {
                    resource_kind: super::resources::customization::RESOURCE_KIND.into(),
                    resource_id: format!("{native_kind}/{id}"),
                    version: version.clone(),
                },
                expected_version: version,
                parameter_ref: Some(reference),
                availability: ActionAvailability::Available,
                confirmation: ConfirmationClass::UserConfirmationRequired,
                authority_class: AuthorityClass::Mutating,
                capabilities: ActionCapabilities {
                    can_preview: true,
                    can_execute: true,
                    can_verify: true,
                    can_recover: true,
                    can_cancel: false,
                },
            };
            attach_prepared_proposal_action(
                app,
                runtime,
                work,
                artifact_id,
                artifact_revision,
                spec,
            )
            .await
        }
    }
}

pub(crate) fn memory_operation_digest(
    operation: &crate::buddy::memory_lifecycle::MemoryLifecycleOp,
) -> Result<String, String> {
    use sha2::Digest;
    let bytes = serde_json::to_vec(&operation.clone().normalized())
        .map_err(|_| "memory operation serialization failed")?;
    Ok(hex::encode(sha2::Sha256::digest(bytes)))
}

/// Reads the exact memory lifecycle operation the engine already recorded for this project.
/// The model supplies only an id; every field of the returned operation is engine-produced.
async fn engine_produced_memory_operation(
    app: crate::app_state::AppState,
    project: &refact_buddy_core::operator_types::ProjectIdentity,
    op_id: &str,
) -> Result<crate::buddy::memory_lifecycle::MemoryLifecycleOp, String> {
    use crate::buddy::memory_lifecycle::{MemoryOpStatus, MemoryOpType};
    let root = std::path::PathBuf::from(&project.canonical_root);
    if !crate::files_correction::get_project_dirs(app.gcx.clone())
        .await
        .contains(&root)
    {
        return Err("memory proposal project is not an open workspace folder".into());
    }
    let state = crate::buddy::storage::load_memory_ops(&root).await;
    let operation = state
        .ops
        .into_iter()
        .find(|op| op.op_id == op_id)
        .ok_or("proposed memory operation was not produced by this engine")?;
    if !matches!(
        operation.status,
        MemoryOpStatus::Pending | MemoryOpStatus::Approved
    ) {
        return Err("proposed memory operation is already finalized".into());
    }
    if !matches!(
        operation.op_type,
        MemoryOpType::CreateMemory | MemoryOpType::Archive | MemoryOpType::MergeArchive
    ) {
        return Err("proposed memory operation is not a reviewable lifecycle change".into());
    }
    Ok(operation.normalized())
}

/// Read-only current-version lookup through the native memory owner itself. The adapter is
/// constructed only to read; its draft and material providers are deliberately inert.
async fn memory_current_version(
    app: crate::app_state::AppState,
    project: &refact_buddy_core::operator_types::ProjectIdentity,
    resource_id: &str,
) -> Result<String, String> {
    use super::resources::{DraftRegistry, RecoveryMaterialStore, ResourceError};
    struct NoDrafts;
    impl DraftRegistry<super::resources::memory::MemoryDraft> for NoDrafts {
        fn get(&self, _: &str) -> Result<super::resources::memory::MemoryDraft, ResourceError> {
            Err(ResourceError::ParameterReferenceMissing)
        }
    }
    struct NoMaterials;
    impl RecoveryMaterialStore for NoMaterials {
        fn put(&self, _: &[u8]) -> Result<String, ResourceError> {
            Err(ResourceError::MaterialUnavailable)
        }
        fn get(&self, _: &str) -> Result<Vec<u8>, ResourceError> {
            Err(ResourceError::MaterialUnavailable)
        }
    }
    super::resources::memory::MemoryAdapter::new(
        app,
        project.clone(),
        std::sync::Arc::new(NoDrafts),
        std::sync::Arc::new(NoMaterials),
    )
    .version(resource_id)
    .await
    .map_err(|e| format!("{e:?}"))
}

/// Current model recorded for one default slot, read from the owner's own document bytes.
fn current_default_model(
    bytes: &Option<Vec<u8>>,
    slot: crate::providers::operator_defaults::DefaultSlot,
) -> Option<String> {
    let value: serde_yaml::Value = serde_yaml::from_slice(bytes.as_ref()?).ok()?;
    value
        .get(slot.key())?
        .get("model")?
        .as_str()
        .map(str::to_string)
}

struct NativeCandidate {
    id: String,
    version: String,
}

async fn bound_candidate(
    app: crate::app_state::AppState,
    runtime: &super::outcome_runtime::OutcomeRuntime,
    binding: &NativeCandidateBinding,
    proposed_worktree_id: &str,
    proposed_chat_id: &str,
) -> Result<NativeCandidate, String> {
    if binding.worktree_id != proposed_worktree_id || binding.candidate_chat_id != proposed_chat_id
    {
        return Err("proposal does not name the engine-bound candidate".into());
    }
    super::supervision::validate_candidate_assessment(&app, runtime, binding).await?;
    Ok(NativeCandidate {
        id: binding.worktree_id.clone(),
        version: binding.candidate_version.clone(),
    })
}

async fn native_bound_candidate(
    app: crate::app_state::AppState,
    runtime: &super::outcome_runtime::OutcomeRuntime,
    source_run_id: &str,
    proposed_worktree_id: &str,
) -> Result<NativeCandidate, String> {
    let thread_worktree =
        if let Some(session) = app.chat.sessions.read().await.get(source_run_id).cloned() {
            let session = session.lock().await;
            (session.chat_id == source_run_id)
                .then(|| session.thread.worktree.clone())
                .flatten()
        } else {
            crate::chat::trajectories::load_trajectory_for_chat(app.gcx.clone(), source_run_id)
                .await
                .and_then(|loaded| {
                    (loaded.thread.id == source_run_id)
                        .then_some(loaded.thread.worktree)
                        .flatten()
                })
        }
        .ok_or("native report chat has no bound isolated worktree")?;
    if thread_worktree.id != proposed_worktree_id
        || thread_worktree.source_workspace_root.to_string_lossy()
            != runtime.project().canonical_root
    {
        return Err("proposed worktree is not the report chat's project-bound candidate".into());
    }
    let service = crate::worktrees::service::WorktreeService::new_async(
        app.paths.cache_dir.clone(),
        runtime.project().canonical_root.clone().into(),
    )
    .await
    .map_err(|e| e.to_string())?;
    let view = service
        .get_worktree(proposed_worktree_id)
        .await
        .map_err(|e| e.to_string())?;
    if view.meta.id != thread_worktree.id
        || view.meta.root != thread_worktree.root
        || view.meta.source_workspace_root != thread_worktree.source_workspace_root
        || !view
            .references
            .iter()
            .any(|reference| reference.chat_id.as_deref() == Some(source_run_id))
    {
        return Err("registered worktree no longer belongs exactly to this report chat".into());
    }
    let version = super::resources::project_code::registered_candidate_version(
        &service,
        proposed_worktree_id,
    )
    .await
    .map_err(|e| format!("{e:?}"))?;
    Ok(NativeCandidate {
        id: proposed_worktree_id.to_string(),
        version,
    })
}

fn proposal_reference<T: serde::Serialize>(
    artifact_id: &str,
    artifact_revision: u32,
    body: &str,
    draft: &T,
) -> Result<String, String> {
    use sha2::Digest;
    let mut fingerprint = sha2::Sha256::new();
    fingerprint.update(artifact_id.as_bytes());
    fingerprint.update(artifact_revision.to_le_bytes());
    fingerprint.update(body.as_bytes());
    fingerprint
        .update(serde_json::to_vec(draft).map_err(|_| "proposal draft serialization failed")?);
    Ok(format!(
        "workflow-proposal:{}",
        hex::encode(fingerprint.finalize())
    ))
}

async fn attach_prepared_proposal_action(
    app: crate::app_state::AppState,
    runtime: &super::outcome_runtime::OutcomeRuntime,
    work: super::operator_store::Stored<refact_buddy_core::operator_types::WorkItem>,
    artifact_id: String,
    artifact_revision: u32,
    spec: refact_buddy_core::operator_types::ActionSpec,
) -> Result<String, String> {
    use super::action_execution::{action_spec_hash, AsyncActionRepository, PreparedAction};
    use super::resource_execution::{bind_prepared_draft, registered_catalog, RuntimeActionRepository};
    let catalog = registered_catalog(app, runtime, &spec)
        .await
        .map_err(|e| format!("{e:?}"))?;
    catalog.preview(&spec).await.map_err(|e| format!("{e:?}"))?;
    let prepared = PreparedAction {
        spec_hash: action_spec_hash(&spec, &spec.parameter_ref),
        bound_parameter_ref: spec.parameter_ref.clone(),
        spec,
    };
    bind_prepared_draft(runtime, &prepared)
        .await
        .map_err(|e| format!("{e:?}"))?;
    RuntimeActionRepository(runtime.clone())
        .prepare(prepared.clone())
        .await
        .map_err(|_| "proposal preparation failed")?;
    let hash = prepared.spec_hash;
    let origin = super::desk_actions::WorkActionOrigin {
        schema_version: 1,
        project: runtime.project(),
        work_id: work.value.id,
        work_revision: work.revision,
        artifact_id,
        artifact_revision,
        spec_hash: hash.clone(),
    };
    runtime
        .with_store(move |store| super::desk_actions::attach_resource_action(store, origin))
        .await
        .map_err(|e| e.to_string())?;
    Ok(hash)
}

/// Body of a model-written assessment for a workflow deliverable.
///
/// `observed` is the engine's own typed observation of the source that fed the model, when the
/// workflow has one. Its scope and completion are printed verbatim under coverage so the reader
/// sees what was actually read, independent of whatever the model wrote about it. The model's
/// prose never changes the recorded terminal: a successful assessment is always `Inconclusive`.
pub(crate) fn assessment_body(
    workflow_id: &str,
    source_evidence: &str,
    report: &str,
    observed: Option<&SourceCheck>,
) -> String {
    let Some(contract) = workflow_deliverable_contract(workflow_id) else {
        return redact_and_cap_text(report, 24_000);
    };
    let evidence = redact_and_cap_text(source_evidence, 8_000);
    let report = redact_and_cap_text(report, 24_000);
    let mut coverage = contract.coverage_limit.to_string();
    match observed {
        Some(check) => {
            coverage.push_str("\nObserved scope: ");
            coverage.push_str(&check.coverage());
            coverage.push_str("\nSource completion: ");
            coverage.push_str(&check.completion_summary());
        }
        None => coverage.push_str(
            "\nSource completion: not observed by the engine for this workflow; the signal below was assembled without a typed source check.",
        ),
    }
    format!(
        "## {} assessment\n\n### Coverage and limitations\n{}\n\n### Retained source signal\n{}\n\n### Assessment / local proposal\n{}\n\n### Next decision\n{}\n\nRead this report in Buddy Desk. Missing or malformed evidence does not support a conclusion.",
        super::outcome_runtime::workflow_display_title(workflow_id),
        coverage,
        if evidence.trim().is_empty() { "No source signal retained." } else { &evidence },
        if report.trim().is_empty() { "No written assessment retained." } else { &report },
        contract.local_proposal,
    )
}

/// How much of the stated source scope was actually read.
///
/// This is deliberately not a boolean: "the scan failed", "this project has no such source",
/// and "the source was read and had nothing in it" are three different operator-visible
/// answers, and only the last one may ever become `NoChange`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SourceCompletion {
    /// Every input named by `scope` was read to completion.
    Complete,
    /// The source does not exist in this project; nothing was checked and nothing is implied.
    NotApplicable { reason: String },
    /// Reading the source errored. Coverage is unknown; absence of findings means nothing.
    Failed { error: String },
    /// Only part of the stated scope was read (a cap, limit, or truncation was hit).
    Partial { uncovered: String },
}

impl SourceCompletion {
    pub(crate) fn failure_detail(&self) -> Option<&str> {
        match self {
            SourceCompletion::Failed { error } => Some(error.as_str()),
            SourceCompletion::NotApplicable { reason } => Some(reason.as_str()),
            SourceCompletion::Partial { uncovered } => Some(uncovered.as_str()),
            SourceCompletion::Complete => None,
        }
    }
}

/// A grounded observation of one native source, before any model has seen it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SourceCheck {
    /// Exactly what was read, in operator-readable terms.
    pub scope: String,
    /// Stable identity of the observed inputs; identical inputs must produce an identical value.
    pub input_fingerprint: String,
    pub evidence: Vec<EvidenceRef>,
    pub completion: SourceCompletion,
    pub findings: Vec<String>,
}

impl SourceCheck {
    /// Evidence describing a native check that the engine itself performed.
    pub(crate) fn check_evidence(
        source_id: &str,
        fingerprint: &str,
        locator: &str,
        observed_at: String,
    ) -> EvidenceRef {
        EvidenceRef {
            evidence_id: format!("{source_id}:{fingerprint}"),
            source: NativeSourceIdentity {
                source_kind: NativeSourceKind::Check,
                source_id: source_id.to_string(),
                source_version: fingerprint.to_string(),
            },
            locator: locator.to_string(),
            observed_at,
            digest: None,
        }
    }

    pub(crate) fn coverage(&self) -> String {
        redact_and_cap_text(&self.scope, 2_000)
    }

    /// One operator-readable line stating how much of the scope was read.
    pub(crate) fn completion_summary(&self) -> String {
        match &self.completion {
            SourceCompletion::Complete => "complete; the whole stated scope was read".to_string(),
            SourceCompletion::NotApplicable { reason } => format!(
                "not applicable; nothing was checked ({})",
                redact_and_cap_text(reason, 500)
            ),
            SourceCompletion::Failed { error } => format!(
                "failed; coverage unknown ({})",
                redact_and_cap_text(error, 500)
            ),
            SourceCompletion::Partial { uncovered } => format!(
                "partial; not covered: {}",
                redact_and_cap_text(uncovered, 500)
            ),
        }
    }

    /// Whether the observation carries findings a model may assess.
    ///
    /// Only findings from a scope that was actually read (fully or partially) are handed to a
    /// model. Failed and absent sources are published as typed outcomes without a model run, so
    /// an LLM can never turn "could not read" into prose about the source.
    pub(crate) fn needs_assessment(&self) -> bool {
        self.is_grounded()
            && !self.findings.is_empty()
            && matches!(
                self.completion,
                SourceCompletion::Complete | SourceCompletion::Partial { .. }
            )
    }

    fn evidence_is_wellformed(&self) -> bool {
        !self.evidence.is_empty()
            && self.evidence.iter().all(|item| {
                !item.evidence_id.trim().is_empty()
                    && !item.source.source_id.trim().is_empty()
                    && !item.source.source_version.trim().is_empty()
                    && !item.locator.trim().is_empty()
                    && chrono::DateTime::parse_from_rfc3339(&item.observed_at).is_ok()
            })
    }

    /// Structural validity of the observation itself, independent of what it found.
    fn is_grounded(&self) -> bool {
        !self.scope.trim().is_empty()
            && !self.input_fingerprint.trim().is_empty()
            && self.evidence_is_wellformed()
            && self.findings.iter().all(|finding| !finding.trim().is_empty())
            // "Nothing was checked" and "here is what was found" cannot both be true.
            && !(matches!(self.completion, SourceCompletion::NotApplicable { .. })
                && !self.findings.is_empty())
    }

    pub(crate) fn terminal(&self) -> super::outcome_runtime::WorkflowTerminal {
        use super::outcome_runtime::WorkflowTerminal;
        if !self.is_grounded() {
            return WorkflowTerminal::Inconclusive;
        }
        match &self.completion {
            SourceCompletion::Failed { .. } => WorkflowTerminal::Failed {
                failure_class: Some("source_read_failed".to_string()),
            },
            SourceCompletion::NotApplicable { .. } => WorkflowTerminal::Blocked {
                failure_class: Some("source_absent".to_string()),
            },
            SourceCompletion::Partial { .. } => WorkflowTerminal::Partial {
                failure_class: Some("partial_source_coverage".to_string()),
            },
            SourceCompletion::Complete if self.findings.is_empty() => WorkflowTerminal::NoChange,
            SourceCompletion::Complete => WorkflowTerminal::Completed,
        }
    }

    fn conclusion(&self) -> String {
        if !self.is_grounded() {
            return "Source check incomplete or evidence invalid; no conclusion.".to_string();
        }
        match &self.completion {
            SourceCompletion::Failed { error } => {
                let mut text = format!(
                    "The source could not be read: {}\nCoverage is unknown.",
                    redact_and_cap_text(error, 1_000)
                );
                if !self.findings.is_empty() {
                    text.push_str("\nFindings from the part that was read before the failure (not assessed, coverage unknown):\n");
                    text.push_str(&self.findings.join("\n"));
                }
                text
            }
            SourceCompletion::NotApplicable { reason } => format!(
                "This source is not present in this project: {}",
                redact_and_cap_text(reason, 1_000)
            ),
            SourceCompletion::Partial { uncovered } => {
                let mut text = format!(
                    "Only part of the stated scope was read. Not covered: {}",
                    redact_and_cap_text(uncovered, 1_000)
                );
                if self.findings.is_empty() {
                    text.push_str("\nNo findings inside the covered part. The uncovered part was not checked.");
                } else {
                    text.push_str("\nFindings inside the covered part:\n");
                    text.push_str(&self.findings.join("\n"));
                }
                text
            }
            SourceCompletion::Complete if self.findings.is_empty() => {
                "No findings within the stated source-check scope, and the whole stated scope was read.".to_string()
            }
            SourceCompletion::Complete => self.findings.join("\n"),
        }
    }

    pub(crate) fn into_report(self, workflow_id: &str) -> super::workflows::WorkflowReport {
        let terminal = self.terminal();
        let conclusion = self.conclusion();
        let coverage = self.coverage();
        let title = super::outcome_runtime::workflow_result_title(
            &super::outcome_runtime::workflow_display_title(workflow_id),
            &terminal,
        );
        super::workflows::WorkflowReport {
            title,
            body: format!(
                "Coverage: {}\n\n{}\n\n### Next action\n{}\n\nFindings here are the engine's own observation, not a model claim.",
                coverage,
                redact_and_cap_text(&conclusion, 24_000),
                workflow_next_action(workflow_id).describe(),
            ),
            terminal,
            evidence: self.evidence,
        }
    }
}

/// The concrete next step a reader can take for a workflow's deliverable.
///
/// A `RegisteredResource` names an adapter/kind pair that actually exists in
/// `resource_execution::registered_catalog`. Naming one here does NOT attach or authorize
/// anything: attachment still needs a typed draft bound to a real resource version, and
/// execution still needs an explicit user grant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkflowNextAction {
    /// A corrective change exists as a registered resource action for this category.
    RegisteredResource {
        adapter: &'static str,
        kind: &'static str,
        /// What the reader must still decide before a draft can be bound.
        needs_from_reader: &'static str,
    },
    /// No corrective resource action exists for this category; the deliverable is read-only.
    ReadOnlyAssessment { why: &'static str },
}

impl WorkflowNextAction {
    pub(crate) fn describe(&self) -> String {
        match self {
            WorkflowNextAction::RegisteredResource {
                adapter,
                kind,
                needs_from_reader,
            } => format!(
                "Review {} after {needs_from_reader}. Nothing is applied until you approve the exact scope.",
                resource_action_label(adapter, kind)
            ),
            WorkflowNextAction::ReadOnlyAssessment { why } => format!(
                "No automatic fix for this: {why}. Act on it yourself; this report is read-only."
            ),
        }
    }
}

fn resource_action_label(adapter: &str, kind: &str) -> &'static str {
    match (adapter, kind) {
        ("memory", "memory_lifecycle") => "the proposed memory changes",
        ("default_models", "default_model_select") => "the proposed default model change",
        ("buddy_settings", "buddy_settings_patch") => "the proposed Buddy preference change",
        ("extensions", "markdown_project_apply") => "the proposed skill or command draft",
        ("project_customization_file", "customization_file_apply") => {
            "the proposed project setup change"
        }
        ("project_code", "checkpoint_isolated_candidate") => "the reviewed code change",
        _ => "the proposed resource change",
    }
}

/// Maps each registered workflow to its real next-action path.
///
/// Every `RegisteredResource` pair below is asserted against the live
/// `registered_catalog` arms in `resource_execution.rs` by
/// `declared_resource_actions_exist_in_registered_catalog`.
pub(crate) fn workflow_next_action(workflow_id: &str) -> WorkflowNextAction {
    use refact_buddy_core::autonomous_workflows::*;
    use WorkflowNextAction::*;
    match workflow_id {
        MEMORY_GARDENER_WORKFLOW_ID | KNOWLEDGE_CONFLICT_WORKFLOW_ID => RegisteredResource {
            adapter: "memory",
            kind: "memory_lifecycle",
            needs_from_reader: "choosing which memory files to archive or merge",
        },
        MODEL_COST_OPTIMIZER_WORKFLOW_ID => RegisteredResource {
            adapter: "default_models",
            kind: "default_model_select",
            needs_from_reader: "choosing a replacement model after weighing cost and quality",
        },
        BEHAVIOR_LEARNER_WORKFLOW_ID | USER_HABIT_COACH_WORKFLOW_ID => RegisteredResource {
            adapter: "buddy_settings",
            kind: "buddy_settings_patch",
            needs_from_reader: "confirming which observed preference to save",
        },
        BUDDY_SKILL_AUTHOR_WORKFLOW_ID => RegisteredResource {
            adapter: "extensions",
            kind: "markdown_project_apply",
            needs_from_reader: "reviewing and approving the proposed skill or command text",
        },
        SETUP_COACH_WORKFLOW_ID | BUDDY_ONBOARDING_WORKFLOW_ID => RegisteredResource {
            adapter: "project_customization_file",
            kind: "customization_file_apply",
            needs_from_reader: "choosing the exact setup values for this project",
        },
        ERROR_DETECTIVE_WORKFLOW_ID => ReadOnlyAssessment {
            why: "a diagnostic cluster names no single resource to change",
        },
        REFACT_SELF_CRITIC_WORKFLOW_ID
        | REFACT_COMPILE_SNIFFER_WORKFLOW_ID
        | BUDDY_REFACTOR_HUNTER_WORKFLOW_ID
        | BUDDY_TEST_COVERAGE_WATCHER_WORKFLOW_ID
        | DOCS_GARDENER_WORKFLOW_ID
        | ARCHITECTURE_DRIFT_WORKFLOW_ID
        | SECURITY_WHISPERER_WORKFLOW_ID
        | DEPENDENCY_RADAR_WORKFLOW_ID => RegisteredResource {
            adapter: "project_code",
            kind: "checkpoint_isolated_candidate",
            needs_from_reader: "opening the existing isolated worktree owned by this report and reviewing its current changes",
        },
        BUDDY_DAILY_DIGEST_WORKFLOW_ID
        | BUDDY_FRIDAY_RETRO_WORKFLOW_ID
        | BUDDY_IDLE_SUGGESTER_WORKFLOW_ID
        | BUDDY_PR_ISSUE_MATCHMAKER_WORKFLOW_ID => ReadOnlyAssessment {
            why: "the deliverable is a summary or candidate list, not a change to a resource",
        },
        _ => ReadOnlyAssessment {
            why: "this workflow is not in the registered deliverable contract",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buddy::outcome_runtime::WorkflowTerminal;
    use refact_buddy_core::autonomous_workflows::*;

    struct ProposalFixture {
        _temp: tempfile::TempDir,
        root: std::path::PathBuf,
        app: crate::app_state::AppState,
        runtime: crate::buddy::outcome_runtime::OutcomeRuntime,
    }

    impl ProposalFixture {
        async fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("project");
            std::fs::create_dir(&root).unwrap();
            let root = std::fs::canonicalize(root).unwrap();
            let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
                temp.path().join("cache"),
                temp.path().join("config"),
            )
            .await;
            let app = crate::app_state::AppState::from_gcx(gcx).await;
            *app.workspace
                .documents_state
                .workspace_folders
                .lock()
                .unwrap() = vec![root.clone()];
            let runtime = crate::buddy::outcome_runtime::OutcomeRuntime::open(
                &root,
                refact_buddy_core::operator_types::ProjectIdentity {
                    project_id: "proposal-fixture".into(),
                    canonical_root: root.to_string_lossy().into_owned(),
                },
                refact_buddy_core::operator_types::WorkerIdentity {
                    worker_id: "worker".into(),
                    engine_instance_id: "engine".into(),
                    epoch: 1,
                },
            )
            .unwrap();
            Self {
                _temp: temp,
                root,
                app,
                runtime,
            }
        }

        async fn publish(
            &self,
            workflow: &str,
            run: &str,
            report: &str,
            evidence: Vec<EvidenceRef>,
        ) -> crate::buddy::operator_store::Stored<refact_buddy_core::operator_types::WorkItem>
        {
            let mut start = self
                .runtime
                .start_workflow(workflow, "2026-01-01T00:00:00Z".into());
            start.run_id = run.into();
            start.input_fingerprint = run.into();
            start.coverage = "scratch native source".into();
            self.runtime
                .publish_assessment(start, "Native proposal".into(), report.into(), evidence)
                .await
                .unwrap();
            let outcome = self
                .runtime
                .get_run_outcome(run.into())
                .await
                .unwrap()
                .unwrap();
            self.runtime
                .get_work_item(outcome.value.work_id)
                .await
                .unwrap()
                .unwrap()
        }

        async fn attach(
            &self,
            workflow: &str,
            run: &str,
            report: &str,
            work: crate::buddy::operator_store::Stored<refact_buddy_core::operator_types::WorkItem>,
        ) -> Result<String, String> {
            attach_local_proposal(
                self.app.clone(),
                &self.runtime,
                work,
                run,
                report,
                parse_local_proposal(workflow, report).unwrap(),
            )
            .await
        }

        async fn prepared(
            &self,
            hash: &str,
            adapter: &str,
        ) -> crate::buddy::action_execution::PreparedAction {
            let prepared =
                crate::buddy::resource_execution::load_prepared(&self.runtime, hash.into())
                    .await
                    .unwrap();
            assert_eq!(prepared.spec.adapter, adapter);
            assert_eq!(
                prepared.spec.confirmation,
                refact_buddy_core::operator_types::ConfirmationClass::UserConfirmationRequired
            );
            assert_eq!(
                prepared.spec.authority_class,
                refact_buddy_core::operator_types::AuthorityClass::Mutating
            );
            crate::buddy::resource_execution::validate_prepared_draft(&self.runtime, &prepared)
                .await
                .unwrap();
            prepared
        }
    }

    #[tokio::test]
    async fn native_preference_and_customization_proposals_attach_without_mutating_targets() {
        let fx = ProposalFixture::new().await;
        let settings = Some(br#"{"enabled":false,"auto_diagnostics":false,"auto_issue_creation":false,"quiet_mode":false,"future":{"keep":1}}"#.to_vec());
        crate::buddy::settings::settings_compare_exchange(&fx.root, &None, &settings)
            .await
            .unwrap();
        let report =
            r#"{"family":"buddy_preference","preference":{"setting":"quiet_mode","enabled":true}}"#;
        let work = fx
            .publish(USER_HABIT_COACH_WORKFLOW_ID, "preference", report, vec![])
            .await;
        let hash = fx
            .attach(
                USER_HABIT_COACH_WORKFLOW_ID,
                "preference",
                report,
                work.clone(),
            )
            .await
            .unwrap();
        let prepared = fx.prepared(&hash, "buddy_settings").await;
        assert_eq!(
            crate::buddy::settings::settings_snapshot(&fx.root)
                .await
                .unwrap(),
            settings
        );
        assert!(fx
            .attach(
                USER_HABIT_COACH_WORKFLOW_ID,
                "foreign",
                report,
                work.clone()
            )
            .await
            .is_err());
        let foreign = fx
            .publish(ERROR_DETECTIVE_WORKFLOW_ID, "diagnostic", report, vec![])
            .await;
        assert!(fx
            .attach(USER_HABIT_COACH_WORKFLOW_ID, "diagnostic", report, foreign)
            .await
            .is_err());
        let different = r#"{"family":"buddy_preference","preference":{"setting":"quiet_mode","enabled":false}}"#;
        assert!(attach_local_proposal(
            fx.app.clone(),
            &fx.runtime,
            work.clone(),
            "preference",
            report,
            parse_local_proposal(USER_HABIT_COACH_WORKFLOW_ID, different).unwrap()
        )
        .await
        .is_err());
        let changed = Some(br#"{"enabled":false,"auto_diagnostics":false,"auto_issue_creation":false,"quiet_mode":true}"#.to_vec());
        crate::buddy::settings::settings_compare_exchange(&fx.root, &settings, &changed)
            .await
            .unwrap();
        assert!(crate::buddy::resource_execution::registered_catalog(
            fx.app.clone(),
            &fx.runtime,
            &prepared.spec
        )
        .await
        .unwrap()
        .preview(&prepared.spec)
        .await
        .is_err());
        let artifact = fx
            .runtime
            .get_artifact_revision(
                work.value.current_artifact_id.unwrap(),
                work.value.current_artifact_revision,
            )
            .await
            .unwrap()
            .unwrap();
        assert!(artifact.value.body.contains(report));

        let mode = fx.root.join(".refact/modes/demo.yaml");
        std::fs::create_dir_all(mode.parent().unwrap()).unwrap();
        let original = b"schema_version: 1\nid: demo\ntitle: Demo\ndescription: Old\nspecific: false\nprompt: ''\nplan_template: ''\ntools: []\nfuture_key: retained\n";
        std::fs::write(&mode, original).unwrap();
        let report = r#"{"family":"customization_description","kind":"modes","id":"demo","description":"Describe local evidence"}"#;
        let work = fx
            .publish(SETUP_COACH_WORKFLOW_ID, "customization", report, vec![])
            .await;
        let hash = fx
            .attach(SETUP_COACH_WORKFLOW_ID, "customization", report, work)
            .await
            .unwrap();
        fx.prepared(&hash, "project_customization_file").await;
        assert_eq!(std::fs::read(&mode).unwrap(), original);
        fx.runtime.shutdown().await;
    }

    #[tokio::test]
    async fn native_default_model_proposal_requires_real_catalog_and_retains_selection() {
        let fx = ProposalFixture::new().await;
        let mut provider = crate::providers::create_provider("custom").unwrap();
        provider.provider_settings_apply(serde_yaml::from_str("enabled: true\nchat_endpoint: http://127.0.0.1:1/never-probe\nenabled_models: [chosen]\ncustom_models:\n  chosen: {n_ctx: 8192}\n").unwrap()).unwrap();
        fx.app.gcx.providers.write().await.add(provider);
        use crate::providers::operator_defaults as owner;
        let before = Some(
            b"chat: {model: 'custom/old'}\nchat_buddy: {model: 'custom/buddy'}\nfuture: retained\n"
                .to_vec(),
        );
        owner::defaults_compare_exchange(&fx.root, owner::DefaultsScope::Project, &None, &before)
            .await
            .unwrap();
        let report =
            r#"{"family":"default_model_selection","slot":"chat","model":"custom/chosen"}"#;
        let work = fx
            .publish(MODEL_COST_OPTIMIZER_WORKFLOW_ID, "model", report, vec![])
            .await;
        let hash = fx
            .attach(MODEL_COST_OPTIMIZER_WORKFLOW_ID, "model", report, work)
            .await
            .unwrap();
        fx.prepared(&hash, "default_models").await;
        assert_eq!(
            owner::defaults_snapshot(&fx.root, owner::DefaultsScope::Project)
                .await
                .unwrap(),
            before
        );
        let report =
            r#"{"family":"default_model_selection","slot":"chat","model":"custom/missing"}"#;
        let work = fx
            .publish(
                MODEL_COST_OPTIMIZER_WORKFLOW_ID,
                "missing-model",
                report,
                vec![],
            )
            .await;
        assert!(fx
            .attach(
                MODEL_COST_OPTIMIZER_WORKFLOW_ID,
                "missing-model",
                report,
                work
            )
            .await
            .is_err());
        assert_eq!(
            owner::defaults_snapshot(&fx.root, owner::DefaultsScope::Project)
                .await
                .unwrap(),
            before
        );
        fx.runtime.shutdown().await;
    }

    #[tokio::test]
    async fn native_memory_proposal_requires_original_run_digest_and_keeps_source_intact() {
        use crate::buddy::memory_lifecycle::{MemoryLifecycleOp, MemoryOpType, MemorySource};
        let fx = ProposalFixture::new().await;
        let path = fx
            .root
            .join(crate::file_filter::KNOWLEDGE_FOLDER_NAME)
            .join("memory.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let original =
            b"---\nid: memory\ntitle: Memory\nstatus: active\n---\n\n# Stable anchor\nBody\n";
        std::fs::write(&path, original).unwrap();
        let operation = MemoryLifecycleOp::pending(
            "fixture-archive",
            MemorySource::MemoryGarden,
            MemoryOpType::Archive,
            vec![path.to_string_lossy().into_owned()],
            "Review old evidence",
            0.9,
            "2026-01-01T00:00:00Z",
        );
        crate::buddy::storage::enqueue_memory_op(&fx.root, operation.clone())
            .await
            .unwrap();
        let digest = memory_operation_digest(&operation).unwrap();
        let evidence = EvidenceRef {
            evidence_id: "memory-source".into(),
            source: refact_buddy_core::operator_types::NativeSourceIdentity {
                source_kind: NativeSourceKind::Workflow,
                source_id: "memory-lifecycle:fixture-archive".into(),
                source_version: digest.clone(),
            },
            locator: "memory-lifecycle:fixture-archive".into(),
            observed_at: "2026-01-01T00:00:00Z".into(),
            digest: Some(digest),
        };
        let report = r#"{"family":"memory_lifecycle_selection","op_id":"fixture-archive"}"#;
        let work = fx
            .publish(
                MEMORY_GARDENER_WORKFLOW_ID,
                "memory",
                report,
                vec![evidence.clone()],
            )
            .await;
        let hash = fx
            .attach(MEMORY_GARDENER_WORKFLOW_ID, "memory", report, work)
            .await
            .unwrap();
        fx.prepared(&hash, "memory").await;
        let absent = fx
            .publish(
                MEMORY_GARDENER_WORKFLOW_ID,
                "unbound-memory",
                report,
                vec![],
            )
            .await;
        assert!(fx
            .attach(
                MEMORY_GARDENER_WORKFLOW_ID,
                "unbound-memory",
                report,
                absent
            )
            .await
            .is_err());
        let mut stale = evidence;
        stale.digest = Some("0".repeat(64));
        let work = fx
            .publish(
                MEMORY_GARDENER_WORKFLOW_ID,
                "stale-memory",
                report,
                vec![stale],
            )
            .await;
        assert!(fx
            .attach(MEMORY_GARDENER_WORKFLOW_ID, "stale-memory", report, work)
            .await
            .is_err());
        assert_eq!(std::fs::read(path).unwrap(), original);
        fx.runtime.shutdown().await;
    }

    #[tokio::test]
    async fn registered_candidate_proposal_attaches_only_to_native_origin_without_checkpointing() {
        let fx = ProposalFixture::new().await;
        for args in [
            vec!["init", "-b", "main"],
            vec!["config", "user.name", "Fixture"],
            vec!["config", "user.email", "fixture@example.invalid"],
        ] {
            assert!(std::process::Command::new("git")
                .args(args)
                .current_dir(&fx.root)
                .output()
                .unwrap()
                .status
                .success());
        }
        std::fs::write(fx.root.join("source.txt"), "initial\n").unwrap();
        for args in [vec!["add", "source.txt"], vec!["commit", "-m", "fixture"]] {
            assert!(std::process::Command::new("git")
                .args(args)
                .current_dir(&fx.root)
                .output()
                .unwrap()
                .status
                .success());
        }
        let owner = crate::worktrees::service::WorktreeService::new(
            fx.app.paths.cache_dir.clone(),
            fx.root.clone(),
        )
        .unwrap();
        let created = owner
            .create_worktree(crate::worktrees::types::CreateWorktreeRequest {
                branch: Some("refact/chat/proposal-fixture".into()),
                chat_id: Some("candidate-run".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        let meta = created.worktree.meta;
        let mut session = crate::chat::types::ChatSession::new("candidate-run".into());
        session.thread.worktree = Some(meta.clone());
        fx.app.chat.sessions.write().await.insert(
            "candidate-run".into(),
            std::sync::Arc::new(tokio::sync::Mutex::new(session)),
        );
        std::fs::write(meta.root.join("source.txt"), "candidate\n").unwrap();
        let registry_before = std::fs::read(owner.registry_path()).unwrap();
        let head = crate::worktrees::git::head_rev(&meta.root).unwrap();
        let report = serde_json::json!({"family":"isolated_candidate_checkpoint","worktree_id":meta.id,"chat_id":"candidate-run"}).to_string();
        let work = fx
            .publish(
                DEPENDENCY_RADAR_WORKFLOW_ID,
                "candidate-run",
                &report,
                vec![],
            )
            .await;
        let hash = fx
            .attach(DEPENDENCY_RADAR_WORKFLOW_ID, "candidate-run", &report, work)
            .await
            .unwrap();
        let prepared = fx.prepared(&hash, "project_code").await;
        assert_eq!(
            std::fs::read(owner.registry_path()).unwrap(),
            registry_before
        );
        assert_eq!(crate::worktrees::git::head_rev(&meta.root).unwrap(), head);
        assert_eq!(
            std::fs::read(meta.root.join("source.txt")).unwrap(),
            b"candidate\n"
        );
        let foreign = fx
            .publish(
                DEPENDENCY_RADAR_WORKFLOW_ID,
                "foreign-candidate",
                &report,
                vec![],
            )
            .await;
        assert!(fx
            .attach(
                DEPENDENCY_RADAR_WORKFLOW_ID,
                "foreign-candidate",
                &report,
                foreign
            )
            .await
            .is_err());
        std::fs::write(meta.root.join("source.txt"), "changed after preview\n").unwrap();
        assert!(crate::buddy::resource_execution::registered_catalog(
            fx.app.clone(),
            &fx.runtime,
            &prepared.spec
        )
        .await
        .unwrap()
        .preview(&prepared.spec)
        .await
        .is_err());
        fx.runtime.shutdown().await;
    }

    fn grounded() -> SourceCheck {
        SourceCheck {
            scope: "Observed local manifest paths only".into(),
            input_fingerprint: "manifest-hash".into(),
            completion: SourceCompletion::Complete,
            findings: vec![],
            evidence: vec![SourceCheck::check_evidence(
                "manifest-check",
                "manifest-hash",
                "refact://check/manifest-observation",
                "2026-01-01T00:00:00Z".into(),
            )],
        }
    }

    #[test]
    fn source_check_requires_completed_scope_and_valid_evidence_for_no_change() {
        let check = grounded();
        assert_eq!(
            check
                .clone()
                .into_report(DEPENDENCY_RADAR_WORKFLOW_ID)
                .terminal,
            WorkflowTerminal::NoChange
        );
        let mut missing = check.clone();
        missing.evidence.clear();
        assert_eq!(
            missing.into_report(DEPENDENCY_RADAR_WORKFLOW_ID).terminal,
            WorkflowTerminal::Inconclusive
        );
        let mut malformed = check.clone();
        malformed.evidence[0].observed_at = "yesterday".into();
        assert_eq!(
            malformed.into_report(DEPENDENCY_RADAR_WORKFLOW_ID).terminal,
            WorkflowTerminal::Inconclusive
        );
        let mut blank_scope = check;
        blank_scope.scope = "   ".into();
        assert_eq!(
            blank_scope
                .into_report(DEPENDENCY_RADAR_WORKFLOW_ID)
                .terminal,
            WorkflowTerminal::Inconclusive
        );
    }

    /// The whole point of `SourceCompletion`: a failed or skipped read must never be able to
    /// render as "no change", and its body must say so in words a reader can act on.
    #[test]
    fn unread_sources_never_become_no_change_and_say_why() {
        for (completion, expected, must_contain) in [
            (
                SourceCompletion::Failed {
                    error: "git index is locked".into(),
                },
                WorkflowTerminal::Failed {
                    failure_class: Some("source_read_failed".into()),
                },
                "git index is locked",
            ),
            (
                SourceCompletion::NotApplicable {
                    reason: "no git repository at the project root".into(),
                },
                WorkflowTerminal::Blocked {
                    failure_class: Some("source_absent".into()),
                },
                "no git repository",
            ),
            (
                SourceCompletion::Partial {
                    uncovered: "stopped after 5000 files".into(),
                },
                WorkflowTerminal::Partial {
                    failure_class: Some("partial_source_coverage".into()),
                },
                "stopped after 5000 files",
            ),
        ] {
            let mut check = grounded();
            check.completion = completion;
            assert!(check.findings.is_empty());
            let report = check.into_report(SECURITY_WHISPERER_WORKFLOW_ID);
            assert_eq!(report.terminal, expected);
            assert!(report.body.contains(must_contain), "{}", report.body);
            assert!(!report
                .body
                .contains("No findings within the stated source-check scope"));
        }
    }

    #[test]
    fn findings_only_complete_the_run_when_the_whole_scope_was_read() {
        let mut check = grounded();
        check.findings = vec!["src/auth.rs: hardcoded token pattern".into()];
        assert_eq!(
            check
                .clone()
                .into_report(SECURITY_WHISPERER_WORKFLOW_ID)
                .terminal,
            WorkflowTerminal::Completed
        );
        check.completion = SourceCompletion::Partial {
            uncovered: "untracked binaries were skipped".into(),
        };
        let report = check.into_report(SECURITY_WHISPERER_WORKFLOW_ID);
        assert_eq!(
            report.terminal,
            WorkflowTerminal::Partial {
                failure_class: Some("partial_source_coverage".into())
            }
        );
        assert!(report.body.contains("src/auth.rs"));
        assert!(report.body.contains("untracked binaries were skipped"));
    }

    #[test]
    fn blank_findings_are_rejected_rather_than_published_as_empty_lines() {
        let mut check = grounded();
        check.findings = vec!["  ".into()];
        assert_eq!(
            check.into_report(DOCS_GARDENER_WORKFLOW_ID).terminal,
            WorkflowTerminal::Inconclusive
        );
    }

    #[test]
    fn skill_author_protocol_is_canonical_and_accepts_only_its_final_fenced_proposal() {
        let instruction = local_proposal_protocol_instruction(BUDDY_SKILL_AUTHOR_WORKFLOW_ID, None)
            .expect("skill author protocol");
        assert!(instruction.contains("```json\n"));
        assert!(instruction.contains("\"family\":\"markdown_extension\""));
        assert!(instruction.contains("\"kind\":\"skill\""));
        assert!(instruction.contains("\"name\":\"example_skill\""));
        assert!(instruction.contains("\"description\""));
        assert!(instruction.contains("\"body\""));
        assert!(instruction.contains("grants no authority"));
        // The candidate vocabulary exists only when the engine already found a registration.
        assert!(local_proposal_protocol_instruction(DEPENDENCY_RADAR_WORKFLOW_ID, None).is_none());
        let checkpoint_instruction = local_proposal_protocol_instruction(
            DEPENDENCY_RADAR_WORKFLOW_ID,
            Some("native_candidate"),
        )
        .unwrap();
        assert!(checkpoint_instruction.contains("isolated_candidate_checkpoint"));
        assert!(checkpoint_instruction.contains("creates no worktree"));
        assert!(checkpoint_instruction.contains("native_candidate"));

        let report = concat!(
            "Observed a repeatable workflow gap.\n\n```json\n",
            "{\"family\":\"markdown_extension\",\"kind\":\"skill\",\"name\":\"safe_skill\",\"description\":\"Safe local draft\",\"body\":\"Use retained evidence only.\"}\n",
            "```"
        );
        assert!(matches!(
            parse_local_proposal(BUDDY_SKILL_AUTHOR_WORKFLOW_ID, report),
            Some(WorkflowProposal::MarkdownExtension { .. })
        ));
        assert!(parse_local_proposal(DEPENDENCY_RADAR_WORKFLOW_ID, report).is_none());
        assert!(parse_local_proposal(
            BUDDY_SKILL_AUTHOR_WORKFLOW_ID,
            &(report.to_string() + "\nApplied it already.")
        )
        .is_none());

        let checkpoint = r#"{"family":"isolated_candidate_checkpoint","worktree_id":"native_candidate","chat_id":"proposal_run"}"#;
        assert!(matches!(
            parse_local_proposal(DEPENDENCY_RADAR_WORKFLOW_ID, checkpoint),
            Some(WorkflowProposal::IsolatedCandidateCheckpoint { .. })
        ));
        assert!(parse_local_proposal(BUDDY_SKILL_AUTHOR_WORKFLOW_ID, checkpoint).is_none());
        assert!(parse_local_proposal(
            DEPENDENCY_RADAR_WORKFLOW_ID,
            r#"{"family":"isolated_candidate_checkpoint","worktree_id":"foreign/candidate","chat_id":"proposal_run"}"#
        ).is_none());
    }

    /// Every family with a registered action offers exactly its own vocabulary, and a report
    /// from a different family is refused rather than reinterpreted.
    #[test]
    fn each_action_family_offers_and_accepts_only_its_own_bounded_vocabulary() {
        use refact_buddy_core::autonomous_workflows::{workflow_action_family, WorkflowActionFamily};
        let cases = [
            (
                MEMORY_GARDENER_WORKFLOW_ID,
                r#"{"family":"memory_lifecycle_selection","op_id":"memory-garden:archive:abc123"}"#,
                "memory_lifecycle_selection",
            ),
            (
                MODEL_COST_OPTIMIZER_WORKFLOW_ID,
                r#"{"family":"default_model_selection","slot":"chat","model":"vendor/model-a"}"#,
                "default_model_selection",
            ),
            (
                USER_HABIT_COACH_WORKFLOW_ID,
                r#"{"family":"buddy_preference","preference":{"setting":"quiet_mode","enabled":true}}"#,
                "buddy_preference",
            ),
            (
                SETUP_COACH_WORKFLOW_ID,
                r#"{"family":"customization_description","kind":"modes","id":"fixture","description":"Readable text"}"#,
                "customization_description",
            ),
        ];
        for (workflow_id, report, vocabulary) in cases {
            let instruction = local_proposal_protocol_instruction(workflow_id, None)
                .unwrap_or_else(|| panic!("{workflow_id} has no protocol"));
            assert!(instruction.contains(vocabulary), "{workflow_id}");
            assert!(instruction.contains("grants no authority"), "{workflow_id}");
            assert!(
                parse_local_proposal(workflow_id, report).is_some(),
                "{workflow_id}"
            );
            // A foreign family's vocabulary never crosses into another family's action path.
            for (other, _, _) in cases {
                if other != workflow_id {
                    assert!(
                        parse_local_proposal(other, report).is_none(),
                        "{other} accepted {vocabulary}"
                    );
                }
            }
            assert!(parse_local_proposal(BUDDY_DAILY_DIGEST_WORKFLOW_ID, report).is_none());
        }
        // Read-only families are offered no proposal vocabulary at all.
        for workflow in AUTONOMOUS_BUDDY_WORKFLOWS {
            if matches!(
                workflow_action_family(workflow.id),
                WorkflowActionFamily::SavedEvidenceOnly { .. }
            ) {
                assert!(
                    local_proposal_protocol_instruction(workflow.id, None).is_none(),
                    "{}",
                    workflow.id
                );
                assert!(
                    local_proposal_protocol_instruction(workflow.id, Some("candidate")).is_none(),
                    "{}",
                    workflow.id
                );
            }
        }
    }

    /// The bounded vocabularies cannot be widened into authority, paths, or adapter arguments,
    /// and out-of-range or unparseable values are refused rather than clamped.
    #[test]
    fn bounded_proposal_variants_reject_authority_paths_and_out_of_range_values() {
        for rejected in [
            // Unknown fields are denied by the tagged representation itself.
            r#"{"family":"memory_lifecycle_selection","op_id":"op","authority":"mutating"}"#,
            r#"{"family":"memory_lifecycle_selection","op_id":"../../etc/passwd"}"#,
            r#"{"family":"memory_lifecycle_selection","op_id":""}"#,
        ] {
            assert!(
                parse_local_proposal(MEMORY_GARDENER_WORKFLOW_ID, rejected).is_none(),
                "{rejected}"
            );
        }
        for rejected in [
            r#"{"family":"default_model_selection","slot":"chat","model":"no-slash"}"#,
            r#"{"family":"default_model_selection","slot":"chat","model":"a/b/c"}"#,
            r#"{"family":"default_model_selection","slot":"autonomy","model":"a/b"}"#,
            r#"{"family":"default_model_selection","slot":"chat","model":"a/b","expected_selected":"x"}"#,
        ] {
            assert!(
                parse_local_proposal(MODEL_COST_OPTIMIZER_WORKFLOW_ID, rejected).is_none(),
                "{rejected}"
            );
        }
        for rejected in [
            r#"{"family":"buddy_preference","preference":{"setting":"daily_digest_hour","hour":24}}"#,
            r#"{"family":"buddy_preference","preference":{"setting":"autonomy_level","enabled":true}}"#,
            r#"{"family":"buddy_preference","preference":{"setting":"quiet_mode","enabled":true,"grant":"all"}}"#,
        ] {
            assert!(
                parse_local_proposal(BEHAVIOR_LEARNER_WORKFLOW_ID, rejected).is_none(),
                "{rejected}"
            );
        }
        for rejected in [
            r#"{"family":"customization_description","kind":"modes","id":"fixture","description":"x","tools":["shell"]}"#,
            r#"{"family":"customization_description","kind":"hooks","id":"fixture","description":"x"}"#,
            r#"{"family":"customization_description","kind":"modes","id":"../escape","description":"x"}"#,
            r#"{"family":"customization_description","kind":"modes","id":"fixture","description":"   "}"#,
        ] {
            assert!(
                parse_local_proposal(BUDDY_ONBOARDING_WORKFLOW_ID, rejected).is_none(),
                "{rejected}"
            );
        }
    }

    #[test]
    fn only_read_scopes_with_findings_go_to_a_model() {
        let clean = grounded();
        assert!(!clean.needs_assessment());
        let mut found = grounded();
        found.findings = vec!["src/auth.rs: hardcoded token pattern".into()];
        assert!(found.needs_assessment());
        let mut partial = found.clone();
        partial.completion = SourceCompletion::Partial {
            uncovered: "untracked binaries".into(),
        };
        assert!(partial.needs_assessment());
        let mut failed = found.clone();
        failed.completion = SourceCompletion::Failed {
            error: "git index is locked".into(),
        };
        assert!(!failed.needs_assessment());
        let mut ungrounded = found;
        ungrounded.evidence.clear();
        assert!(!ungrounded.needs_assessment());
    }

    /// A failed read may still list what it saw before failing, clearly labelled, but a source
    /// that was never checked cannot carry findings at all.
    #[test]
    fn failed_reads_keep_early_findings_and_absent_sources_reject_them() {
        let mut failed = grounded();
        failed.findings = vec!["recent diagnostic: token=[REDACTED]".into()];
        failed.completion = SourceCompletion::Failed {
            error: "git status: index locked".into(),
        };
        let report = failed.into_report(SECURITY_WHISPERER_WORKFLOW_ID);
        assert_eq!(
            report.terminal,
            WorkflowTerminal::Failed {
                failure_class: Some("source_read_failed".into())
            }
        );
        assert!(report.body.contains("read before the failure"));
        assert!(report.body.contains("recent diagnostic: token=[REDACTED]"));
        assert!(report.body.contains("Coverage is unknown"));

        let mut absent = grounded();
        absent.findings = vec!["impossible".into()];
        absent.completion = SourceCompletion::NotApplicable {
            reason: "no git repository".into(),
        };
        assert!(!absent.needs_assessment());
        assert_eq!(
            absent.into_report(SECURITY_WHISPERER_WORKFLOW_ID).terminal,
            WorkflowTerminal::Inconclusive
        );
    }

    /// Exercise the production attachment boundary against a real outcome store and real
    /// extension catalog. Preparation and preview must leave the native target absent; the
    /// attached desk action is still only a reviewable, ungranted proposal.
    #[tokio::test]
    async fn markdown_proposal_attaches_only_to_its_saved_artifact_without_writing() {
        use crate::buddy::outcome_runtime::OutcomeRuntime;
        use refact_buddy_core::operator_types::{ProjectIdentity, WorkerIdentity};

        let root = tempfile::tempdir().unwrap();
        let project_root = root.path().join("project");
        std::fs::create_dir(&project_root).unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            root.path().join("cache"),
            root.path().join("config"),
        )
        .await;
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        let project = ProjectIdentity {
            project_id: "proposal-fixture".into(),
            canonical_root: dunce::canonicalize(&project_root)
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        };
        let worker = WorkerIdentity {
            worker_id: "worker".into(),
            engine_instance_id: "engine".into(),
            epoch: 1,
        };
        let runtime = OutcomeRuntime::open(&project_root, project.clone(), worker).unwrap();
        let source_report = r#"{"family":"markdown_extension","kind":"skill","name":"fixture","description":"Fixture skill","body":"Read fixture evidence only."}"#;
        let mut start = runtime.start_workflow(
            BUDDY_SKILL_AUTHOR_WORKFLOW_ID,
            "2026-01-01T00:00:00Z".into(),
        );
        start.run_id = "proposal-run".into();
        start.input_fingerprint = "proposal-source-fingerprint".into();
        start.coverage = "fixture source".into();
        runtime
            .publish_assessment(
                start,
                "Fixture proposal".into(),
                source_report.into(),
                vec![],
            )
            .await
            .unwrap();
        let outcome = runtime
            .get_run_outcome("proposal-run".into())
            .await
            .unwrap()
            .unwrap();
        let work = runtime
            .get_work_item(outcome.value.work_id.clone())
            .await
            .unwrap()
            .unwrap();
        let proposal = parse_local_proposal(BUDDY_SKILL_AUTHOR_WORKFLOW_ID, source_report).unwrap();
        let hash = attach_local_proposal(
            app.clone(),
            &runtime,
            work.clone(),
            "proposal-run",
            source_report,
            proposal,
        )
        .await
        .unwrap();
        assert!(!hash.is_empty());
        assert!(!project_root
            .join(".refact/skills/fixture/SKILL.md")
            .exists());
        let work_id = work.value.id.clone();
        let actions = runtime
            .with_store(move |store| crate::buddy::desk_actions::actions_for_work(store, &work_id))
            .await
            .unwrap();
        assert_eq!(actions.len(), 3);
        assert_eq!(actions[0].spec_hash, hash);
        assert_eq!(actions[0].spec.adapter, "extensions");
        assert_eq!(actions[0].spec.kind, "markdown_project_apply");
        assert_eq!(
            actions[1].spec.action_id,
            crate::buddy::desk_actions::CHECK_SAVED_EVIDENCE
        );
        assert_eq!(
            actions[2].spec.action_id,
            crate::buddy::desk_actions::DISMISS_NOT_A_PROBLEM
        );
        assert!(attach_local_proposal(
            app.clone(),
            &runtime,
            work.clone(),
            "proposal-run",
            "different retained report",
            parse_local_proposal(BUDDY_SKILL_AUTHOR_WORKFLOW_ID, source_report).unwrap(),
        )
        .await
        .is_err());
        let different_proposal = parse_local_proposal(
            BUDDY_SKILL_AUTHOR_WORKFLOW_ID,
            r#"{"family":"markdown_extension","kind":"skill","name":"other","body":"A different proposal."}"#,
        )
        .unwrap();
        assert!(attach_local_proposal(
            app,
            &runtime,
            work,
            "proposal-run",
            source_report,
            different_proposal,
        )
        .await
        .is_err());
        assert!(!project_root
            .join(".refact/skills/fixture/SKILL.md")
            .exists());
        runtime.shutdown().await;
    }
}
