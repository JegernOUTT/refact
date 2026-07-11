use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex as AMutex;
use tokio::sync::oneshot;
use uuid::Uuid;

pub const MCP_INTERACTION_TIMEOUT_SECS: u64 = 300;

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MCPInteractionKind {
    Elicitation {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        requested_schema: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        elicitation_id: Option<String>,
    },
    SamplingApproval {
        message_count: usize,
        max_tokens: u64,
        preview: String,
    },
}

#[derive(Serialize, Clone, Debug)]
pub struct MCPInteractionInfo {
    pub id: String,
    pub config_path: String,
    pub server_name: String,
    #[serde(flatten)]
    pub kind: MCPInteractionKind,
    pub created_at_ms: i64,
    pub timeout_at_ms: i64,
}

#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct MCPInteractionResponse {
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<serde_json::Value>,
}

struct PendingInteraction {
    info: MCPInteractionInfo,
    responder: oneshot::Sender<MCPInteractionResponse>,
}

static PENDING_INTERACTIONS: OnceLock<AMutex<HashMap<String, PendingInteraction>>> =
    OnceLock::new();

fn pending_interactions() -> &'static AMutex<HashMap<String, PendingInteraction>> {
    PENDING_INTERACTIONS.get_or_init(|| AMutex::new(HashMap::new()))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

pub fn server_name_from_config_path(config_path: &str) -> String {
    let stem = std::path::Path::new(config_path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| config_path.to_string());
    for prefix in ["mcp_http_", "mcp_sse_", "mcp_stdio_", "mcp_"] {
        if let Some(rest) = stem.strip_prefix(prefix) {
            if !rest.is_empty() {
                return rest.to_string();
            }
        }
    }
    stem
}

pub async fn create_mcp_interaction(
    config_path: &str,
    kind: MCPInteractionKind,
) -> (String, oneshot::Receiver<MCPInteractionResponse>) {
    let id = Uuid::new_v4().to_string();
    let (tx, rx) = oneshot::channel();
    let created = now_ms();
    let info = MCPInteractionInfo {
        id: id.clone(),
        config_path: config_path.to_string(),
        server_name: server_name_from_config_path(config_path),
        kind,
        created_at_ms: created,
        timeout_at_ms: created + (MCP_INTERACTION_TIMEOUT_SECS as i64) * 1000,
    };
    pending_interactions().lock().await.insert(
        id.clone(),
        PendingInteraction {
            info,
            responder: tx,
        },
    );
    (id, rx)
}

pub async fn respond_mcp_interaction(
    id: &str,
    response: MCPInteractionResponse,
) -> Result<(), String> {
    let entry = pending_interactions()
        .lock()
        .await
        .remove(id)
        .ok_or_else(|| format!("No pending MCP interaction: {}", id))?;
    entry
        .responder
        .send(response)
        .map_err(|_| "Interaction is no longer awaited".to_string())
}

pub async fn list_mcp_interactions() -> Vec<MCPInteractionInfo> {
    let mut infos: Vec<MCPInteractionInfo> = pending_interactions()
        .lock()
        .await
        .values()
        .map(|e| e.info.clone())
        .collect();
    infos.sort_by_key(|i| i.created_at_ms);
    infos
}

pub async fn cancel_mcp_interactions_for_config(config_path: &str) {
    let mut pending = pending_interactions().lock().await;
    pending.retain(|_, e| e.info.config_path != config_path);
}

pub async fn wait_for_mcp_interaction_response(
    id: &str,
    rx: oneshot::Receiver<MCPInteractionResponse>,
    timeout_secs: u64,
) -> MCPInteractionResponse {
    match tokio::time::timeout(Duration::from_secs(timeout_secs), rx).await {
        Ok(Ok(response)) => response,
        Ok(Err(_)) => MCPInteractionResponse {
            action: "decline".to_string(),
            content: None,
        },
        Err(_) => {
            pending_interactions().lock().await.remove(id);
            MCPInteractionResponse {
                action: "decline".to_string(),
                content: None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_name_from_config_path() {
        assert_eq!(
            server_name_from_config_path("/x/integrations.d/mcp_http_github.yaml"),
            "github"
        );
        assert_eq!(
            server_name_from_config_path("/x/integrations.d/mcp_linear.yaml"),
            "linear"
        );
        assert_eq!(server_name_from_config_path("/x/mcp_.yaml"), "mcp_");
        assert_eq!(server_name_from_config_path("plain.yaml"), "plain");
    }

    #[tokio::test]
    async fn test_interaction_respond_roundtrip() {
        let (id, rx) = create_mcp_interaction(
            "/tmp/mcp_test_roundtrip.yaml",
            MCPInteractionKind::Elicitation {
                message: "Enter token".to_string(),
                requested_schema: Some(serde_json::json!({"type": "object"})),
                url: None,
                elicitation_id: None,
            },
        )
        .await;

        let listed = list_mcp_interactions().await;
        assert!(listed.iter().any(|i| i.id == id));

        respond_mcp_interaction(
            &id,
            MCPInteractionResponse {
                action: "accept".to_string(),
                content: Some(serde_json::json!({"token": "abc"})),
            },
        )
        .await
        .unwrap();

        let response = wait_for_mcp_interaction_response(&id, rx, 5).await;
        assert_eq!(response.action, "accept");
        assert_eq!(response.content, Some(serde_json::json!({"token": "abc"})));

        assert!(!list_mcp_interactions().await.iter().any(|i| i.id == id));
        assert!(
            respond_mcp_interaction(&id, MCPInteractionResponse::default())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn test_interaction_timeout_declines_and_cleans_up() {
        let (id, rx) = create_mcp_interaction(
            "/tmp/mcp_test_timeout.yaml",
            MCPInteractionKind::SamplingApproval {
                message_count: 2,
                max_tokens: 100,
                preview: "hi".to_string(),
            },
        )
        .await;
        let response = wait_for_mcp_interaction_response(&id, rx, 0).await;
        assert_eq!(response.action, "decline");
        assert!(!list_mcp_interactions().await.iter().any(|i| i.id == id));
    }

    #[tokio::test]
    async fn test_cancel_for_config_drops_and_declines() {
        let (id, rx) = create_mcp_interaction(
            "/tmp/mcp_test_cancel.yaml",
            MCPInteractionKind::Elicitation {
                message: "q".to_string(),
                requested_schema: None,
                url: None,
                elicitation_id: None,
            },
        )
        .await;
        cancel_mcp_interactions_for_config("/tmp/mcp_test_cancel.yaml").await;
        assert!(!list_mcp_interactions().await.iter().any(|i| i.id == id));
        let response = wait_for_mcp_interaction_response(&id, rx, 5).await;
        assert_eq!(response.action, "decline");
    }

    #[test]
    fn test_interaction_info_serialization_shape() {
        let info = MCPInteractionInfo {
            id: "i1".to_string(),
            config_path: "/x/mcp_http_srv.yaml".to_string(),
            server_name: "srv".to_string(),
            kind: MCPInteractionKind::Elicitation {
                message: "m".to_string(),
                requested_schema: None,
                url: Some("https://example.com".to_string()),
                elicitation_id: Some("e1".to_string()),
            },
            created_at_ms: 1,
            timeout_at_ms: 2,
        };
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(
            json.get("kind").and_then(|v| v.as_str()),
            Some("elicitation")
        );
        assert_eq!(
            json.get("url").and_then(|v| v.as_str()),
            Some("https://example.com")
        );
        assert!(json.get("requested_schema").is_none());

        let sampling = MCPInteractionInfo {
            id: "i2".to_string(),
            config_path: "c".to_string(),
            server_name: "s".to_string(),
            kind: MCPInteractionKind::SamplingApproval {
                message_count: 3,
                max_tokens: 42,
                preview: "p".to_string(),
            },
            created_at_ms: 1,
            timeout_at_ms: 2,
        };
        let json = serde_json::to_value(&sampling).unwrap();
        assert_eq!(
            json.get("kind").and_then(|v| v.as_str()),
            Some("sampling_approval")
        );
        assert_eq!(json.get("max_tokens").and_then(|v| v.as_u64()), Some(42));
    }
}
