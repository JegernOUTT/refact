#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use tokio::sync::Mutex as AMutex;

    use crate::at_commands::at_commands::AtCommandsContext;
    use crate::buddy::user_activity::UserAction;
    use crate::integrations::integr_cmdline::{CmdlineToolConfig, ToolCmdline};
    use crate::tools::tools_description::Tool;

    #[tokio::test]
    async fn command_run_pushed_from_tool_cmdline() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        {
            *gcx.documents_state.workspace_folders.lock().unwrap() =
                vec![std::env::temp_dir()];
        }
        let ccx = Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                crate::app_state::AppState::from_gcx(gcx.clone()).await,
                1000,
                1,
                false,
                Vec::new(),
                "chat-cmd".to_string(),
                None,
                "test-model".to_string(),
                None,
                None,
            )
            .await,
        ));
        let mut tool = ToolCmdline {
            name: "cmdline_test".to_string(),
            cfg: CmdlineToolConfig {
                command: "printf hello".to_string(),
                timeout: "5".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };

        tool.tool_execute(ccx, &"tc-cmd".to_string(), &HashMap::new())
            .await
            .unwrap();

        let user_activity = gcx.user_activity.clone();
        let ring = user_activity.lock().await;
        assert!(ring.snapshot().iter().any(|action| matches!(
            action,
            UserAction::CommandRun { command_preview, chat_id, .. }
                if command_preview == "printf hello" && chat_id == "chat-cmd"
        )));
    }
}
