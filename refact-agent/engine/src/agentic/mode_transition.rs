pub use refact_agentic::mode_transition::{
    ConversationMetadata, FileReference, ParsedDecisions, TransitionContextBudget,
    assemble_new_chat as assemble_new_chat_pure, calculate_transition_context_budget,
    carried_goal_messages, carried_plan_messages, context_file_rendered_symbols,
    count_images_in_messages, current_base_goal_message, extract_conversation_metadata,
    extract_initial_plan_text, format_annotated_messages, format_budget_summary, format_file_list,
    format_memory_list, insert_goal_messages_before_plan, is_goal_delta_event,
    make_pinned_plan_message, message_symbols, normalize_goal_message_content, parse_llm_response,
    push_context_file_with_budget, text_symbols, transfer_goal_ownership, GoalTransferResult,
    truncate_utf8,
};

use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use refact_context_api::PathsAccess;

use crate::call_validation::{ChatContent, ChatMessage};
use crate::global_context::GlobalContext;
use crate::subchat::{
    run_subchat, resolve_subchat_config_with_explicit_params, resolve_subchat_params,
    resolve_subchat_model, ExplicitSubchatSpec, TraceParent,
};
use crate::yaml_configs::customization_registry::get_subagent_config;

const SUBAGENT_ID: &str = "mode_transition";

#[derive(Clone, Debug)]
pub struct AgenticPathContext {
    cache_dir: PathBuf,
    config_dir: PathBuf,
    workspace_folders: Vec<PathBuf>,
}

impl AgenticPathContext {
    pub fn from_context<T: PathsAccess + ?Sized>(context: &T) -> Self {
        Self {
            cache_dir: context.cache_dir(),
            config_dir: context.config_dir(),
            workspace_folders: context.workspace_folders(),
        }
    }
}

impl PathsAccess for AgenticPathContext {
    fn cache_dir(&self) -> PathBuf {
        self.cache_dir.clone()
    }

    fn config_dir(&self) -> PathBuf {
        self.config_dir.clone()
    }

    fn workspace_folders(&self) -> Vec<PathBuf> {
        self.workspace_folders.clone()
    }
}

pub type ReconstructionHints = ParsedDecisions;

pub struct ReconstructionRequest<'a> {
    pub messages: &'a [ChatMessage],
    pub target_mode: &'a str,
    pub target_mode_description: &'a str,
    pub parent_chat_id: Option<&'a str>,
    pub model_override: Option<String>,
    pub abort_flag: Option<Arc<AtomicBool>>,
    pub hints: Option<ReconstructionHints>,
    pub target_budget_symbols: Option<usize>,
    pub preserve_goal_messages: bool,
}

pub struct ReconstructionOutcome {
    pub messages: Vec<ChatMessage>,
    pub decisions: ParsedDecisions,
    pub model: String,
    pub budget: TransitionContextBudget,
    pub reread_paths: Vec<String>,
}

fn check_cancelled(request: &ReconstructionRequest<'_>) -> Result<(), String> {
    if request
        .abort_flag
        .as_ref()
        .is_some_and(|f| f.load(Ordering::Acquire))
    {
        Err("Context reconstruction cancelled".into())
    } else {
        Ok(())
    }
}

struct MessageAudit<'a>(&'a [ChatMessage]);
impl refact_privacy::PrivacyAudited for MessageAudit<'_> {
    fn privacy_records(
        &self,
    ) -> Result<Vec<(usize, refact_privacy::FileRecord)>, refact_privacy::PrivacyAuditError> {
        refact_privacy::records_from_messages(self.0)
    }
}

async fn analyze_reconstruction(
    gcx: Arc<GlobalContext>,
    request: &ReconstructionRequest<'_>,
) -> Result<(ParsedDecisions, String, TransitionContextBudget), String> {
    check_cancelled(request)?;
    if request.messages.is_empty() {
        return Err("The provided chat is empty".into());
    }
    let subagent = get_subagent_config(gcx.clone(), SUBAGENT_ID, None)
        .await
        .ok_or("mode_transition config not found")?;
    let template = subagent
        .messages
        .user_template
        .as_ref()
        .ok_or("Missing reconstruction prompt")?;
    let mut params = resolve_subchat_params(gcx.clone(), SUBAGENT_ID).await?;
    if let Some(model) = &request.model_override {
        params.subchat_model = model.clone();
    }
    let model = resolve_subchat_model(gcx.clone(), &params).await?;
    let spec = ExplicitSubchatSpec {
        params,
        model: model.clone(),
        autonomous_no_confirm: false,
    };
    let mut config = resolve_subchat_config_with_explicit_params(
        gcx.clone(),
        SUBAGENT_ID,
        &spec,
        false,
        None,
        None,
        None,
        None,
        None,
        Some(vec![]),
        1,
        false,
        "agent".into(),
        None,
        None,
        None,
        None,
        request.abort_flag.clone(),
        0,
    )
    .await?;
    config.trace_parent = TraceParent::from_parts(request.parent_chat_id, None);
    let caps = crate::global_context::try_load_caps_quickly_if_not_present(gcx.clone(), 0)
        .await
        .map_err(|e| e.to_string())?;
    let model_rec = crate::caps::resolve_chat_model(caps, &model)?;
    let tokenizer = crate::tokens::cached_tokenizer(gcx.clone(), &model_rec.base).await?;
    let mut budget = calculate_transition_context_budget(request.messages);
    if let Some(total) = request.target_budget_symbols {
        budget.total_symbols = total;
        budget.files_symbols = total * 70 / 100;
        budget.messages_symbols = total - budget.files_symbols;
    }
    let metadata = extract_conversation_metadata(request.messages);
    // Full current request/control artifacts are mandatory; older entries have bounded previews.
    let latest_user = request.messages.iter().rposition(|m| m.role == "user");
    let hints = request
        .hints
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    let render = |cap: usize| {
        let mut annotated = String::new();
        for (index, (id, message)) in metadata.annotated_messages.iter().enumerate() {
            let mandatory = Some(index) == latest_user
                || matches!(message.role.as_str(), "plan" | "goal")
                || message
                    .extra
                    .get("event")
                    .and_then(|e| e.get("subkind"))
                    .and_then(|s| s.as_str())
                    .is_some_and(|s| matches!(s, "plan_delta" | "goal_delta" | "goal_pursuit"));
            let text = message.content.content_text_only();
            let text = if mandatory {
                text
            } else {
                truncate_utf8(&text, cap)
            };
            annotated.push_str(&format!("[{id}] [{}]\n{text}\n", message.role));
            if let Some(calls) = &message.tool_calls {
                for call in calls {
                    annotated.push_str(&format!(
                        "tool: {} {}\n",
                        call.function.name,
                        truncate_utf8(&call.function.arguments, cap)
                    ));
                }
            }
        }
        format!(
            "{}\nCaller preservation hints (source-grounded only): {}",
            template
                .replace("{target_mode}", request.target_mode)
                .replace("{target_mode_description}", request.target_mode_description)
                .replace("{annotated_message_list}", &annotated)
                .replace("{file_list}", &format_file_list(&metadata))
                .replace("{memory_list}", &format_memory_list(&metadata))
                .replace(
                    "{budget_summary}",
                    &format_budget_summary(budget, request.messages)
                ),
            hints
        )
    };
    let input_limit = config
        .n_ctx
        .saturating_sub(config.max_new_tokens)
        .saturating_sub(1024);
    let mut cap = 16000;
    let prompt = loop {
        let prompt = render(cap);
        // Without a local tokenizer use a conservative byte bound rather than
        // chars/4, which can undercount dense Unicode or source text.
        let tokens = if tokenizer.is_some() {
            crate::tokens::count_text_tokens(tokenizer.clone(), &prompt)?
        } else {
            prompt.len()
        };
        if tokens <= input_limit {
            break prompt;
        }
        if cap == 0 {
            return Err(
                "Mandatory reconstruction input and instructions exceed resolved analyzer context"
                    .into(),
            );
        }
        cap /= 2;
    };
    let mut analysis = ChatMessage::new("user".into(), prompt);
    crate::privacy::records::carry_records_into(&mut analysis, request.messages)
        .map_err(|e| e.to_string())?;
    crate::privacy::destinations::clear_for_model(
        &gcx,
        MessageAudit(std::slice::from_ref(&analysis)),
        &model_rec.base,
    )
    .map_err(|e| e.to_string())?;
    check_cancelled(request)?;
    let result = run_subchat(gcx, vec![analysis], config).await?;
    check_cancelled(request)?;
    let response = result
        .messages
        .last()
        .filter(|m| m.role == "assistant" && m.tool_calls.as_ref().is_none_or(|c| c.is_empty()))
        .ok_or("No reconstruction analysis answer generated")?;
    let decisions = parse_llm_response(&response.content.content_text_only());
    refact_agentic::mode_transition::validate_decisions(&decisions, request.messages)?;
    Ok((decisions, model, budget))
}

pub async fn reconstruct_context(
    gcx: Arc<GlobalContext>,
    request: ReconstructionRequest<'_>,
) -> Result<ReconstructionOutcome, String> {
    let (mut decisions, model, budget) = analyze_reconstruction(gcx.clone(), &request).await?;
    let workspace_dirs = gcx.workspace_folders();
    let metadata = extract_conversation_metadata(request.messages);
    let allowed: std::collections::HashSet<_> = metadata
        .context_files
        .iter()
        .chain(&metadata.edited_files)
        .map(|f| f.path.clone())
        .chain(metadata.memory_paths.iter().cloned())
        .collect();
    let caps = crate::global_context::try_load_caps_quickly_if_not_present(gcx.clone(), 0)
        .await
        .map_err(|e| e.to_string())?;
    let model_rec = crate::caps::resolve_chat_model(caps, &model)?;
    let mut reread_records = std::collections::HashMap::new();
    let mut preloaded = std::collections::HashMap::new();
    // Only canonical, conversation-allowlisted, runtime-cleared paths may be read by assembly.
    for paths in [
        &mut decisions.files_to_open,
        &mut decisions.memories_to_include,
    ] {
        let mut safe = Vec::new();
        for path in paths.iter() {
            check_cancelled(&request)?;
            if !allowed.contains(path) {
                continue;
            }
            let full = PathBuf::from(path);
            let full = if full.is_absolute() {
                full
            } else if let Some(root) = workspace_dirs.first() {
                root.join(full)
            } else {
                continue;
            };
            let Ok(canonical) = full.canonicalize() else {
                continue;
            };
            let in_workspace = workspace_dirs.iter().any(|root| {
                root.canonicalize()
                    .is_ok_and(|root| canonical.starts_with(root))
            });
            let in_refact = canonical
                .components()
                .any(|c| matches!(c, std::path::Component::Normal(name) if name == ".refact"));
            if !in_workspace && !in_refact {
                continue;
            }
            crate::files_in_workspace::check_file_privacy_for_model_context(
                gcx.clone(),
                &canonical,
            )
            .await?;
            let record = crate::privacy::records::declared_file_record(&gcx, &canonical)?;
            let mut audit = ChatMessage::default();
            crate::privacy::records::attach_record(&mut audit, record.clone());
            crate::privacy::destinations::clear_for_model(
                &gcx,
                MessageAudit(std::slice::from_ref(&audit)),
                &model_rec.base,
            )
            .map_err(|e| e.to_string())?;
            // Read the same canonical path that was cleared, once; pure assembly must
            // never resolve the original alias again after the privacy check.
            if tokio::fs::metadata(&canonical)
                .await
                .map_err(|e| e.to_string())?
                .len()
                > 1024 * 1024
            {
                continue;
            }
            let content = tokio::fs::read_to_string(&canonical)
                .await
                .map_err(|e| e.to_string())?;
            preloaded.insert(path.clone(), content);
            reread_records.insert(path.clone(), record);
            safe.push(path.clone());
        }
        *paths = safe;
    }
    let mut messages = refact_agentic::mode_transition::assemble_reconstruction(
        request.messages,
        &decisions,
        &workspace_dirs,
        budget,
        request.preserve_goal_messages,
        &preloaded,
    )
    .await?;
    let reread_paths = messages
        .iter()
        .filter_map(|m| match &m.content {
            ChatContent::ContextFiles(f) => Some(f.iter().map(|f| f.file_name.clone())),
            _ => None,
        })
        .flatten()
        .collect();
    let carried =
        crate::privacy::records::records_to_carry(request.messages).map_err(|e| e.to_string())?;
    attach_reconstruction_records(&mut messages, &carried, &reread_records);
    check_cancelled(&request)?;
    Ok(ReconstructionOutcome {
        messages,
        decisions,
        model,
        budget,
        reread_paths,
    })
}

/// Fresh derived `user` messages carry the source provenance union; fresh context files
/// carry the records of their own re-read files; cloned control messages keep their own.
fn attach_reconstruction_records(
    messages: &mut [ChatMessage],
    carried: &[refact_privacy::FileRecord],
    reread_records: &std::collections::HashMap<String, refact_privacy::FileRecord>,
) {
    for message in messages.iter_mut() {
        if !message.message_id.is_empty() {
            continue;
        }
        message.message_id = uuid::Uuid::new_v4().to_string();
        let records = match &message.content {
            ChatContent::ContextFiles(files) => files
                .iter()
                .filter_map(|file| reread_records.get(&file.file_name).cloned())
                .collect::<Vec<_>>(),
            _ if message.role == "user" => carried.to_vec(),
            _ => Vec::new(),
        };
        crate::privacy::records::merge_records(message, records);
    }
    crate::privacy::records::compact_reconstruction_privacy(messages);
}

pub async fn analyze_mode_transition(
    gcx: Arc<GlobalContext>,
    messages: &[ChatMessage],
    target_mode: &str,
    target_mode_description: &str,
    parent_chat_id: Option<&str>,
) -> Result<ParsedDecisions, String> {
    analyze_reconstruction(
        gcx,
        &ReconstructionRequest {
            messages,
            target_mode,
            target_mode_description,
            parent_chat_id,
            model_override: None,
            abort_flag: None,
            hints: None,
            target_budget_symbols: None,
            preserve_goal_messages: false,
        },
    )
    .await
    .map(|(decisions, _, _)| decisions)
}

pub async fn assemble_new_chat<T: PathsAccess + ?Sized>(
    context: &T,
    original_messages: &[ChatMessage],
    decisions: &ParsedDecisions,
) -> Result<Vec<ChatMessage>, String> {
    let workspace_dirs = context.workspace_folders();
    assemble_new_chat_pure(original_messages, decisions, &workspace_dirs).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call_validation::{ChatContent, ChatMessage, ContextFile};

    #[test]
    fn reconstruction_cancellation_is_checked_before_work() {
        let flag = Arc::new(AtomicBool::new(true));
        let request = ReconstructionRequest {
            messages: &[],
            target_mode: "agent",
            target_mode_description: "",
            parent_chat_id: None,
            model_override: None,
            abort_flag: Some(flag.clone()),
            hints: None,
            target_budget_symbols: None,
            preserve_goal_messages: true,
        };
        assert!(check_cancelled(&request).is_err());
        flag.store(false, Ordering::Release);
        assert!(check_cancelled(&request).is_ok());
    }

    #[test]
    fn reconstruction_audit_keeps_source_records() {
        let mut source = ChatMessage::new("user".into(), "source".into());
        crate::privacy::records::attach_record(
            &mut source,
            refact_privacy::FileRecord {
                path: "secret.rs".into(),
                zone: "private".into(),
                attribution: refact_privacy::Attribution::Declared,
            },
        );
        let mut narrative = ChatMessage::new("user".into(), "summary".into());
        crate::privacy::records::carry_records_into(&mut narrative, &[source]).unwrap();
        let records = refact_privacy::records_from_messages(&[narrative]).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].1.zone, "private");
    }

    fn guarded_record(path: &str, zone: &str) -> refact_privacy::FileRecord {
        refact_privacy::FileRecord {
            path: path.to_string(),
            zone: zone.to_string(),
            attribution: refact_privacy::Attribution::Declared,
        }
    }

    fn message_records(message: &ChatMessage) -> Vec<refact_privacy::FileRecord> {
        message
            .extra
            .get("privacy")
            .map(|value| {
                serde_json::from_value::<refact_privacy::PrivacyRecord>(value.clone())
                    .unwrap()
                    .files
            })
            .unwrap_or_default()
    }

    fn context_file_message(file_name: &str) -> ChatMessage {
        ChatMessage {
            role: "context_file".to_string(),
            content: ChatContent::ContextFiles(vec![ContextFile {
                file_name: file_name.to_string(),
                file_content: "fn main() {}\n".to_string(),
                line1: 1,
                line2: 1,
                ..Default::default()
            }]),
            ..Default::default()
        }
    }

    fn cloned_control(
        id: &str,
        role: &str,
        records: Vec<refact_privacy::FileRecord>,
    ) -> ChatMessage {
        let mut message = ChatMessage {
            message_id: id.to_string(),
            role: role.to_string(),
            content: ChatContent::SimpleText(format!("{role} body")),
            ..Default::default()
        };
        crate::privacy::records::merge_records(&mut message, records);
        message
    }

    #[test]
    fn attach_reconstruction_records_stamps_union_once_per_derived_user_message() {
        let secret = guarded_record("secret.rs", "secrets");
        let another = guarded_record("another.rs", "secrets");
        let other = guarded_record("other.rs", "secrets");
        let reread = guarded_record("src/a.rs", "secrets");
        let carried = vec![secret.clone(), another.clone()];
        let reread_records =
            std::collections::HashMap::from([("src/a.rs".to_string(), reread.clone())]);
        let mut messages = vec![
            context_file_message("src/a.rs"),
            ChatMessage::new("user".to_string(), "narrative".to_string()),
            ChatMessage::new("user".to_string(), "continuation".to_string()),
            cloned_control("evt-1", "event", vec![secret.clone()]),
            cloned_control("plan-1", "plan", vec![other.clone()]),
        ];

        attach_reconstruction_records(&mut messages, &carried, &reread_records);

        assert_eq!(message_records(&messages[0]), vec![reread.clone()]);
        assert_eq!(message_records(&messages[1]), carried);
        assert_eq!(message_records(&messages[2]), carried);
        assert!(messages[3].extra.get("privacy").is_none());
        assert_eq!(message_records(&messages[4]), vec![other.clone()]);
        assert!(messages.iter().all(|m| !m.message_id.is_empty()));
        assert_eq!(
            messages
                .iter()
                .filter(|m| message_records(m) == carried)
                .count(),
            2
        );
        let attached = refact_privacy::records_from_messages(&messages)
            .unwrap()
            .into_iter()
            .map(|(_, record)| record)
            .collect::<std::collections::HashSet<_>>();
        for record in [secret, another, reread, other] {
            assert!(attached.contains(&record), "{record:?} must survive");
        }
    }

    #[test]
    fn attach_reconstruction_records_bounds_copies_by_derived_user_messages() {
        let carried = (0..16_000)
            .map(|index| guarded_record(&format!("src/file_{index}.rs"), "secrets"))
            .collect::<Vec<_>>();
        let reread = guarded_record("src/a.rs", "secrets");
        let reread_records =
            std::collections::HashMap::from([("src/a.rs".to_string(), reread.clone())]);
        let mut messages = vec![
            context_file_message("src/a.rs"),
            ChatMessage::new("user".to_string(), "narrative".to_string()),
            ChatMessage::new("user".to_string(), "continuation".to_string()),
        ];
        messages.extend(
            (0..35).map(|index| cloned_control(&format!("evt-{index}"), "event", carried.clone())),
        );

        attach_reconstruction_records(&mut messages, &carried, &reread_records);

        let total_records = messages
            .iter()
            .map(|message| message_records(message).len())
            .sum::<usize>();
        assert_eq!(total_records, 2 * carried.len() + 1);
    }

    #[test]
    fn reconstruction_guarded_record_still_refuses_untrusted_provider() {
        let policy = refact_privacy::PrivacyPolicy {
            blocked: Vec::new(),
            zones: vec![
                refact_privacy::Zone {
                    name: "secrets".to_string(),
                    patterns: vec!["secret.rs".to_string()],
                    send_to: vec!["trusted".to_string()],
                    on_shell_read: refact_privacy::ShellBehavior::Withhold,
                },
                refact_privacy::Zone {
                    name: "normal".to_string(),
                    patterns: vec!["*".to_string()],
                    send_to: vec!["*".to_string()],
                    on_shell_read: refact_privacy::ShellBehavior::Withhold,
                },
            ],
            subagents: refact_privacy::SubagentPolicy::default(),
            ..Default::default()
        };
        let compiled = policy.compile().unwrap();
        let source = cloned_control(
            "src-1",
            "user",
            vec![guarded_record("secret.rs", "secrets")],
        );
        let carried = crate::privacy::records::records_to_carry(&[source]).unwrap();
        let mut messages = vec![
            context_file_message("src/a.rs"),
            ChatMessage::new("user".to_string(), "narrative".to_string()),
            ChatMessage::new("user".to_string(), "continuation".to_string()),
            cloned_control("evt-1", "event", carried.clone()),
        ];

        attach_reconstruction_records(&mut messages, &carried, &std::collections::HashMap::new());

        let destination = |id: &str| refact_privacy::Destination {
            id: refact_privacy::DestinationId(id.to_string()),
            kind: refact_privacy::DestinationKind::Provider,
            display_name: id.to_string(),
        };
        assert!(
            refact_privacy::clear(
                MessageAudit(&messages),
                &destination("untrusted"),
                &compiled
            )
            .is_err(),
            "compaction must not hide guarded records from the gate"
        );
        assert!(
            refact_privacy::clear(MessageAudit(&messages), &destination("trusted"), &compiled)
                .is_ok()
        );
    }

    struct TestPaths {
        workspace_folders: Vec<PathBuf>,
    }

    impl TestPaths {
        fn new(workspace_folders: Vec<PathBuf>) -> Self {
            Self { workspace_folders }
        }
    }

    impl PathsAccess for TestPaths {
        fn cache_dir(&self) -> PathBuf {
            PathBuf::new()
        }

        fn config_dir(&self) -> PathBuf {
            PathBuf::new()
        }

        fn workspace_folders(&self) -> Vec<PathBuf> {
            self.workspace_folders.clone()
        }
    }

    #[tokio::test]
    async fn test_assemble_new_chat_limits_message_budget_and_images() {
        use crate::scratchpads::multimodality::MultimodalElement;

        let paths = TestPaths::new(vec![]);
        let original_messages = vec![
            ChatMessage {
                role: "user".to_string(),
                content: ChatContent::SimpleText("baseline ".repeat(400)),
                ..Default::default()
            },
            ChatMessage {
                role: "user".to_string(),
                content: ChatContent::Multimodal(vec![
                    MultimodalElement {
                        m_type: "text".to_string(),
                        m_content: "first image context ".repeat(100),
                    },
                    MultimodalElement {
                        m_type: "image/png".to_string(),
                        m_content: "base64data-one".to_string(),
                    },
                ]),
                ..Default::default()
            },
            ChatMessage {
                role: "user".to_string(),
                content: ChatContent::Multimodal(vec![
                    MultimodalElement {
                        m_type: "text".to_string(),
                        m_content: "second image context ".repeat(100),
                    },
                    MultimodalElement {
                        m_type: "image/png".to_string(),
                        m_content: "base64data-two".to_string(),
                    },
                ]),
                ..Default::default()
            },
        ];
        let budget = calculate_transition_context_budget(&original_messages);
        let decisions = ParsedDecisions {
            summary: "summary ".repeat(200),
            messages_to_preserve: vec!["MSG_ID:1".to_string(), "MSG_ID:2".to_string()],
            handoff_message: "continue ".repeat(200),
            ..Default::default()
        };

        let new_messages = assemble_new_chat(&paths, &original_messages, &decisions)
            .await
            .unwrap();
        let message_symbols = new_messages
            .iter()
            .filter(|msg| msg.role != "context_file")
            .map(|msg| text_symbols(&msg.content.content_text_only()))
            .sum::<usize>();
        let image_count = count_images_in_messages(&new_messages);

        assert!(message_symbols <= budget.messages_symbols);
        assert_eq!(image_count, 2);
    }

    #[tokio::test]
    async fn test_assemble_new_chat_limits_file_budget() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("src/main.rs");
        std::fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        std::fs::write(
            &file_path,
            "fn main() { println!(\"hello\"); }\n".repeat(300),
        )
        .unwrap();

        let paths = TestPaths::new(vec![dir.path().to_path_buf()]);

        let original_messages = vec![
            ChatMessage {
                role: "context_file".to_string(),
                content: ChatContent::ContextFiles(vec![ContextFile {
                    file_name: "src/main.rs".to_string(),
                    file_content: "old content\n".repeat(300),
                    line1: 1,
                    line2: 300,
                    ..Default::default()
                }]),
                ..Default::default()
            },
            ChatMessage {
                role: "user".to_string(),
                content: ChatContent::SimpleText("requirements ".repeat(500)),
                ..Default::default()
            },
        ];
        let budget = calculate_transition_context_budget(&original_messages);
        let decisions = ParsedDecisions {
            files_to_open: vec!["src/main.rs".to_string()],
            ..Default::default()
        };

        let new_messages = assemble_new_chat(&paths, &original_messages, &decisions)
            .await
            .unwrap();
        let file_symbols = new_messages
            .iter()
            .filter(|msg| msg.role == "context_file")
            .map(|msg| text_symbols(&msg.content.content_text_only()))
            .sum::<usize>();

        assert!(file_symbols <= budget.files_symbols);
        assert!(file_symbols > 0);
    }
}
