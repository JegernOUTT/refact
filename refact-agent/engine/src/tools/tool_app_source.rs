#[cfg(test)]
mod tests {
    use std::{collections::HashMap, sync::Arc};

    use serde_json::json;
    use tokio::sync::Mutex;

    use crate::at_commands::at_commands::AtCommandsContext;
    use crate::tools::tool_app::{Family, ToolApp};
    use crate::tools::tools_description::{MatchConfirmDenyResult, Tool};

    #[tokio::test]
    async fn source_grant_requires_tool_confirmation() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        let context = Arc::new(Mutex::new(
            AtCommandsContext::new_from_app(
                app.clone(),
                4096,
                20,
                false,
                vec![],
                "app-grant-test".into(),
                None,
                "test/model".into(),
                None,
                None,
            )
            .await,
        ));
        let app_id = uuid::Uuid::new_v4();
        let args: HashMap<String, serde_json::Value> = serde_json::from_value(json!({
            "action":"control", "app_id":app_id, "source_id":"training",
            "operation":"grant", "expected_revision":"immutable-approved-revision"
        }))
        .unwrap();
        let tool = ToolApp {
            config_path: String::new(),
            family: Family::Source,
        };
        let decision = tool
            .match_against_confirm_deny(context, &args)
            .await
            .unwrap();
        assert!(matches!(
            decision.result,
            MatchConfirmDenyResult::CONFIRMATION
        ));
    }
}
