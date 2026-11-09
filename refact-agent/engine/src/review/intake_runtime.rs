use refact_review_core::{ContextIntakeReceipt, Review};
use crate::app_state::AppState;
use super::intake::ContextIntakeRequest;

pub(crate) async fn validate(
    app: &AppState,
    owner: &str,
    request: &ContextIntakeRequest,
    phase: &str,
    repository: &str,
    base: &str,
    head: &str,
    candidate: Option<&str>,
) -> Result<ContextIntakeReceipt, String> {
    let live = app
        .chat
        .sessions
        .read()
        .await
        .get(owner)
        .cloned()
        .ok_or("Owning chat unavailable")?;
    let session = live.lock().await;
    if session.closed
        || session
            .abort_flag
            .load(std::sync::atomic::Ordering::Acquire)
    {
        return Err("Context intake session is no longer live".into());
    }
    let activation = activation_index(&session.messages, candidate);
    let mut receipt = super::intake::validate(
        request,
        phase,
        owner,
        repository,
        base,
        head,
        candidate,
        activation,
        &session.messages,
        &session.pending_questions,
        super::service::now_ms(),
    )?;
    receipt.id = format!("{}:{}", request_hash(request)?, receipt.id);
    Ok(receipt)
}

fn activation_index(
    messages: &[crate::call_validation::ChatMessage],
    candidate: Option<&str>,
) -> Option<usize> {
    candidate.and_then(|id| {
        messages.iter().rposition(|m| {
            m.role == "tool"
                && m.tool_failed == Some(false)
                && (m
                    .extra
                    .get("review")
                    .is_some_and(|v| v["candidate_id"].as_str() == Some(id))
                    || m.extra
                        .get("review_intake_members")
                        .and_then(|v| v.as_array())
                        .is_some_and(|members| {
                            members
                                .iter()
                                .any(|v| v["candidate_id"].as_str() == Some(id))
                        }))
                && messages
                    .iter()
                    .flat_map(|m| m.tool_calls.iter().flatten())
                    .any(|c| c.id == m.tool_call_id && c.function.name == "set_review_context")
        })
    })
}

pub(crate) async fn context_sources(
    app: &AppState,
    owner: &str,
    phase: &str,
    candidate: Option<&str>,
) -> Result<(serde_json::Value, Vec<refact_privacy::FileRecord>), String> {
    let live = app
        .chat
        .sessions
        .read()
        .await
        .get(owner)
        .cloned()
        .ok_or("Owning chat unavailable")?;
    let session = live.lock().await;
    if session.closed
        || session
            .abort_flag
            .load(std::sync::atomic::Ordering::Acquire)
    {
        return Err("Context intake session is no longer live".into());
    }
    let after = match phase {
        "before_review" => session.messages.iter().rposition(|m| m.role == "user" && !m.extra.contains_key("review")),
        "before_tour" => activation_index(&session.messages, candidate),
        _ => return Err("Invalid context source phase".into()),
    }.ok_or("No current context checkpoint; an ordinary user message or matching candidate activation is required")?;
    let (refs, privacy, truncated) = super::intake::context_sources(&session.messages, after)?;
    Ok((
        serde_json::json!({"phase":phase,"candidate_id":candidate,"sources":refs,"truncated":truncated,
        "scope_notice":"Completed supported retrievals in this owning chat after the latest phase checkpoint, newest first (limit 64). Reread sources if absent. Cite tool_call_id in context_intake; this metadata is not consent, category coverage or an admitted receipt."}),
        privacy,
    ))
}

fn request_hash(request: &ContextIntakeRequest) -> Result<String, String> {
    Ok(refact_review_core::content_hash(
        &serde_json::to_vec(request).map_err(|_| "Invalid context intake")?,
    ))
}
pub(crate) fn verify_request(
    request: &ContextIntakeRequest,
    receipt: &ContextIntakeReceipt,
) -> Result<(), String> {
    if receipt
        .id
        .starts_with(&format!("{}:", request_hash(request)?))
    {
        Ok(())
    } else {
        Err("Context intake request key conflicts with a different request".into())
    }
}

pub(crate) fn privacy(review: &Review) -> Vec<refact_privacy::FileRecord> {
    review
        .context_intake
        .iter()
        .chain(review.tour.iter().flat_map(|s| {
            std::iter::once(&s.current)
                .chain(s.history.iter())
                .filter_map(|v| v.context_intake.as_ref())
        }))
        .flat_map(|r| r.privacy.iter().cloned())
        .collect()
}

pub(crate) fn summary(receipt: &ContextIntakeReceipt) -> serde_json::Value {
    let mut value = serde_json::to_value(receipt).unwrap_or(serde_json::Value::Null);
    if let Some(map) = value.as_object_mut() {
        map.remove("privacy");
        map.insert("privacy_total".into(), receipt.privacy.len().into());
        map.insert(
            "privacy_omitted".into(),
            (!receipt.privacy.is_empty()).into(),
        );
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call_validation::ChatMessage;
    use serde_json::json;

    fn invocation(id: &str, name: &str, arguments: serde_json::Value) -> ChatMessage {
        let mut message = ChatMessage::new("assistant".into(), "".into());
        message.message_id = format!("invocation:{id}");
        message.tool_calls = Some(vec![serde_json::from_value(json!({
            "id":id,"type":"function","function":{
                "name":name,"arguments":arguments.to_string()
            }
        }))
        .unwrap()]);
        message
    }

    fn result(id: &str, payload: serde_json::Value) -> ChatMessage {
        let mut message = ChatMessage::new("tool".into(), payload.to_string().into());
        message.message_id = format!("result:{id}");
        message.tool_call_id = id.into();
        message.tool_failed = Some(false);
        message
    }

    async fn check(
        app: &AppState,
        request: &ContextIntakeRequest,
    ) -> Result<ContextIntakeReceipt, String> {
        validate(
            app,
            "owner",
            request,
            "before_tour",
            "/repo",
            "base",
            "head",
            Some("candidate"),
        )
        .await
    }

    #[tokio::test]
    async fn latest_activation_invalidates_old_reads_and_published_inactive_members_are_fresh() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            temp.path().join("cache"),
            temp.path().join("config"),
        )
        .await;
        let app = AppState::from_gcx(gcx).await;
        let mut session = crate::chat::types::ChatSession::new("owner".into());
        let mut user = ChatMessage::new("user".into(), "Inspect the candidate".into());
        user.message_id = "human".into();
        let mut activation = result("activate", json!({}));
        activation
            .extra
            .insert("review".into(), json!({"candidate_id":"candidate"}));
        let read_payload = json!({"candidate_id":"candidate","path":"src/a.rs","lines":[]});
        let read_args = json!({"candidate_id":"candidate","path":"src/a.rs"});
        session.messages = vec![
            user,
            invocation("activate", "set_review_context", json!({})),
            activation.clone(),
            invocation("read", "review_get", read_args.clone()),
            result("read", read_payload.clone()),
        ];
        let session = std::sync::Arc::new(tokio::sync::Mutex::new(session));
        app.chat
            .sessions
            .write()
            .await
            .insert("owner".into(), session.clone());
        let gaps = [
            "mcp",
            "web",
            "messenger",
            "knowledge",
            "trajectories",
            "tasks",
            "memory",
        ]
        .map(|category| json!({"category":category,"status":"unknown","reason":"Not consulted"}));
        let mut request: ContextIntakeRequest = serde_json::from_value(json!({
            "user_message_id":"human","retrieval_tool_call_ids":["read"],"summary":"Candidate inspected",
            "gaps":gaps
        })).unwrap();
        assert!(check(&app, &request).await.is_ok());
        let checkout = session.lock().await.thread.worktree.clone();
        {
            let mut live = session.lock().await;
            live.messages
                .push(invocation("reactivate", "set_review_context", json!({})));
            activation.tool_call_id = "reactivate".into();
            activation.message_id = "result:reactivate".into();
            live.messages.push(activation);
        }
        assert!(check(&app, &request)
            .await
            .unwrap_err()
            .contains("fresh candidate"));
        {
            let mut live = session.lock().await;
            live.messages
                .push(invocation("fresh-read", "review_get", read_args.clone()));
            live.messages
                .push(result("fresh-read", read_payload.clone()));
        }
        request.retrieval_tool_call_ids = vec!["fresh-read".into()];
        assert!(check(&app, &request).await.is_ok());
        {
            let mut live = session.lock().await;
            live.messages.truncate(1);
            live.messages
                .push(invocation("whole-import", "set_review_context", json!({})));
            let mut published = result("whole-import", json!({}));
            published
                .extra
                .insert("review".into(), json!({"candidate_id":"active-other"}));
            published.extra.insert(
                "review_intake_members".into(),
                json!([
                    {"candidate_id":"active-other"},{"candidate_id":"candidate"}
                ]),
            );
            live.messages.push(published);
            live.messages
                .push(invocation("member-read", "review_get", read_args));
            live.messages.push(result("member-read", read_payload));
        }
        request.retrieval_tool_call_ids = vec!["member-read".into()];
        assert!(check(&app, &request).await.is_ok());
        assert_eq!(session.lock().await.thread.worktree, checkout);
        {
            let mut live = session.lock().await;
            live.messages[1] = invocation("whole-import", "review_get", json!({}));
        }
        assert!(check(&app, &request).await.is_err());
        {
            let mut live = session.lock().await;
            live.messages[1] = invocation("unrelated", "set_review_context", json!({}));
        }
        assert!(check(&app, &request).await.is_err());
        assert_eq!(session.lock().await.thread.worktree, checkout);
    }
}
