#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::json;

    use crate::tools::tool_app::{Family, ToolApp};
    use crate::tools::tools_description::{MatchConfirmDenyResult, Tool};

    #[cfg(unix)]
    #[tokio::test]
    async fn source_grant_does_not_require_tool_confirmation() {
        let (app, context, _dir) = crate::tools::tool_app::tests::approval_fixture().await;
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
        assert!(matches!(decision.result, MatchConfirmDenyResult::PASS));
        app.gcx.apps.shutdown().await;
    }
}
