#[cfg(test)]
mod tests {
    use crate::chat::snapshot_with_agents;
    use serde_json::json;
    use crate::agents::types::{AgentCompletion, BgAgentKind, BgAgentStatus, CreateAgentRequest};
    use crate::app_state::AppState;
    use crate::call_validation::{ChatMessage, ChatContent, ChatUsage, ChatToolCall, ChatToolFunction};
    use crate::scratchpads::multimodality::MultimodalElement;
    use crate::chat::types::{
        BackgroundAgentSummary, ChatEvent, ChatSession, DeltaOp, EventEnvelope, SessionState,
        PauseReason, QueuedItem, RuntimeState, ThreadParams,
    };
    use crate::exec::{ExecMode, ExecProcessId, ExecStatus, ProcessCompletionEvent};
    use std::collections::HashSet;

    fn extract_extra_fields(
        json_val: &serde_json::Value,
    ) -> serde_json::Map<String, serde_json::Value> {
        let mut result = serde_json::Map::new();
        if let Some(obj) = json_val.as_object() {
            for (key, val) in obj {
                if val.is_null() {
                    continue;
                }
                let dominated = key.starts_with("metering_")
                    || key.starts_with("billing_")
                    || key.starts_with("cost_")
                    || key.starts_with("cache_")
                    || key == "system_fingerprint";
                if dominated {
                    result.insert(key.clone(), val.clone());
                }
            }
        }
        if let Some(psf) = json_val.get("provider_specific_fields") {
            if !psf.is_null() {
                result.insert("provider_specific_fields".to_string(), psf.clone());
            }
        }
        result
    }

    fn background_agent_summary() -> BackgroundAgentSummary {
        BackgroundAgentSummary {
            agent_id: "bgagent-1".to_string(),
            parent_chat_id: "parent-chat".to_string(),
            child_chat_id: Some("child-chat".to_string()),
            kind: "delegate".to_string(),
            status: "waiting_for_approval".to_string(),
            title: "Patch frog pond".to_string(),
            progress: Some("Inspecting reeds".to_string()),
            step_count: 3,
            last_activity: Some("reading files".to_string()),
            target_files: vec!["src/frog.rs".to_string()],
            edited_files: vec!["src/frog.rs".to_string()],
            diff_summary: Some("one frog changed".to_string()),
            conflict_summary: None,
            result_summary: Some("frog patched".to_string()),
            error: None,
            started_at: Some("2026-05-27T00:00:00Z".to_string()),
            finished_at: None,
            change_seq: 7,
            ..BackgroundAgentSummary::default()
        }
    }

    fn create_agent_request(parent_chat_id: &str, title: &str) -> CreateAgentRequest {
        CreateAgentRequest {
            parent_chat_id: parent_chat_id.to_string(),
            parent_root_chat_id: Some(parent_chat_id.to_string()),
            parent_tool_call_id: Some(format!("tool-{title}")),
            kind: BgAgentKind::Delegate,
            config_name: "delegate".to_string(),
            title: title.to_string(),
            prompt: format!("prompt for {title}"),
            target_files: vec![format!("{title}.rs")],
            model: "test-model".to_string(),
            model_type: None,
            goal_summary: None,
            plan_present: false,
            worktree_id: None,
            worktree_branch: None,
        }
    }

    #[test]
    fn background_agent_updated_roundtrips() {
        let event = ChatEvent::BackgroundAgentUpdated {
            chat_id: "parent-chat".to_string(),
            seq: 11,
            agent: background_agent_summary(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["type"], "background_agent_updated");
        assert_eq!(value["agent"]["agentId"], "bgagent-1");

        let parsed: ChatEvent = serde_json::from_str(&json).unwrap();
        match parsed {
            ChatEvent::BackgroundAgentUpdated {
                chat_id,
                seq,
                agent,
            } => {
                assert_eq!(chat_id, "parent-chat");
                assert_eq!(seq, 11);
                assert_eq!(agent, background_agent_summary());
            }
            _ => panic!("Expected BackgroundAgentUpdated"),
        }
    }

    #[tokio::test]
    async fn test_snapshot_includes_background_agents() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let parent_chat_id = "parent-background-snapshot";
        let session = ChatSession::new(parent_chat_id.to_string());

        let (queued, _, _) = app
            .agents
            .create(create_agent_request(parent_chat_id, "queued"))
            .await
            .unwrap();
        let (running, _, _) = app
            .agents
            .create(create_agent_request(parent_chat_id, "running"))
            .await
            .unwrap();
        let (completed, _, _) = app
            .agents
            .create(create_agent_request(parent_chat_id, "completed"))
            .await
            .unwrap();
        let (interrupted, _, _) = app
            .agents
            .create(create_agent_request(parent_chat_id, "interrupted"))
            .await
            .unwrap();
        let (other_parent, _, _) = app
            .agents
            .create(create_agent_request("other-parent", "other"))
            .await
            .unwrap();

        app.agents
            .mark_running(&running.agent_id, "running-child".to_string())
            .await
            .unwrap();
        app.agents
            .mark_completed(
                &completed.agent_id,
                AgentCompletion {
                    result_summary: "done".to_string(),
                    edited_files: vec!["completed.rs".to_string()],
                    diff_summary: Some("diff".to_string()),
                    conflict_summary: None,
                    child_chat_id: Some("completed-child".to_string()),
                    deliverable_status: None,
                    handoff: None,
                },
            )
            .await
            .unwrap();
        app.agents
            .mark_interrupted(&interrupted.agent_id, "restart".to_string())
            .await
            .unwrap();

        let (snap, snapshot_background_agents) = snapshot_with_agents(app, &session).await;
        assert_eq!(snapshot_background_agents.len(), 4);

        match snap {
            ChatEvent::Snapshot {
                background_agents, ..
            } => {
                let statuses: HashSet<_> = background_agents
                    .iter()
                    .map(|agent| agent.status.as_str())
                    .collect();
                let agent_ids: HashSet<_> = background_agents
                    .iter()
                    .map(|agent| agent.agent_id.as_str())
                    .collect();

                assert_eq!(background_agents.len(), 4);
                assert!(agent_ids.contains(queued.agent_id.as_str()));
                assert!(agent_ids.contains(running.agent_id.as_str()));
                assert!(agent_ids.contains(completed.agent_id.as_str()));
                assert!(agent_ids.contains(interrupted.agent_id.as_str()));
                assert!(!agent_ids.contains(other_parent.agent_id.as_str()));
                assert!(statuses.contains(BgAgentStatus::Queued.as_str()));
                assert!(statuses.contains(BgAgentStatus::Running.as_str()));
                assert!(statuses.contains(BgAgentStatus::Completed.as_str()));
                assert!(statuses.contains(BgAgentStatus::Interrupted.as_str()));
            }
            _ => panic!("Expected Snapshot"),
        }
    }

    #[test]
    fn test_chat_message_roundtrip_all_fields() {
        let original = ChatMessage {
            message_id: "msg-123".to_string(),
            role: "assistant".to_string(),
            content: ChatContent::SimpleText("Hello world".to_string()),
            tool_calls: Some(vec![ChatToolCall {
                id: "call-1".to_string(),
                index: None,
                function: ChatToolFunction {
                    name: "test_tool".to_string(),
                    arguments: r#"{"arg": "value"}"#.to_string(),
                },
                tool_type: "function".to_string(),
                extra_content: None,
                started_at_ms: None,
                completed_at_ms: None,
            }]),
            tool_call_id: "".to_string(),
            tool_failed: None,
            preserve: Some(true),
            usage: Some(ChatUsage {
                prompt_tokens: 100,
                completion_tokens: 50,
                total_tokens: 150,
                cache_creation_tokens: None,
                cache_read_tokens: None,
                metering_usd: None,
            }),
            finish_reason: Some("stop".to_string()),
            reasoning_content: Some("I think therefore I am".to_string()),
            thinking_blocks: Some(vec![
                json!({"type": "thinking", "thinking": "deep thought"}),
            ]),
            citations: vec![json!({"url": "https://example.com", "title": "Example"})],
            server_content_blocks: vec![],
            extra: {
                let mut m = serde_json::Map::new();
                m.insert("custom_field".to_string(), json!("custom_value"));
                m
            },
            checkpoints: vec![],
            output_filter: None,
            summarized_range: None,
            summarization_tier: None,
            summarized_token_estimate: None,
        };

        let serialized = serde_json::to_value(&original).expect("serialize");
        let deserialized: ChatMessage =
            serde_json::from_value(serialized.clone()).expect("deserialize");

        assert_eq!(deserialized.message_id, original.message_id);
        assert_eq!(deserialized.role, original.role);
        assert_eq!(deserialized.finish_reason, original.finish_reason);
        assert_eq!(deserialized.reasoning_content, original.reasoning_content);
        assert_eq!(deserialized.preserve, Some(true));
        assert_eq!(serialized.get("preserve"), Some(&json!(true)));

        assert!(deserialized.tool_calls.is_some());
        let tc = deserialized.tool_calls.as_ref().unwrap();
        assert_eq!(tc.len(), 1);
        assert_eq!(tc[0].id, "call-1");
        assert_eq!(tc[0].function.name, "test_tool");

        assert!(deserialized.usage.is_some());
        let usage = deserialized.usage.as_ref().unwrap();
        assert_eq!(usage.prompt_tokens, 100);
        assert_eq!(usage.completion_tokens, 50);

        assert!(deserialized.thinking_blocks.is_some());
        assert_eq!(deserialized.thinking_blocks.as_ref().unwrap().len(), 1);

        assert_eq!(deserialized.citations.len(), 1);

        assert!(deserialized.extra.contains_key("custom_field"));
    }

    #[test]
    fn test_chat_message_roundtrip_multimodal_content() {
        let original = ChatMessage {
            message_id: "msg-mm".to_string(),
            role: "user".to_string(),
            content: ChatContent::Multimodal(vec![MultimodalElement::new(
                "text".to_string(),
                "Hello".to_string(),
            )
            .unwrap()]),
            ..Default::default()
        };

        let serialized = serde_json::to_value(&original).expect("serialize");
        let deserialized: ChatMessage = serde_json::from_value(serialized).expect("deserialize");

        match &deserialized.content {
            ChatContent::Multimodal(elements) => {
                assert_eq!(elements.len(), 1);
                assert_eq!(elements[0].m_type, "text");
                assert_eq!(elements[0].m_content, "Hello");
            }
            _ => panic!("Expected Multimodal content"),
        }
    }

    fn merge_tool_calls(existing: &mut Vec<ChatToolCall>, new_calls: &[serde_json::Value]) {
        for call_val in new_calls {
            let index = call_val
                .get("index")
                .and_then(|v| {
                    v.as_u64()
                        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                })
                .map(|i| i as usize);

            let id = call_val
                .get("id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let call_type = call_val
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("function")
                .to_string();

            let func = call_val.get("function");
            let name = func
                .and_then(|f| f.get("name"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let args = func
                .and_then(|f| f.get("arguments"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            if let Some(name) = name {
                let new_call = ChatToolCall {
                    id: id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
                    index,
                    function: ChatToolFunction {
                        name,
                        arguments: args,
                    },
                    tool_type: call_type,
                    extra_content: None,
                    started_at_ms: None,
                    completed_at_ms: None,
                };
                existing.push(new_call);
            } else if !args.is_empty() {
                if let Some(last) = existing.last_mut() {
                    last.function.arguments.push_str(&args);
                }
            }
        }
    }

    #[test]
    fn test_chat_event_serialization_stream_finished() {
        let event = ChatEvent::StreamFinished {
            message_id: "msg-123".to_string(),
            finish_reason: Some("abort".to_string()),
        };

        let json = serde_json::to_value(&event).expect("serialize");

        assert_eq!(json.get("type"), Some(&json!("stream_finished")));
        assert_eq!(json.get("message_id"), Some(&json!("msg-123")));
        assert_eq!(json.get("finish_reason"), Some(&json!("abort")));
    }

    #[test]
    fn test_chat_event_serialization_message_removed() {
        let event = ChatEvent::MessageRemoved {
            message_id: "msg-456".to_string(),
        };

        let json = serde_json::to_value(&event).expect("serialize");

        assert_eq!(json.get("type"), Some(&json!("message_removed")));
        assert_eq!(json.get("message_id"), Some(&json!("msg-456")));
    }

    #[test]
    fn test_chat_event_serialization_queue_updated() {
        let event = ChatEvent::QueueUpdated {
            queue_size: 2,
            queued_items: vec![QueuedItem {
                client_request_id: "req-1".to_string(),
                priority: false,
                command_type: "user_message".to_string(),
                preview: "Hello".to_string(),
                content: "Hello".to_string(),
                push: None,
                source: None,
                event: None,
                enqueued_at_ms: None,
            }],
        };

        let json = serde_json::to_value(&event).expect("serialize");

        assert_eq!(json.get("type"), Some(&json!("queue_updated")));
        assert_eq!(json.get("queue_size"), Some(&json!(2)));
        let items = json.get("queued_items").unwrap().as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].get("client_request_id"), Some(&json!("req-1")));
    }

    #[test]
    fn test_chat_event_serialization_pause_required() {
        let event = ChatEvent::PauseRequired {
            reasons: vec![PauseReason {
                reason_type: "confirmation".to_string(),
                tool_name: "shell".to_string(),
                command: "shell".to_string(),
                rule: "deny_all".to_string(),
                tool_call_id: "tc-1".to_string(),
                integr_config_path: None,
            }],
        };

        let json = serde_json::to_value(&event).expect("serialize");

        assert_eq!(json.get("type"), Some(&json!("pause_required")));
        let reasons = json.get("reasons").unwrap().as_array().unwrap();
        assert_eq!(reasons.len(), 1);
        assert_eq!(reasons[0].get("tool_call_id"), Some(&json!("tc-1")));
    }

    fn normalize_tool_call(tc: &serde_json::Value) -> Option<ChatToolCall> {
        let function = tc.get("function")?;
        let name = function
            .get("name")
            .and_then(|n| n.as_str())
            .filter(|s| !s.is_empty())?;

        let id = tc
            .get("id")
            .and_then(|i| i.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                format!(
                    "call_{}",
                    uuid::Uuid::new_v4().to_string().replace("-", "")[..24].to_string()
                )
            });

        let arguments = match function.get("arguments") {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(v) if !v.is_null() => serde_json::to_string(v).unwrap_or_default(),
            _ => String::new(),
        };

        let tool_type = tc
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("function")
            .to_string();

        let index = tc.get("index").and_then(|i| i.as_u64()).map(|i| i as usize);

        Some(ChatToolCall {
            id,
            index,
            function: ChatToolFunction {
                name: name.to_string(),
                arguments,
            },
            tool_type,
            extra_content: None,
            started_at_ms: None,
            completed_at_ms: None,
        })
    }

    fn is_thinking_enabled(
        sampling_parameters: &crate::call_validation::SamplingParameters,
    ) -> bool {
        sampling_parameters
            .thinking
            .as_ref()
            .and_then(|t| t.get("type"))
            .and_then(|t| t.as_str())
            .map(|t| t == "enabled")
            .unwrap_or(false)
            || sampling_parameters.reasoning_effort.is_some()
            || sampling_parameters.enable_thinking == Some(true)
    }

    #[test]
    fn hidden_roles_reconstruction_preserves_plan_and_archive() {
        let plan_content = "Keep this plan exactly intact.";
        let mut messages = vec![crate::chat::internal_roles::plan(
            "agent",
            1,
            plan_content,
            None,
        )];
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: ChatContent::SimpleText("first".to_string()),
            ..Default::default()
        });
        messages.push(ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText("assistant turn".to_string()),
            ..Default::default()
        });
        messages.push(crate::chat::internal_roles::event(
            crate::chat::internal_roles::EventSubkind::Tick,
            "smoke.tick",
            json!({ "tick": 1 }),
            "tick 1".to_string(),
        ));
        messages.push(ChatMessage {
            role: "user".to_string(),
            content: ChatContent::SimpleText("second".to_string()),
            ..Default::default()
        });

        let archive = serde_json::to_value(&messages).unwrap();
        let continuation = ChatMessage {
            message_id: "continuation".to_string(),
            role: "user".to_string(),
            content: ChatContent::SimpleText("Continue the approved work".to_string()),
            ..Default::default()
        };
        messages.push(
            refact_core::active_context::make_reconstruction_report(
                vec![messages[0].clone(), continuation],
                refact_core::active_context::ReconstructionMetadata::default(),
            )
            .unwrap(),
        );
        assert_eq!(
            serde_json::to_value(&messages[..messages.len() - 1]).unwrap(),
            archive
        );

        let plans: Vec<_> = messages
            .iter()
            .filter(|message| message.role == "plan")
            .collect();
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].content.content_text_only(), plan_content);
        let users: Vec<_> = messages
            .iter()
            .filter(|message| message.role == "user")
            .map(|message| message.content.content_text_only())
            .collect();
        assert_eq!(users, vec!["first", "second"]);
        let active = refact_core::active_context::active_context(&messages).unwrap();
        assert_eq!(active.messages.len(), 2);
        assert_eq!(active.messages[0].role, "plan");
        assert_eq!(active.messages[0].content.content_text_only(), plan_content);
    }

    #[tokio::test]
    async fn mode_switch_emits_event_not_user_message() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let mut session = ChatSession::new("mode-switch-test".to_string());
        session.thread.mode = "agent".to_string();
        session.add_message(ChatMessage {
            role: "user".to_string(),
            content: ChatContent::SimpleText("real user prompt".to_string()),
            ..Default::default()
        });
        let user_count_before = session.messages.iter().filter(|m| m.role == "user").count();
        let old_mode = session.thread.mode.clone();

        let patch = json!({"mode": "TASK_PLANNER"});
        let (changed, _) = crate::chat::queue::apply_setparams_patch(&mut session.thread, &patch);
        assert!(changed);
        crate::chat::queue::add_mode_switch_event_and_plan_if_changed(
            app,
            &mut session,
            &old_mode,
            None,
            "chat.session",
        )
        .await;

        let event_messages: Vec<_> = session
            .messages
            .iter()
            .filter(|m| {
                m.role == "event"
                    && m.extra
                        .get("event")
                        .and_then(|event| event.get("subkind"))
                        .and_then(|subkind| subkind.as_str())
                        == Some("mode_switch")
            })
            .collect();
        assert_eq!(event_messages.len(), 1);
        assert_eq!(
            session.messages.iter().filter(|m| m.role == "user").count(),
            user_count_before
        );

        let event = event_messages[0].extra.get("event").unwrap();
        assert_eq!(event["source"], json!("chat.session"));
        assert_eq!(event["payload"]["from"], json!("agent"));
        assert_eq!(event["payload"]["to"], json!("task_planner"));
        assert!(event["payload"]["reason"].is_null());
        assert_eq!(
            event["payload"]["diff"]["thread_defaults"]["auto_approve_dangerous_commands"],
            json!({"from": false, "to": true, "changed": true})
        );
        assert_eq!(
            event_messages[0].content.content_text_only(),
            "Mode switched: agent → task_planner"
        );
    }

    #[tokio::test]
    async fn mode_switch_to_agent_installs_plan_v1() {
        let temp = tempfile::tempdir().unwrap();
        let src_dir = temp.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        std::fs::write(src_dir.join("lib.rs"), "pub fn frog() {}\n").unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![temp.path().to_path_buf()];
        *gcx.documents_state.workspace_files.lock().unwrap() = vec![src_dir.join("lib.rs")];
        let app = AppState::from_gcx(gcx).await;
        let mut session = ChatSession::new("plan-agent-test".to_string());
        session.thread.mode = "explore".to_string();
        session.install_goal("explore", "Inspect and implement", true, Default::default());

        let old_mode = session.thread.mode.clone();
        let patch = json!({"mode": "agent"});
        let (changed, _) = crate::chat::queue::apply_setparams_patch(&mut session.thread, &patch);
        assert!(changed);
        crate::chat::queue::add_mode_switch_event_and_plan_if_changed(
            app,
            &mut session,
            &old_mode,
            None,
            "chat.session",
        )
        .await;

        let plan = session.goal.as_ref().unwrap().plan.as_ref().unwrap();
        assert_eq!(plan.mode, "agent");
        assert_eq!(plan.version, 1);
        assert!(crate::chat::plan_role::plan_history(&session).is_empty());
        let content = &plan.content;
        assert!(content.contains("Mode: Agent"));
        assert!(content.contains("src/"));
        assert!(!content.contains("%MODE_NAME%"));
        assert!(!content.contains("%PROJECT_TREE%"));
        assert!(!content.contains("%TASK_BRIEF%"));
    }

    #[tokio::test]
    async fn mode_switch_does_not_create_second_plan() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let mut session = ChatSession::new("plan-version-test".to_string());
        session.thread.mode = "explore".to_string();
        session.install_goal("explore", "Complete the task", true, Default::default());

        let old_mode = session.thread.mode.clone();
        let patch = json!({"mode": "agent"});
        let (changed, _) = crate::chat::queue::apply_setparams_patch(&mut session.thread, &patch);
        assert!(changed);
        crate::chat::queue::add_mode_switch_event_and_plan_if_changed(
            app.clone(),
            &mut session,
            &old_mode,
            None,
            "chat.session",
        )
        .await;

        let goal_id = crate::chat::goal_role::current_base_goal(&session)
            .unwrap()
            .message_id;
        session.add_message(crate::chat::internal_roles::goal_delta(
            "tool.update_goal",
            json!({"target":"plan","goal_id":goal_id,"seq": 1}),
            "keep this update",
        ));
        let before = session.goal.as_ref().unwrap().plan.clone().unwrap();
        let old_mode = session.thread.mode.clone();
        let patch = json!({"mode": "task_planner"});
        let (changed, _) = crate::chat::queue::apply_setparams_patch(&mut session.thread, &patch);
        assert!(changed);
        crate::chat::queue::add_mode_switch_event_and_plan_if_changed(
            app,
            &mut session,
            &old_mode,
            None,
            "chat.session",
        )
        .await;

        let chronological: Vec<_> = session
            .messages
            .iter()
            .filter(|message| message.role == "plan")
            .collect();
        assert!(chronological.is_empty());
        let plan = session.goal.as_ref().unwrap().plan.as_ref().unwrap();
        assert_eq!(plan.version, before.version);
        assert_eq!(plan.mode, before.mode);
        assert_eq!(plan.content, before.content);
        assert_eq!(plan.delta_count, 1);
        assert!(plan.content.contains("keep this update"));
    }

    #[tokio::test]
    async fn mode_without_plan_template_installs_no_plan() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let mut session = ChatSession::new("plan-none-test".to_string());
        session.thread.mode = "agent".to_string();

        let old_mode = session.thread.mode.clone();
        let patch = json!({"mode": "ask"});
        let (changed, _) = crate::chat::queue::apply_setparams_patch(&mut session.thread, &patch);
        assert!(changed);
        crate::chat::queue::add_mode_switch_event_and_plan_if_changed(
            app,
            &mut session,
            &old_mode,
            None,
            "chat.session",
        )
        .await;

        assert!(crate::chat::plan_role::plan_history(&session).is_empty());
        assert_eq!(
            session
                .messages
                .iter()
                .filter(|m| m.role == "event")
                .count(),
            1
        );
    }

    #[test]
    fn test_normalize_mode_id() {
        use crate::call_validation::normalize_mode_id;

        assert_eq!(normalize_mode_id("agent").unwrap(), "agent");
        assert_eq!(normalize_mode_id("AGENT").unwrap(), "agent");
        assert_eq!(normalize_mode_id("Agent").unwrap(), "agent");
        assert_eq!(normalize_mode_id("explore").unwrap(), "explore");
        assert_eq!(normalize_mode_id("task_planner").unwrap(), "task_planner");
        assert_eq!(normalize_mode_id("").unwrap(), "agent");
        assert!(normalize_mode_id("invalid!mode").is_err());
    }

    #[test]
    fn test_canonical_mode_id() {
        use crate::call_validation::canonical_mode_id;

        assert_eq!(canonical_mode_id("agent").unwrap(), "agent");
        assert_eq!(canonical_mode_id("AGENT").unwrap(), "agent");
        assert_eq!(canonical_mode_id("Agent").unwrap(), "agent");
        assert_eq!(canonical_mode_id("CONFIGURE").unwrap(), "configurator");
        assert_eq!(canonical_mode_id("configure").unwrap(), "configurator");
        assert_eq!(canonical_mode_id("CONFIGURATOR").unwrap(), "configurator");
        assert_eq!(canonical_mode_id("NO_TOOLS").unwrap(), "explore");
        assert_eq!(canonical_mode_id("no_tools").unwrap(), "explore");
        assert_eq!(canonical_mode_id("EXPLORE").unwrap(), "explore");
        assert_eq!(canonical_mode_id("TASK_PLANNER").unwrap(), "task_planner");
        assert_eq!(canonical_mode_id("task_planner").unwrap(), "task_planner");
        assert_eq!(canonical_mode_id("TASK_AGENT").unwrap(), "task_agent");
        assert_eq!(canonical_mode_id("task_agent").unwrap(), "task_agent");
        assert_eq!(canonical_mode_id("PLAN").unwrap(), "plan");
        assert_eq!(
            canonical_mode_id("my_custom_mode").unwrap(),
            "my_custom_mode"
        );
        assert_eq!(canonical_mode_id("").unwrap(), "agent");
        assert_eq!(canonical_mode_id("  ").unwrap(), "agent");
        assert!(canonical_mode_id("invalid!mode").is_err());
        assert!(canonical_mode_id(&"x".repeat(200)).is_err());
    }
}
