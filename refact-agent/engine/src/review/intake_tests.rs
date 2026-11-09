use super::*;

fn request() -> ContextIntakeRequest {
    ContextIntakeRequest {
        user_message_id: Some("human".into()),
        question_tool_call_id: None,
        retrieval_tool_call_ids: vec![],
        gaps: CATEGORIES
            .iter()
            .map(|c| ContextIntakeGap {
                category: (*c).into(),
                status: "unknown".into(),
                reason: "Not checked".into(),
            })
            .collect(),
        summary: "No sources asserted".into(),
    }
}

fn user() -> ChatMessage {
    let mut m = ChatMessage::new("user".into(), "Please proceed".into());
    m.message_id = "human".into();
    m
}

#[test]
fn answered_intakes_require_phase_pins_and_carry_answer_provenance() {
    let question = expected_question("before_review", "/repo", "base", "head", None);
    let mut req = request();
    req.user_message_id = None;
    req.question_tool_call_id = Some("ask".into());
    let mut invocation = ChatMessage::new("assistant".into(), "Gather context".into());
    invocation.message_id = "ask-message".into();
    invocation.tool_calls = Some(vec![serde_json::from_value(serde_json::json!({
        "id":"ask", "type":"function", "function":{"name":"ask_questions","arguments":"{}"}
    }))
    .unwrap()]);
    let mut answer = user();
    answer
        .extra
        .insert("client_message_id".into(), serde_json::json!("resolution"));
    let record: refact_privacy::FileRecord = serde_json::from_value(serde_json::json!({
        "path":"/repo/intent.md","zone":"guarded","attribution":"declared"
    }))
    .unwrap();
    crate::privacy::records::merge_records(&mut answer, vec![record.clone()]);
    let set: PendingQuestionSet = serde_json::from_value(serde_json::json!({
        "tool_call_id":"ask","questions":[question],"asked_at_ms":1,"status":"answered",
        "resolution_id":"resolution","response_protocol_version":2,
        "response":{"answers":[{"id":question.id,"disposition":"answered","answer":"Use intent.md"}]}
    })).unwrap();
    let messages = vec![invocation, answer];
    let receipt = validate(
        &req,
        "before_review",
        "owner",
        "/repo",
        "base",
        "head",
        None,
        None,
        &messages,
        &[set.clone()],
        2,
    )
    .unwrap();
    assert_eq!(receipt.input_reference.status, "answered");
    assert_eq!(
        receipt.input_reference.message_id.as_deref(),
        Some("ask-message")
    );
    assert_eq!(receipt.privacy, vec![record]);
    assert!(validate(
        &req,
        "before_review",
        "owner",
        "/repo",
        "base",
        "changed",
        None,
        None,
        &messages,
        &[set],
        2
    )
    .is_err());
}

#[test]
fn tour_requires_landed_matching_fresh_read_not_admission() {
    let mut req = request();
    req.retrieval_tool_call_ids = vec!["read".into()];
    let mut assistant = ChatMessage::new("assistant".into(), "".into());
    assistant.tool_calls = Some(vec![serde_json::from_value(serde_json::json!({"id":"read","type":"function","function":{"name":"review_get","arguments":"{\"path\":\"src/a.rs\"}"}})).unwrap()]);
    let mut result = ChatMessage::new(
        "tool".into(),
        r#"{"candidate_id":"candidate","path":"src/a.rs","lines":[{"line":1,"text":"x"}]}"#.into(),
    );
    result.message_id = "result".into();
    result.tool_call_id = "read".into();
    result.tool_failed = Some(false);
    let messages = vec![user(), assistant, result];
    let check = |messages: &[ChatMessage], start| {
        validate(
            &req,
            "before_tour",
            "owner",
            "/repo",
            "base",
            "head",
            Some("candidate"),
            Some(start),
            messages,
            &[],
            1,
        )
    };
    let receipt = check(&messages, 0).unwrap();
    assert_eq!(receipt.consultations[0].message_id, "result");
    assert!(check(&messages, 2).is_err());
    let mut admission = messages.clone();
    admission[2]
        .extra
        .insert("tool_job".into(), serde_json::json!({}));
    assert!(check(&admission, 0).is_err());
    let mut failed = messages.clone();
    failed[2].tool_failed = Some(true);
    assert!(check(&failed, 0).is_err());
    let mut forged = messages.clone();
    forged.remove(1);
    assert!(check(&forged, 0).is_err());
    let mut stale = messages.clone();
    stale[2].content = crate::call_validation::ChatContent::SimpleText(
        r#"{"candidate_id":"old","path":"src/a.rs","lines":[]}"#.into(),
    );
    assert!(check(&stale, 0).is_err());
}

#[test]
fn user_reference_is_attributed_not_consent_and_gaps_are_required() {
    let messages = vec![user()];
    let mut req = request();
    let receipt = validate(
        &req,
        "before_review",
        "owner",
        "/repo",
        "base",
        "head",
        None,
        None,
        &messages,
        &[],
        1,
    )
    .unwrap();
    assert_eq!(receipt.input_reference.status, "cited_user_intent");
    assert!(receipt.consultations.is_empty());
    req.gaps.pop();
    assert!(validate(
        &req,
        "before_review",
        "owner",
        "/repo",
        "base",
        "head",
        None,
        None,
        &messages,
        &[],
        1
    )
    .is_err());
}

#[test]
fn review_delivery_cannot_be_user_directive_and_tour_requires_fresh_source() {
    let mut m = user();
    m.extra.insert("review".into(), serde_json::json!({}));
    assert!(validate(
        &request(),
        "before_review",
        "owner",
        "/repo",
        "base",
        "head",
        None,
        None,
        &[m],
        &[],
        1
    )
    .is_err());
    assert!(validate(
        &request(),
        "before_tour",
        "owner",
        "/repo",
        "base",
        "head",
        Some("candidate"),
        Some(0),
        &[user()],
        &[],
        1
    )
    .is_err());
}

#[test]
fn context_sources_use_exact_admission_resolver_and_bounds() {
    let mut messages = vec![user()];
    for (id, name) in [
        ("ok", "cat"),
        ("failed", "cat"),
        ("admission", "cat"),
        ("unsupported", "shell"),
        ("event", "knowledge"),
        ("mismatch", "cat"),
        ("orphan", "cat"),
    ] {
        let mut call = ChatMessage::new("assistant".into(), "".into());
        call.tool_calls = Some(vec![serde_json::from_value(serde_json::json!({"id":id,"type":"function","function":{"name":name,"arguments":"{}"}})).unwrap()]);
        if id != "orphan" {
            messages.push(call);
        }
        let mut result = ChatMessage::new("tool".into(), "PRIVATE_RESULT_BYTES".into());
        result.message_id = format!("result:{id}");
        result.tool_call_id = id.into();
        result.tool_failed = Some(id == "failed");
        if id == "admission" {
            result
                .extra
                .insert("tool_job".into(), serde_json::json!({}));
        }
        if ["event", "mismatch"].contains(&id) {
            result.role = "event".into();
            result.tool_call_id.clear();
            result.extra.insert(
                "tool_job_result".into(),
                serde_json::json!({
                    "actor":"tool_completion","original_role":"tool","tool_call_id":id,
                    "tool_name":if id == "mismatch" { "glob" } else { name }
                }),
            );
        }
        messages.push(result);
    }
    let (refs, _, truncated) = context_sources(&messages, 0).unwrap();
    assert!(!truncated);
    assert_eq!(
        refs.iter()
            .map(|r| r.tool_call_id.as_str())
            .collect::<Vec<_>>(),
        vec!["event", "ok"]
    );
    assert!(!serde_json::to_string(&refs)
        .unwrap()
        .contains("PRIVATE_RESULT_BYTES"));
    let mut req = request();
    req.retrieval_tool_call_ids = refs.iter().map(|r| r.tool_call_id.clone()).collect();
    let admitted = validate(
        &req,
        "before_review",
        "owner",
        "/repo",
        "base",
        "head",
        None,
        None,
        &messages,
        &[],
        1,
    )
    .unwrap();
    assert_eq!(admitted.consultations, refs);
    for id in ["failed", "admission", "unsupported", "mismatch", "orphan"] {
        req.retrieval_tool_call_ids = vec![id.into()];
        assert!(
            validate(
                &req,
                "before_review",
                "owner",
                "/repo",
                "base",
                "head",
                None,
                None,
                &messages,
                &[],
                1
            )
            .is_err(),
            "{id}"
        );
    }
    let start = messages.len() - 1;
    for i in 0..65 {
        let mut call = messages[1].clone();
        call.tool_calls.as_mut().unwrap()[0].id = format!("bounded-{i}");
        let mut result = messages[2].clone();
        result.tool_call_id = format!("bounded-{i}");
        result.message_id = format!("result:bounded-{i}");
        messages.extend([call, result]);
    }
    let (refs, _, truncated) = context_sources(&messages, start).unwrap();
    assert!(truncated);
    assert_eq!(refs.len(), 64);
}
