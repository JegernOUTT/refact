pub use refact_buddy_core::user_action::UserAction;
pub use refact_buddy_core::user_activity::{time_of_day_pattern, UserActivityRing};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_state::AppState;
    use chrono::{DateTime, Local, TimeZone, Utc};
    use hyper::{Body, Request, StatusCode};
    use tower::ServiceExt;

    #[tokio::test]
    async fn post_user_action_endpoint_writes_to_ring() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app_state = crate::app_state::AppState::from_gcx(gcx.clone()).await;
        let activity_path = app_state
            .paths
            .cache_dir
            .join(".refact/buddy/user_activity.jsonl");
        let app = crate::http::routers::v1::make_v1_router(app_state.clone()).with_state(app_state);
        let body = serde_json::to_vec(&UserAction::ChatStarted {
            chat_id: "chat-1".to_string(),
            first_user_text_preview: "hello".to_string(),
            ts: local_ts(10),
        })
        .unwrap();

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/buddy/user_action")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let ring_arc = AppState::from_gcx(gcx).await.buddy.user_activity.clone();
        let ring = ring_arc.lock().await;
        assert_eq!(ring.snapshot().len(), 1);
        drop(ring);
        assert!(tokio::fs::read_to_string(&activity_path)
            .await
            .unwrap()
            .contains("chat_started"));

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/buddy/user_activity?hours=100000")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = hyper::body::to_bytes(response.into_body()).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["actions"].as_array().unwrap().len(), 1);
        assert!(value["time_of_day_pattern"]
            .as_str()
            .unwrap()
            .starts_with("mostly active "));
    }

    fn local_ts(hour: u32) -> DateTime<Utc> {
        Local
            .with_ymd_and_hms(2024, 1, 2, hour, 0, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc)
    }
}
