#[cfg(test)]
mod tests {
    use serde_json::json;
    use crate::agents::types::{AgentCompletion, BgAgentKind, BgAgentStatus, CreateAgentRequest};
    use crate::app_state::AppState;
    use crate::call_validation::{ChatMessage, ChatContent, ChatUsage, ChatToolCall, ChatToolFunction};
    use crate::scratchpads::multimodality::MultimodalElement;
    use crate::chat::types::{
        BackgroundAgentSummary, BurstGuard, BurstGuardDecision, ChatEvent, ChatSession, DeltaOp,
        EventEnvelope, SessionState, PauseReason, QueuedItem, RuntimeState, ThreadParams,
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
        }
    }

    #[tokio::test]
    async fn burst_guard_allows_first_five_calls() {
        let guard = BurstGuard::new();
        for _ in 0..5 {
            assert_eq!(guard.record_and_check().await, BurstGuardDecision::Allow);
        }
    }

    #[tokio::test]
    async fn burst_guard_defers_sixth_call() {
        let guard = BurstGuard::new();
        for _ in 0..5 {
            assert_eq!(guard.record_and_check().await, BurstGuardDecision::Allow);
        }

        assert_eq!(guard.record_and_check().await, BurstGuardDecision::Defer);
    }

    #[tokio::test]
    async fn burst_guard_allows_after_window_slides() {
        let guard = BurstGuard::new();
        for _ in 0..5 {
            assert_eq!(guard.record_and_check().await, BurstGuardDecision::Allow);
        }

        tokio::time::sleep(std::time::Duration::from_secs(11)).await;

        assert_eq!(guard.record_and_check().await, BurstGuardDecision::Allow);
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

    #[test]
    fn process_completed_emits_envelope_and_message_added() {
        let mut session = ChatSession::new("process-completed-chat".to_string());
        let mut rx = session.subscribe();

        crate::chat::notifications::inject_process_completion_message(
            &mut session,
            ProcessCompletionEvent {
                process_id: ExecProcessId("exec_done".to_string()),
                chat_id: "process-completed-chat".to_string(),
                status: ExecStatus::Exited { exit_code: Some(0) },
                exit_code: Some(0),
                duration_ms: Some(123),
                short_description: "test process".to_string(),
                mode: ExecMode::Background,
            },
        );

        let first_json = rx.try_recv().expect("message event");
        let first: EventEnvelope = serde_json::from_str(&first_json).unwrap();
        assert_eq!(first.seq, 1);
        match first.event {
            ChatEvent::MessageAdded { message, index } => {
                assert_eq!(index, 0);
                assert_eq!(message.role, "event");
                assert_eq!(
                    message.extra["event"]["subkind"],
                    json!("process_completed")
                );
                assert_eq!(
                    message.extra["event"]["payload"]["process_id"],
                    json!("exec_done")
                );
            }
            other => panic!("expected process-completed message_added, got {other:?}"),
        }

        let second_json = rx.try_recv().expect("process-completed envelope");
        let second: EventEnvelope = serde_json::from_str(&second_json).unwrap();
        assert_eq!(second.seq, 2);
        match second.event {
            ChatEvent::ProcessCompleted {
                process_id,
                status,
                exit_code,
                short_description,
                mode,
            } => {
                assert_eq!(process_id, "exec_done");
                assert_eq!(status, "exited");
                assert_eq!(exit_code, Some(0));
                assert_eq!(short_description, "test process");
                assert_eq!(mode, "background");
            }
            other => panic!("expected process-completed envelope, got {other:?}"),
        }
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn snapshot_roundtrips_with_background_agents() {
        let event = ChatEvent::Snapshot {
            thread: ThreadParams::default(),
            runtime: RuntimeState::default(),
            messages: vec![],
            background_agents: vec![background_agent_summary()],
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: ChatEvent = serde_json::from_str(&json).unwrap();
        match parsed {
            ChatEvent::Snapshot {
                background_agents, ..
            } => assert_eq!(background_agents, vec![background_agent_summary()]),
            _ => panic!("Expected Snapshot"),
        }
    }

    #[test]
    fn background_agent_summary_kind_and_status_are_snake_case() {
        let value = serde_json::to_value(background_agent_summary()).unwrap();
        assert_eq!(value["kind"], "delegate");
        assert_eq!(value["status"], "waiting_for_approval");
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
                },
            )
            .await
            .unwrap();
        app.agents
            .mark_interrupted(&interrupted.agent_id, "restart".to_string())
            .await
            .unwrap();

        let (snap, snapshot_background_agents) =
            ChatSession::snapshot_with_agents(app, &session).await;
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

    #[test]
    fn test_chat_message_empty_optional_fields() {
        let original = ChatMessage {
            message_id: "msg-empty".to_string(),
            role: "user".to_string(),
            content: ChatContent::SimpleText("Just text".to_string()),
            ..Default::default()
        };

        let serialized = serde_json::to_value(&original).expect("serialize");
        let deserialized: ChatMessage = serde_json::from_value(serialized).expect("deserialize");

        assert_eq!(deserialized.message_id, "msg-empty");
        assert!(deserialized.tool_calls.is_none());
        assert!(deserialized.usage.is_none());
        assert!(deserialized.reasoning_content.is_none());
        assert!(deserialized.thinking_blocks.is_none());
        assert!(deserialized.citations.is_empty());
    }

    #[test]
    fn test_chat_message_preserves_extra_unknown_keys() {
        let json_with_unknown = json!({
            "message_id": "msg-unk",
            "role": "assistant",
            "content": "test",
            "unknown_field_1": "value1",
            "unknown_field_2": 42,
            "nested_unknown": {"a": 1, "b": 2}
        });

        let deserialized: ChatMessage =
            serde_json::from_value(json_with_unknown).expect("deserialize");

        assert_eq!(deserialized.message_id, "msg-unk");
        assert!(
            deserialized.extra.contains_key("unknown_field_1")
                || deserialized.extra.contains_key("unknown_field_2")
                || deserialized.extra.contains_key("nested_unknown")
        );
    }

    #[test]
    fn test_chat_usage_roundtrip() {
        let usage = ChatUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: 150,
            cache_creation_tokens: None,
            cache_read_tokens: None,
            metering_usd: None,
        };

        let serialized = serde_json::to_value(&usage).expect("serialize");
        let deserialized: ChatUsage = serde_json::from_value(serialized).expect("deserialize");

        assert_eq!(deserialized.prompt_tokens, 100);
        assert_eq!(deserialized.completion_tokens, 50);
        assert_eq!(deserialized.total_tokens, 150);
    }

    #[test]
    fn test_extract_extra_metering_fields() {
        let json = json!({
            "metering_prompt_tokens_n": 50,
            "metering_generated_tokens_n": 25,
            "other_field": "ignored"
        });

        let extra = extract_extra_fields(&json);

        assert_eq!(extra.get("metering_prompt_tokens_n"), Some(&json!(50)));
        assert_eq!(extra.get("metering_generated_tokens_n"), Some(&json!(25)));
        assert!(extra.get("other_field").is_none());
    }

    #[test]
    fn test_extract_extra_new_metering_fields() {
        let json = json!({
            "metering_new_field_2025": 999,
            "metering_another_new": "value"
        });

        let extra = extract_extra_fields(&json);

        assert_eq!(extra.get("metering_new_field_2025"), Some(&json!(999)));
        assert_eq!(extra.get("metering_another_new"), Some(&json!("value")));
    }

    #[test]
    fn test_extract_extra_billing_cost_cache_fields() {
        let json = json!({
            "billing_total": 1.5,
            "cost_per_token": 0.001,
            "cache_hit": true
        });

        let extra = extract_extra_fields(&json);

        assert_eq!(extra.get("billing_total"), Some(&json!(1.5)));
        assert_eq!(extra.get("cost_per_token"), Some(&json!(0.001)));
        assert_eq!(extra.get("cache_hit"), Some(&json!(true)));
    }

    #[test]
    fn test_extract_extra_system_fingerprint() {
        let json = json!({
            "system_fingerprint": "fp_abc123",
            "id": "ignored"
        });

        let extra = extract_extra_fields(&json);

        assert_eq!(extra.get("system_fingerprint"), Some(&json!("fp_abc123")));
        assert!(extra.get("id").is_none());
    }

    #[test]
    fn test_extract_extra_provider_specific_fields() {
        let json = json!({
            "provider_specific_fields": {
                "custom_field": "value",
                "nested": {"a": 1}
            }
        });

        let extra = extract_extra_fields(&json);

        let psf = extra.get("provider_specific_fields").unwrap();
        assert_eq!(psf.get("custom_field"), Some(&json!("value")));
    }

    #[test]
    fn test_extract_extra_null_values_ignored() {
        let json = json!({
            "metering_tokens": 100
        });

        let extra = extract_extra_fields(&json);

        assert_eq!(extra.get("metering_tokens"), Some(&json!(100)));
    }

    #[test]
    fn test_extract_extra_empty_object() {
        let json = json!({});
        let extra = extract_extra_fields(&json);
        assert!(extra.is_empty());
    }

    #[test]
    fn test_extract_extra_combined() {
        let json = json!({
            "billing_amount": 5.0,
            "cost_total": 0.05,
            "cache_status": "hit",
            "system_fingerprint": "fp_123",
            "provider_specific_fields": {"x": 1},
            "ignored_field": "nope",
            "choices": [{"delta": {}}]
        });

        let extra = extract_extra_fields(&json);

        assert_eq!(extra.len(), 5);
        assert!(extra.contains_key("billing_amount"));
        assert!(extra.contains_key("cost_total"));
        assert!(extra.contains_key("cache_status"));
        assert!(extra.contains_key("system_fingerprint"));
        assert!(extra.contains_key("provider_specific_fields"));
        assert!(!extra.contains_key("ignored_field"));
        assert!(!extra.contains_key("choices"));
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
    fn test_merge_tool_calls_new_call_with_name() {
        let mut existing = Vec::new();
        let new_calls = vec![json!({
            "id": "call-1",
            "type": "function",
            "function": {
                "name": "test_tool",
                "arguments": "{\"a\": 1}"
            }
        })];

        merge_tool_calls(&mut existing, &new_calls);

        assert_eq!(existing.len(), 1);
        assert_eq!(existing[0].id, "call-1");
        assert_eq!(existing[0].function.name, "test_tool");
        assert_eq!(existing[0].function.arguments, "{\"a\": 1}");
    }

    #[test]
    fn test_merge_tool_calls_argument_continuation() {
        let mut existing = vec![ChatToolCall {
            id: "call-1".to_string(),
            index: Some(0),
            function: ChatToolFunction {
                name: "test_tool".to_string(),
                arguments: "{\"a\":".to_string(),
            },
            tool_type: "function".to_string(),
            extra_content: None,
        }];

        let new_calls = vec![json!({
            "function": {
                "arguments": " 1}"
            }
        })];

        merge_tool_calls(&mut existing, &new_calls);

        assert_eq!(existing.len(), 1);
        assert_eq!(existing[0].function.arguments, "{\"a\": 1}");
    }

    #[test]
    fn test_merge_tool_calls_missing_id_generates_uuid() {
        let mut existing = Vec::new();
        let new_calls = vec![json!({
            "function": {
                "name": "no_id_tool",
                "arguments": "{}"
            }
        })];

        merge_tool_calls(&mut existing, &new_calls);

        assert_eq!(existing.len(), 1);
        assert!(!existing[0].id.is_empty());
        assert!(existing[0].id.len() > 10);
    }

    #[test]
    fn test_merge_tool_calls_missing_type_defaults_to_function() {
        let mut existing = Vec::new();
        let new_calls = vec![json!({
            "id": "call-1",
            "function": {
                "name": "test",
                "arguments": "{}"
            }
        })];

        merge_tool_calls(&mut existing, &new_calls);

        assert_eq!(existing[0].tool_type, "function");
    }

    #[test]
    fn test_merge_tool_calls_index_as_string() {
        let mut existing = Vec::new();
        let new_calls = vec![json!({
            "index": "1",
            "id": "call-1",
            "function": {
                "name": "test",
                "arguments": "{}"
            }
        })];

        merge_tool_calls(&mut existing, &new_calls);

        assert_eq!(existing[0].index, Some(1));
    }

    #[test]
    fn test_merge_tool_calls_multiple_calls() {
        let mut existing = Vec::new();
        let new_calls = vec![
            json!({
                "index": 0,
                "id": "call-0",
                "function": {"name": "tool_a", "arguments": "{}"}
            }),
            json!({
                "index": 1,
                "id": "call-1",
                "function": {"name": "tool_b", "arguments": "{}"}
            }),
        ];

        merge_tool_calls(&mut existing, &new_calls);

        assert_eq!(existing.len(), 2);
        assert_eq!(existing[0].function.name, "tool_a");
        assert_eq!(existing[1].function.name, "tool_b");
    }

    #[test]
    fn test_merge_tool_calls_empty_arguments_only_ignored() {
        let mut existing = vec![ChatToolCall {
            id: "call-1".to_string(),
            index: Some(0),
            function: ChatToolFunction {
                name: "test".to_string(),
                arguments: "{}".to_string(),
            },
            tool_type: "function".to_string(),
            extra_content: None,
        }];

        let new_calls = vec![json!({
            "function": {
                "arguments": ""
            }
        })];

        merge_tool_calls(&mut existing, &new_calls);

        assert_eq!(existing.len(), 1);
        assert_eq!(existing[0].function.arguments, "{}");
    }

    #[test]
    fn test_delta_op_append_content() {
        let ops = vec![
            DeltaOp::AppendContent {
                text: "Hello ".to_string(),
            },
            DeltaOp::AppendContent {
                text: "world".to_string(),
            },
        ];

        let mut content = String::new();
        for op in ops {
            if let DeltaOp::AppendContent { text } = op {
                content.push_str(&text);
            }
        }

        assert_eq!(content, "Hello world");
    }

    #[test]
    fn test_delta_op_append_reasoning() {
        let ops = vec![
            DeltaOp::AppendReasoning {
                text: "First ".to_string(),
            },
            DeltaOp::AppendReasoning {
                text: "thought".to_string(),
            },
        ];

        let mut reasoning = String::new();
        for op in ops {
            if let DeltaOp::AppendReasoning { text } = op {
                reasoning.push_str(&text);
            }
        }

        assert_eq!(reasoning, "First thought");
    }

    #[test]
    fn test_delta_op_merge_extra_preserves_existing() {
        let mut extra = serde_json::Map::new();
        extra.insert("existing".to_string(), json!("value"));

        let op = DeltaOp::MergeExtra {
            extra: {
                let mut m = serde_json::Map::new();
                m.insert("new_field".to_string(), json!(123));
                m
            },
        };

        if let DeltaOp::MergeExtra { extra: new_extra } = op {
            extra.extend(new_extra);
        }

        assert_eq!(extra.get("existing"), Some(&json!("value")));
        assert_eq!(extra.get("new_field"), Some(&json!(123)));
    }

    #[test]
    fn test_delta_op_merge_extra_successive_updates() {
        let mut extra = serde_json::Map::new();

        let ops = vec![
            DeltaOp::MergeExtra {
                extra: {
                    let mut m = serde_json::Map::new();
                    m.insert("metering_a".to_string(), json!(1));
                    m
                },
            },
            DeltaOp::MergeExtra {
                extra: {
                    let mut m = serde_json::Map::new();
                    m.insert("metering_b".to_string(), json!(2));
                    m
                },
            },
            DeltaOp::MergeExtra {
                extra: {
                    let mut m = serde_json::Map::new();
                    m.insert("metering_a".to_string(), json!(10));
                    m
                },
            },
        ];

        for op in ops {
            if let DeltaOp::MergeExtra { extra: new_extra } = op {
                extra.extend(new_extra);
            }
        }

        assert_eq!(extra.get("metering_a"), Some(&json!(10)));
        assert_eq!(extra.get("metering_b"), Some(&json!(2)));
    }

    #[test]
    fn test_delta_op_merge_extra_does_not_overwrite_core_fields() {
        let mut msg = ChatMessage {
            message_id: "msg-1".to_string(),
            role: "assistant".to_string(),
            content: ChatContent::SimpleText("Hello".to_string()),
            ..Default::default()
        };

        let dangerous_extra = {
            let mut m = serde_json::Map::new();
            m.insert("content".to_string(), json!("OVERWRITTEN"));
            m.insert("role".to_string(), json!("hacker"));
            m.insert("message_id".to_string(), json!("fake-id"));
            m.insert("metering_safe".to_string(), json!(100));
            m
        };

        msg.extra.extend(dangerous_extra);

        assert_eq!(msg.message_id, "msg-1");
        assert_eq!(msg.role, "assistant");
        match &msg.content {
            ChatContent::SimpleText(s) => assert_eq!(s, "Hello"),
            _ => panic!("Content type changed"),
        }
        assert_eq!(msg.extra.get("metering_safe"), Some(&json!(100)));
    }

    #[test]
    fn test_session_state_transitions() {
        assert_eq!(format!("{:?}", SessionState::Idle), "Idle");
        assert_eq!(format!("{:?}", SessionState::Generating), "Generating");
        assert_eq!(format!("{:?}", SessionState::Paused), "Paused");
        assert_eq!(
            format!("{:?}", SessionState::ExecutingTools),
            "ExecutingTools"
        );
        assert_eq!(format!("{:?}", SessionState::Error), "Error");
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

    #[test]
    fn test_chat_event_serialization_pause_cleared() {
        let event = ChatEvent::PauseCleared {};

        let json = serde_json::to_value(&event).expect("serialize");

        assert_eq!(json.get("type"), Some(&json!("pause_cleared")));
    }

    #[test]
    fn test_normalize_tool_call_valid_complete() {
        let tc = json!({
            "id": "call_abc123",
            "index": 0,
            "type": "function",
            "function": {
                "name": "test_tool",
                "arguments": "{\"key\": \"value\"}"
            }
        });

        let result = normalize_tool_call(&tc);
        assert!(result.is_some());

        let call = result.unwrap();
        assert_eq!(call.id, "call_abc123");
        assert_eq!(call.index, Some(0));
        assert_eq!(call.function.name, "test_tool");
        assert_eq!(call.function.arguments, "{\"key\": \"value\"}");
        assert_eq!(call.tool_type, "function");
    }

    #[test]
    fn test_normalize_tool_call_missing_id_generates_uuid() {
        let tc = json!({
            "function": {
                "name": "test_tool",
                "arguments": "{}"
            }
        });

        let result = normalize_tool_call(&tc);
        assert!(result.is_some());

        let call = result.unwrap();
        assert!(call.id.starts_with("call_"));
        assert!(call.id.len() >= 20);
    }

    #[test]
    fn test_normalize_tool_call_missing_type_defaults_function() {
        let tc = json!({
            "id": "call_123",
            "function": {
                "name": "my_tool",
                "arguments": "{}"
            }
        });

        let result = normalize_tool_call(&tc);
        assert!(result.is_some());
        assert_eq!(result.unwrap().tool_type, "function");
    }

    #[test]
    fn test_normalize_tool_call_arguments_as_object() {
        let tc = json!({
            "id": "call_123",
            "function": {
                "name": "my_tool",
                "arguments": {"nested": "object", "num": 42}
            }
        });

        let result = normalize_tool_call(&tc);
        assert!(result.is_some());

        let call = result.unwrap();
        assert!(call.function.arguments.contains("nested"));
        assert!(call.function.arguments.contains("42"));
    }

    #[test]
    fn test_normalize_tool_call_missing_arguments() {
        let tc = json!({
            "id": "call_123",
            "function": {
                "name": "my_tool"
            }
        });

        let result = normalize_tool_call(&tc);
        assert!(result.is_some());
        assert_eq!(result.unwrap().function.arguments, "");
    }

    #[test]
    fn test_normalize_tool_call_null_arguments() {
        let tc = json!({
            "id": "call_123",
            "function": {
                "name": "my_tool",
                "arguments": null
            }
        });

        let result = normalize_tool_call(&tc);
        assert!(result.is_some());
        assert_eq!(result.unwrap().function.arguments, "");
    }

    #[test]
    fn test_normalize_tool_call_missing_name_returns_none() {
        let tc = json!({
            "id": "call_123",
            "function": {
                "arguments": "{}"
            }
        });

        let result = normalize_tool_call(&tc);
        assert!(result.is_none());
    }

    #[test]
    fn test_normalize_tool_call_empty_name_returns_none() {
        let tc = json!({
            "id": "call_123",
            "function": {
                "name": "",
                "arguments": "{}"
            }
        });

        let result = normalize_tool_call(&tc);
        assert!(result.is_none());
    }

    #[test]
    fn test_normalize_tool_call_missing_function_returns_none() {
        let tc = json!({
            "id": "call_123",
            "type": "function"
        });

        let result = normalize_tool_call(&tc);
        assert!(result.is_none());
    }

    #[test]
    fn test_normalize_tool_call_index_preserved() {
        let tc = json!({
            "id": "call_123",
            "index": 5,
            "function": {
                "name": "indexed_tool",
                "arguments": "{}"
            }
        });

        let result = normalize_tool_call(&tc);
        assert!(result.is_some());
        assert_eq!(result.unwrap().index, Some(5));
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
        })
    }

    #[test]
    fn test_chat_prepare_options_default() {
        use crate::chat::prepare::ChatPrepareOptions;

        let opts = ChatPrepareOptions::default();

        assert!(opts.prepend_system_prompt);
        assert!(opts.allow_at_commands);
        assert!(opts.allow_tool_prerun);
        assert!(opts.supports_tools);
    }

    #[test]
    fn test_chat_prepare_options_custom() {
        use crate::chat::prepare::ChatPrepareOptions;

        let opts = ChatPrepareOptions {
            prepend_system_prompt: false,
            allow_at_commands: false,
            allow_tool_prerun: false,
            supports_tools: true,
            tool_choice: None,
            parallel_tool_calls: None,
            ..Default::default()
        };

        assert!(!opts.prepend_system_prompt);
        assert!(!opts.allow_at_commands);
        assert!(!opts.allow_tool_prerun);
        assert!(opts.supports_tools);
    }

    #[test]
    fn test_is_thinking_enabled_with_thinking_json() {
        use crate::call_validation::SamplingParameters;

        let params = SamplingParameters {
            thinking: Some(json!({"type": "enabled", "budget_tokens": 1024})),
            ..Default::default()
        };

        assert!(is_thinking_enabled(&params));
    }

    #[test]
    fn test_is_thinking_enabled_with_thinking_disabled() {
        use crate::call_validation::SamplingParameters;

        let params = SamplingParameters {
            thinking: Some(json!({"type": "disabled"})),
            ..Default::default()
        };

        assert!(!is_thinking_enabled(&params));
    }

    #[test]
    fn test_is_thinking_enabled_with_reasoning_effort() {
        use crate::call_validation::{SamplingParameters, ReasoningEffort};

        let params = SamplingParameters {
            reasoning_effort: Some(ReasoningEffort::Medium),
            ..Default::default()
        };

        assert!(is_thinking_enabled(&params));
    }

    #[test]
    fn test_is_thinking_enabled_with_enable_thinking_true() {
        use crate::call_validation::SamplingParameters;

        let params = SamplingParameters {
            enable_thinking: Some(true),
            ..Default::default()
        };

        assert!(is_thinking_enabled(&params));
    }

    #[test]
    fn test_is_thinking_enabled_with_enable_thinking_false() {
        use crate::call_validation::SamplingParameters;

        let params = SamplingParameters {
            enable_thinking: Some(false),
            ..Default::default()
        };

        assert!(!is_thinking_enabled(&params));
    }

    #[test]
    fn test_is_thinking_enabled_all_none() {
        use crate::call_validation::SamplingParameters;

        let params = SamplingParameters::default();

        assert!(!is_thinking_enabled(&params));
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
    fn test_strip_thinking_blocks_removes_when_disabled() {
        let messages = vec![ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText("Hello".to_string()),
            thinking_blocks: Some(vec![
                json!({"type": "thinking", "thinking": "deep thought"}),
            ]),
            ..Default::default()
        }];

        let stripped: Vec<_> = messages
            .into_iter()
            .map(|mut msg| {
                msg.thinking_blocks = None;
                msg
            })
            .collect();

        assert!(stripped[0].thinking_blocks.is_none());
    }

    #[test]
    fn test_strip_thinking_blocks_preserves_content() {
        let messages = vec![ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText("Hello world".to_string()),
            thinking_blocks: Some(vec![json!({"type": "thinking", "thinking": "thought"})]),
            reasoning_content: Some("reasoning".to_string()),
            ..Default::default()
        }];

        let stripped: Vec<_> = messages
            .into_iter()
            .map(|mut msg| {
                msg.thinking_blocks = None;
                msg
            })
            .collect();

        match &stripped[0].content {
            ChatContent::SimpleText(s) => assert_eq!(s, "Hello world"),
            _ => panic!("Content type changed"),
        }
        assert_eq!(stripped[0].reasoning_content, Some("reasoning".to_string()));
    }

    #[test]
    fn test_tools_json_not_null_when_empty() {
        let tools: Vec<serde_json::Value> = vec![];
        let tools_str = if tools.is_empty() {
            None
        } else {
            serde_json::to_string(&tools).ok()
        };

        assert!(tools_str.is_none());
    }

    #[test]
    fn test_tools_json_serializes_when_present() {
        let tools = vec![json!({"type": "function", "function": {"name": "test"}})];
        let tools_str = if tools.is_empty() {
            None
        } else {
            serde_json::to_string(&tools).ok()
        };

        assert!(tools_str.is_some());
        assert!(tools_str.unwrap().contains("test"));
    }

    #[test]
    fn test_tool_names_filtering() {
        use std::collections::HashSet;

        let all_tool_names = vec!["tool_a", "tool_b", "tool_c", "tool_d"];
        let allowed: HashSet<String> = vec!["tool_a".to_string(), "tool_c".to_string()]
            .into_iter()
            .collect();

        let filtered: Vec<_> = all_tool_names
            .into_iter()
            .filter(|name| allowed.contains(*name))
            .collect();

        assert_eq!(filtered.len(), 2);
        assert!(filtered.contains(&"tool_a"));
        assert!(filtered.contains(&"tool_c"));
        assert!(!filtered.contains(&"tool_b"));
    }

    #[test]
    fn test_prompt_tool_names_empty_when_at_commands_disabled() {
        use std::collections::HashSet;

        let tool_names: HashSet<String> = vec!["tool_a".to_string(), "tool_b".to_string()]
            .into_iter()
            .collect();
        let allow_at_commands = false;

        let prompt_tool_names = if allow_at_commands {
            tool_names.clone()
        } else {
            HashSet::new()
        };

        assert!(prompt_tool_names.is_empty());
    }

    #[test]
    fn test_prompt_tool_names_preserved_when_at_commands_enabled() {
        use std::collections::HashSet;

        let tool_names: HashSet<String> = vec!["tool_a".to_string(), "tool_b".to_string()]
            .into_iter()
            .collect();
        let allow_at_commands = true;

        let prompt_tool_names = if allow_at_commands {
            tool_names.clone()
        } else {
            HashSet::new()
        };

        assert_eq!(prompt_tool_names.len(), 2);
        assert!(prompt_tool_names.contains("tool_a"));
    }

    #[test]
    fn test_tool_step_outcome_variants() {
        use crate::chat::tools::ToolStepOutcome;

        let no_tools = ToolStepOutcome::NoToolCalls;
        let paused = ToolStepOutcome::Paused;
        let cont = ToolStepOutcome::Continue;

        assert!(matches!(no_tools, ToolStepOutcome::NoToolCalls));
        assert!(matches!(paused, ToolStepOutcome::Paused));
        assert!(matches!(cont, ToolStepOutcome::Continue));
    }

    #[test]
    fn test_tool_step_outcome_in_match() {
        use crate::chat::tools::ToolStepOutcome;

        fn should_continue(outcome: ToolStepOutcome) -> bool {
            match outcome {
                ToolStepOutcome::NoToolCalls => false,
                ToolStepOutcome::Paused => false,
                ToolStepOutcome::Continue => true,
                ToolStepOutcome::Stop => false,
            }
        }

        assert!(!should_continue(ToolStepOutcome::NoToolCalls));
        assert!(!should_continue(ToolStepOutcome::Paused));
        assert!(should_continue(ToolStepOutcome::Continue));
    }

    #[test]
    fn test_iterative_loop_simulation() {
        use crate::chat::tools::ToolStepOutcome;

        const MAX_CYCLES: usize = 50;

        fn simulate_agent_loop(outcomes: &[ToolStepOutcome]) -> usize {
            let mut cycles = 0;
            for cycle in 0..MAX_CYCLES {
                cycles = cycle + 1;
                if cycle >= outcomes.len() {
                    break;
                }
                match &outcomes[cycle] {
                    ToolStepOutcome::NoToolCalls => break,
                    ToolStepOutcome::Paused => break,
                    ToolStepOutcome::Continue => continue,
                    ToolStepOutcome::Stop => break,
                }
            }
            cycles
        }

        assert_eq!(simulate_agent_loop(&[ToolStepOutcome::NoToolCalls]), 1);
        assert_eq!(simulate_agent_loop(&[ToolStepOutcome::Paused]), 1);
        assert_eq!(
            simulate_agent_loop(&[ToolStepOutcome::Continue, ToolStepOutcome::NoToolCalls]),
            2
        );
        assert_eq!(
            simulate_agent_loop(&[
                ToolStepOutcome::Continue,
                ToolStepOutcome::Continue,
                ToolStepOutcome::Continue,
                ToolStepOutcome::Paused
            ]),
            4
        );

        let many_continues: Vec<_> = (0..100).map(|_| ToolStepOutcome::Continue).collect();
        assert_eq!(simulate_agent_loop(&many_continues), MAX_CYCLES);
    }

    #[test]
    fn test_abort_breaks_loop_simulation() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        const MAX_CYCLES: usize = 50;

        fn simulate_with_abort(abort_at: Option<usize>) -> usize {
            let abort_flag = Arc::new(AtomicBool::new(false));
            let mut cycles = 0;

            for cycle in 0..MAX_CYCLES {
                if abort_flag.load(Ordering::SeqCst) {
                    break;
                }
                cycles = cycle + 1;

                if Some(cycle) == abort_at {
                    abort_flag.store(true, Ordering::SeqCst);
                }
            }
            cycles
        }

        assert_eq!(simulate_with_abort(None), MAX_CYCLES);
        assert_eq!(simulate_with_abort(Some(0)), 1);
        assert_eq!(simulate_with_abort(Some(5)), 6);
        assert_eq!(simulate_with_abort(Some(10)), 11);
    }

    #[test]
    fn test_server_executed_tool_filtering() {
        fn is_server_executed_tool(tool_call_id: &str) -> bool {
            tool_call_id.starts_with("srvtoolu_")
        }

        let tool_calls = vec![
            ("call_abc123", false),
            ("srvtoolu_xyz789", true),
            ("toolu_def456", false),
            ("srvtoolu_", true),
        ];

        for (id, expected) in tool_calls {
            assert_eq!(
                is_server_executed_tool(id),
                expected,
                "Failed for id: {}",
                id
            );
        }

        let all_calls = vec!["call_1", "srvtoolu_2", "call_3", "srvtoolu_4"];
        let client_calls: Vec<_> = all_calls
            .into_iter()
            .filter(|id| !is_server_executed_tool(id))
            .collect();

        assert_eq!(client_calls, vec!["call_1", "call_3"]);
    }

    #[test]
    fn test_no_tool_calls_when_last_message_not_assistant() {
        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: ChatContent::SimpleText("Hello".to_string()),
            ..Default::default()
        }];

        let last_msg = messages.last();
        let has_tool_calls = match last_msg {
            Some(m) if m.role == "assistant" && m.tool_calls.is_some() => true,
            _ => false,
        };

        assert!(!has_tool_calls);
    }

    #[test]
    fn test_no_tool_calls_when_assistant_has_none() {
        let messages = vec![ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText("Hello".to_string()),
            tool_calls: None,
            ..Default::default()
        }];

        let last_msg = messages.last();
        let has_tool_calls = match last_msg {
            Some(m) if m.role == "assistant" && m.tool_calls.is_some() => true,
            _ => false,
        };

        assert!(!has_tool_calls);
    }

    #[test]
    fn test_has_tool_calls_when_assistant_has_calls() {
        let messages = vec![ChatMessage {
            role: "assistant".to_string(),
            content: ChatContent::SimpleText("Let me help".to_string()),
            tool_calls: Some(vec![ChatToolCall {
                id: "call_123".to_string(),
                index: Some(0),
                function: ChatToolFunction {
                    name: "test_tool".to_string(),
                    arguments: "{}".to_string(),
                },
                tool_type: "function".to_string(),
                extra_content: None,
            }]),
            ..Default::default()
        }];

        let last_msg = messages.last();
        let has_tool_calls = match last_msg {
            Some(m) if m.role == "assistant" && m.tool_calls.is_some() => true,
            _ => false,
        };

        assert!(has_tool_calls);
    }

    #[test]
    fn test_empty_tool_calls_after_server_filter() {
        fn is_server_executed_tool(id: &str) -> bool {
            id.starts_with("srvtoolu_")
        }

        let tool_calls = vec![
            ChatToolCall {
                id: "srvtoolu_1".to_string(),
                index: Some(0),
                function: ChatToolFunction {
                    name: "server_tool".to_string(),
                    arguments: "{}".to_string(),
                },
                tool_type: "function".to_string(),
                extra_content: None,
            },
            ChatToolCall {
                id: "srvtoolu_2".to_string(),
                index: Some(1),
                function: ChatToolFunction {
                    name: "another_server_tool".to_string(),
                    arguments: "{}".to_string(),
                },
                tool_type: "function".to_string(),
                extra_content: None,
            },
        ];

        let client_calls: Vec<_> = tool_calls
            .iter()
            .filter(|tc| !is_server_executed_tool(&tc.id))
            .collect();

        assert!(client_calls.is_empty());
    }

    #[test]
    fn hidden_roles_compression_preserves_plan_and_summarizes_assistant_segment() {
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

        assert!(
            crate::chat::summarization::summarize_oldest_segment_with_static_summary(
                &mut messages,
                "summary",
                "test",
            )
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
        assert!(messages
            .iter()
            .any(crate::chat::summarization::is_segment_summary));
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

        let patch = json!({"mode": "TASK_AGENT"});
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
        assert_eq!(
            event["payload"],
            json!({"from": "agent", "to": "task_agent", "reason": null})
        );
        assert_eq!(
            event_messages[0].content.content_text_only(),
            "Mode switched: agent → task_agent"
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

        let plans = crate::chat::plan_role::plan_history(&session);
        assert_eq!(plans.len(), 1);
        let plan = plans[0];
        assert_eq!(plan.extra["plan"]["mode"], json!("agent"));
        assert_eq!(plan.extra["plan"]["version"], json!(1));
        assert!(plan.extra["plan"]["supersedes"].is_null());
        let content = plan.content.content_text_only();
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

        session.add_message(crate::chat::internal_roles::plan_delta(
            "tool.set_plan",
            json!({"seq": 1}),
            "keep this update",
        ));
        let base_plan_id = crate::chat::plan_role::current_base_plan(&session)
            .unwrap()
            .message_id
            .clone();
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
        assert_eq!(chronological.len(), 1);
        assert_eq!(chronological[0].message_id, base_plan_id);
        assert_eq!(chronological[0].extra["plan"]["version"], json!(1));
        assert_eq!(chronological[0].extra["plan"]["mode"], json!("agent"));
        let deltas = crate::chat::plan_role::plan_delta_events(&session);
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].content.content_text_only(), "keep this update");
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

#[test]
fn cache_guard_emits_event_not_user_message() {
    let message = crate::chat::cache_guard::cache_guard_event_message(
        serde_json::json!({"model": "test/model", "probe": "cache_warm"}),
        "cache guard probe",
    );

    assert_eq!(message.role, "event");
    assert_ne!(message.role, "user");
    let event = message.extra.get("event").unwrap();
    assert_eq!(event["subkind"], serde_json::json!("system_notice"));
    assert_eq!(event["source"], serde_json::json!("chat.cache_guard"));
    assert_eq!(event["payload"]["probe"], serde_json::json!("cache_warm"));
    assert_eq!(message.content.content_text_only(), "cache guard probe");
}

#[test]
fn summarization_marker_is_event_not_user_message() {
    let mut marker = crate::chat::internal_roles::event(
        crate::chat::internal_roles::EventSubkind::SummarizationMarker,
        "chat.summarizer",
        serde_json::json!({
            "tokens_before": 1200,
            "tokens_after": 300,
            "messages_compacted": 4,
        }),
        "compacted 4 msgs",
    );
    marker.summarized_range = Some((1, 4));
    marker.summarization_tier = Some("llm_segment_summary".to_string());
    marker.summarized_token_estimate = Some(1200);

    assert_eq!(marker.role, "event");
    assert_ne!(marker.role, "user");
    let event = marker.extra.get("event").unwrap();
    assert_eq!(event["subkind"], serde_json::json!("summarization_marker"));
    assert_eq!(event["source"], serde_json::json!("chat.summarizer"));
    assert_eq!(event["payload"]["messages_compacted"], serde_json::json!(4));
    assert_eq!(marker.content.content_text_only(), "compacted 4 msgs");
}
