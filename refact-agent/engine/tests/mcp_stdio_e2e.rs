//! End-to-end MCP stdio tests: drive the real rmcp client stack against the
//! in-repo `mcp_echo_server` binary (`src/bin/mcp_echo_server.rs`).
//!
//! Covers the full protocol path our integrations use — child process spawn,
//! initialize handshake, tool listing/calls, prompts, resources — plus
//! recovery behavior when the server process dies.
use rmcp::model::{CallToolRequestParams, ClientInfo};
use rmcp::transport::TokioChildProcess;
use rmcp::{serve_client, ClientHandler};

#[derive(Clone)]
struct TestClientHandler;

impl ClientHandler for TestClientHandler {
    fn get_info(&self) -> ClientInfo {
        ClientInfo::default()
    }
}

fn echo_server_command() -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_mcp_echo_server"));
    cmd.stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    cmd
}

#[tokio::test]
async fn test_stdio_e2e_initialize_list_and_call() {
    let transport =
        TokioChildProcess::new(echo_server_command()).expect("failed to spawn echo server");
    let client = serve_client(TestClientHandler, transport)
        .await
        .expect("initialize handshake must succeed");

    let server_info = client.peer_info().expect("server info must be present");
    assert_eq!(server_info.server_info.name, "refact-mcp-echo-server");

    let tools = client
        .list_all_tools()
        .await
        .expect("tools/list must succeed");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name.as_ref(), "echo");

    let mut call_params = CallToolRequestParams::new("echo");
    call_params.arguments = serde_json::json!({ "text": "hello mcp" })
        .as_object()
        .cloned();
    let result = client
        .call_tool(call_params)
        .await
        .expect("tools/call must succeed");
    let rendered = serde_json::to_string(&result.content).unwrap_or_default();
    assert!(
        rendered.contains("echo: hello mcp"),
        "unexpected call result: {}",
        rendered
    );

    let prompts = client
        .list_all_prompts()
        .await
        .expect("prompts/list must succeed");
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts[0].name, "greet");

    let resources = client
        .list_all_resources()
        .await
        .expect("resources/list must succeed");
    assert_eq!(resources.len(), 1);
    assert_eq!(resources[0].raw.uri, "echo://readme");

    let _ = client.cancel().await;
}

#[tokio::test]
async fn test_stdio_e2e_fresh_connection_after_shutdown() {
    // First connection: verify it works, then shut the service down cleanly
    // (which terminates the child process).
    let transport =
        TokioChildProcess::new(echo_server_command()).expect("failed to spawn echo server");
    let client = serve_client(TestClientHandler, transport)
        .await
        .expect("first connection must succeed");
    let tools = client
        .list_all_tools()
        .await
        .expect("first tools/list must succeed");
    assert_eq!(tools.len(), 1);

    // Shut down the first service; its child process goes with it.
    let _ = client.cancel().await;

    // A fresh connection must come up cleanly afterwards — this is the
    // transport-level guarantee the reconnect_with_backoff loop builds on.
    let transport =
        TokioChildProcess::new(echo_server_command()).expect("failed to respawn echo server");
    let client = serve_client(TestClientHandler, transport)
        .await
        .expect("reconnection must succeed");
    let tools = client
        .list_all_tools()
        .await
        .expect("tools/list after reconnect must succeed");
    assert_eq!(tools.len(), 1);
    let _ = client.cancel().await;
}
