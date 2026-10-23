use chrono::{DateTime, Utc};
use refact_agents_core::{
    BackgroundAgent, BackgroundAgentRegistry, BgAgentKind, BgAgentStatus, CreateAgentRequest,
};
use refact_chat_api::{QuestionKind, QuestionState};
use serde::Serialize;

#[derive(Serialize)]
struct V1AgentQuestion {
    id: String,
    text: String,
    asked_at: DateTime<Utc>,
    answer: Option<String>,
    answered_at: Option<DateTime<Utc>>,
}

#[tokio::test]
async fn v1_fixture_from_historical_question_struct_initializes_registry() {
    let temp = tempfile::tempdir().unwrap();
    let registry = BackgroundAgentRegistry::new(temp.path().to_path_buf())
        .await
        .unwrap();
    let request = CreateAgentRequest {
        parent_chat_id: "parent".into(),
        parent_root_chat_id: None,
        parent_tool_call_id: None,
        kind: BgAgentKind::Subagent,
        config_name: "test".into(),
        title: "Legacy agent".into(),
        prompt: "Continue".into(),
        target_files: Vec::new(),
        model: "test-model".into(),
        model_type: None,
        goal_summary: None,
        plan_present: false,
        worktree_id: None,
        worktree_branch: None,
    };
    let (record, _, _) = registry.create(request).await.unwrap();
    drop(registry);
    let now = Utc::now();
    let questions = vec![
        V1AgentQuestion {
            id: "pending".into(),
            text: "Continue?".into(),
            asked_at: now,
            answer: None,
            answered_at: None,
        },
        V1AgentQuestion {
            id: "answered".into(),
            text: "Which tests?".into(),
            asked_at: now,
            answer: Some("Unit tests".into()),
            answered_at: Some(now),
        },
    ];
    let mut legacy = serde_json::to_value(&record).unwrap();
    legacy["schema_version"] = serde_json::json!(1);
    legacy["questions"] = serde_json::to_value(questions).unwrap();
    let bytes = serde_json::to_vec(&vec![legacy]).unwrap();
    let decode_error = serde_json::from_slice::<Vec<BackgroundAgent>>(&bytes).unwrap_err();
    assert!(decode_error.to_string().contains("missing field `kind`"));
    tokio::fs::write(temp.path().join("records.json"), bytes)
        .await
        .unwrap();

    let restored = BackgroundAgentRegistry::new(temp.path().to_path_buf())
        .await
        .unwrap_or_else(|error| panic!("v1 registry initialization returned Err: {error}"));
    let migrated = restored.get_any(&record.agent_id).await.unwrap();
    assert_eq!(migrated.schema_version, 2);
    assert_eq!(migrated.status, BgAgentStatus::Interrupted);
    assert!(!restored.has_runtime(&record.agent_id).await);
    for question in &migrated.questions {
        assert_eq!(question.kind, QuestionKind::AgentParent);
        assert_eq!(question.asker, record.agent_id);
        assert_eq!(question.answerer, "parent");
        assert_eq!(
            question.correlation_id,
            format!("{}:{}", record.agent_id, question.id)
        );
        assert_eq!(question.asked_at, now);
    }
    assert_eq!(migrated.questions[0].state, QuestionState::Withdrawn);
    assert_eq!(migrated.questions[1].state, QuestionState::Answered);
    assert_eq!(migrated.questions[1].answered(), Some("Unit tests"));
    assert_eq!(migrated.questions[1].answered_at, Some(now));
}
