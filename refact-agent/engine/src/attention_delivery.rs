use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::app_state::AppState;
use crate::attention::{AttentionLimits, PromptDelivery};
use crate::http::routers::v1::sidebar::{NotificationEvent, NotificationQuestion};

const MAX_TICK: Duration = Duration::from_millis(500);
const MIN_TICK: Duration = Duration::from_millis(50);
const SHUTDOWN_POLL: Duration = Duration::from_millis(200);

pub async fn deliver_due(app: &AppState, now: DateTime<Utc>) -> usize {
    let due = app.attention().take_due(now);
    let mut grouped: Vec<((String, String), (u64, Vec<NotificationQuestion>))> = Vec::new();
    let mut positions = HashMap::new();

    for prompt in due {
        for delivery in prompt.deliveries {
            let key = (delivery.chat_id, delivery.tool_call_id);
            let group_index = *positions.entry(key.clone()).or_insert_with(|| {
                grouped.push((
                    (key.0.clone(), key.1.clone()),
                    (delivery.asked_at_ms, Vec::new()),
                ));
                grouped.len() - 1
            });
            let (_, questions) = &mut grouped[group_index].1;
            questions.push(NotificationQuestion {
                id: delivery.question.id,
                question_type: delivery.question.question_type,
                text: delivery.question.text,
                options: delivery.question.options,
            });
        }
    }

    let Some(tx) = app.gcx.notification_events_tx.as_ref() else {
        return 0;
    };
    let mut sent = 0;
    for ((chat_id, tool_call_id), (asked_at_ms, questions)) in grouped {
        if questions.is_empty() {
            continue;
        }
        if tx
            .send(NotificationEvent::AskQuestions {
                chat_id,
                tool_call_id,
                questions,
                asked_at_ms: Some(asked_at_ms),
            })
            .is_ok()
        {
            sent += 1;
        }
    }
    sent
}

async fn wait_for_shutdown(shutdown: Arc<AtomicBool>) {
    while !shutdown.load(Ordering::SeqCst) {
        tokio::time::sleep(SHUTDOWN_POLL).await;
    }
}

pub async fn attention_delivery_background_task(app: AppState, shutdown: Arc<AtomicBool>) {
    loop {
        if shutdown.load(Ordering::SeqCst) {
            return;
        }
        let limits = AttentionLimits::from_settings();
        let tick_ms = (limits.batch_window_ms / 2)
            .min(MAX_TICK.as_millis() as u64)
            .max(MIN_TICK.as_millis() as u64);
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(tick_ms)) => {}
            _ = wait_for_shutdown(shutdown.clone()) => return,
        }
        if shutdown.load(Ordering::SeqCst) {
            return;
        }
        deliver_due(&app, Utc::now()).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration as ChronoDuration;
    use std::collections::HashMap;
    use refact_chat_api::QuestionKind;

    use crate::attention::{AttentionPriority, AttentionRequest, AttentionScope};
    use crate::tools::tools_description::Tool;

    fn request(text: &str, delivery: Option<PromptDelivery>) -> AttentionRequest {
        AttentionRequest::new(
            AttentionScope::new("/project", "AllowToSendAnywhere"),
            QuestionKind::UserInput,
            "agent",
            "user",
            text,
        )
        .with_delivery(delivery)
    }

    fn delivery(
        chat_id: &str,
        tool_call_id: &str,
        id: &str,
        text: &str,
        asked_at_ms: u64,
    ) -> PromptDelivery {
        PromptDelivery {
            chat_id: chat_id.to_string(),
            tool_call_id: tool_call_id.to_string(),
            asked_at_ms,
            question: crate::attention::PromptDeliveryQuestion {
                id: id.to_string(),
                question_type: "free_text".to_string(),
                text: text.to_string(),
                options: None,
            },
        }
    }

    async fn app_with_receiver() -> (
        AppState,
        tokio::sync::broadcast::Receiver<NotificationEvent>,
    ) {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let rx = gcx.notification_events_tx.as_ref().unwrap().subscribe();
        (AppState::from_gcx(gcx).await, rx)
    }

    async fn ask_through_tool(app: &AppState, chat_id: &str, tool_call_id: &str, question: &str) {
        let context = crate::at_commands::at_commands::AtCommandsContext::new_from_app(
            app.clone(),
            4096,
            20,
            false,
            vec![],
            chat_id.to_string(),
            None,
            "test-model".to_string(),
            None,
            None,
        )
        .await;
        let mut tool = crate::tools::tool_ask_questions::ToolAskQuestions {
            config_path: String::new(),
        };
        let args = HashMap::from([(
            "questions".to_string(),
            serde_json::json!([{"id":"q1","type":"free_text","text":question}]),
        )]);
        tool.tool_execute(
            Arc::new(tokio::sync::Mutex::new(context)),
            &tool_call_id.to_string(),
            &args,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn normal_questions_wait_for_the_window_then_go_out_as_one_ranked_batch() {
        let (app, mut rx) = app_with_receiver().await;
        let t0 = Utc::now();
        let id1 = delivery(
            "chat-a",
            "tool-a",
            "q1",
            "One?",
            t0.timestamp_millis() as u64,
        );
        let id2 = delivery(
            "chat-a",
            "tool-a",
            "q2",
            "Two?",
            t0.timestamp_millis() as u64,
        );
        let id3 = delivery(
            "chat-b",
            "tool-b",
            "q3",
            "Three?",
            t0.timestamp_millis() as u64,
        );
        let budget = app.attention();
        budget
            .join(request("One?", Some(id1)), "chat-a:tool-a", t0)
            .unwrap();
        budget
            .join(request("One?", None), "waiter-one", t0)
            .unwrap();
        budget
            .join(request("Two?", Some(id2)), "chat-a:tool-a", t0)
            .unwrap();
        for waiter in ["waiter-two", "waiter-three", "waiter-four"] {
            budget.join(request("Two?", None), waiter, t0).unwrap();
        }
        budget
            .join(request("Three?", Some(id3)), "chat-b:tool-b", t0)
            .unwrap();

        assert_eq!(deliver_due(&app, t0).await, 0);
        assert!(rx.try_recv().is_err());
        assert_eq!(
            deliver_due(
                &app,
                t0 + ChronoDuration::milliseconds(budget.limits().batch_window_ms as i64)
            )
            .await,
            2
        );

        let events = [rx.try_recv().unwrap(), rx.try_recv().unwrap()];
        assert!(
            matches!(&events[0], NotificationEvent::AskQuestions { chat_id, tool_call_id, questions, .. } if chat_id == "chat-a" && tool_call_id == "tool-a" && questions.iter().map(|question| question.id.as_str()).collect::<Vec<_>>() == vec!["q2", "q1"])
        );
        assert!(
            matches!(&events[1], NotificationEvent::AskQuestions { chat_id, tool_call_id, questions, .. } if chat_id == "chat-b" && tool_call_id == "tool-b" && questions[0].id == "q3")
        );
    }

    #[tokio::test]
    async fn a_blocking_question_goes_out_on_the_next_tick() {
        let (app, mut rx) = app_with_receiver().await;
        let now = Utc::now();
        app.attention()
            .join(
                request(
                    "Now?",
                    Some(delivery(
                        "chat",
                        "tool",
                        "q",
                        "Now?",
                        now.timestamp_millis() as u64,
                    )),
                )
                .with_priority(AttentionPriority::Blocking),
                "chat:tool",
                now,
            )
            .unwrap();
        assert_eq!(deliver_due(&app, now).await, 1);
        assert!(matches!(
            rx.try_recv().unwrap(),
            NotificationEvent::AskQuestions { .. }
        ));
    }

    #[tokio::test]
    async fn repeated_ticks_never_resend_a_delivered_prompt() {
        let (app, mut rx) = app_with_receiver().await;
        let now = Utc::now();
        app.attention()
            .join(
                request(
                    "Once?",
                    Some(delivery(
                        "chat",
                        "tool",
                        "q",
                        "Once?",
                        now.timestamp_millis() as u64,
                    )),
                ),
                "chat:tool",
                now,
            )
            .unwrap();
        let due =
            now + ChronoDuration::milliseconds(app.attention().limits().batch_window_ms as i64);
        assert_eq!(deliver_due(&app, due).await, 1);
        assert_eq!(deliver_due(&app, due + ChronoDuration::seconds(1)).await, 0);
        assert!(matches!(
            rx.try_recv().unwrap(),
            NotificationEvent::AskQuestions { .. }
        ));
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn ask_questions_tool_waits_until_the_attention_batch_is_due() {
        let (app, mut rx) = app_with_receiver().await;
        let workspace = tempfile::tempdir().unwrap();
        *app.gcx.documents_state.workspace_folders.lock().unwrap() =
            vec![workspace.path().to_path_buf()];
        for chat_id in ["chat-a", "chat-b"] {
            app.chat.sessions.write().await.insert(
                chat_id.to_string(),
                Arc::new(tokio::sync::Mutex::new(
                    crate::chat::types::ChatSession::new(chat_id.to_string()),
                )),
            );
        }
        let called_at = Utc::now();
        for (chat_id, tool_call_id, text) in [
            ("chat-a", "tool-a1", "A?"),
            ("chat-a", "tool-a2", "A?"),
            ("chat-b", "tool-b1", "A?"),
            ("chat-a", "tool-a3", "B?"),
            ("chat-b", "tool-b2", "B?"),
        ] {
            ask_through_tool(&app, chat_id, tool_call_id, text).await;
        }

        assert!(
            rx.try_recv().is_err(),
            "ask_questions must not ping before batching expires"
        );
        let due_at = called_at
            + ChronoDuration::milliseconds(app.attention().limits().batch_window_ms as i64 + 2_000);
        assert_eq!(deliver_due(&app, due_at).await, 5);
        let events = (0..5).map(|_| rx.try_recv().unwrap()).collect::<Vec<_>>();
        let chat_tool_pairs = events
            .iter()
            .map(|event| match event {
                NotificationEvent::AskQuestions {
                    chat_id,
                    tool_call_id,
                    ..
                } => (chat_id.as_str(), tool_call_id.as_str()),
                _ => panic!("expected question notification"),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            chat_tool_pairs,
            vec![
                ("chat-a", "tool-a1"),
                ("chat-a", "tool-a2"),
                ("chat-b", "tool-b1"),
                ("chat-a", "tool-a3"),
                ("chat-b", "tool-b2"),
            ]
        );
    }
}
