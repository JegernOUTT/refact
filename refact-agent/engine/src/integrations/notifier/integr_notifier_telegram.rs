use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::global_context::GlobalContext;
use crate::integrations::integr_abstract::{IntegrationCommon, IntegrationTrait};
use crate::integrations::notifier::NotifierBackend;

pub const INTEGRATION_ID: &str = "notifier_telegram";
const TELEGRAM_API_BASE: &str = "https://api.telegram.org";
const TELEGRAM_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Deserialize, Serialize, Clone, Default)]
pub struct TelegramNotifierConfig {
    #[serde(default)]
    pub bot_token: String,
    #[serde(default)]
    pub default_chat_id: String,
    #[cfg(test)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_base_for_test: Option<String>,
}

#[derive(Default)]
pub struct IntegrationNotifierTelegram {
    pub common: IntegrationCommon,
    pub cfg: TelegramNotifierConfig,
    pub config_path: String,
}

#[async_trait]
impl IntegrationTrait for IntegrationNotifierTelegram {
    async fn integr_settings_apply(
        &mut self,
        _gcx: Arc<GlobalContext>,
        config_path: String,
        value: &serde_json::Value,
    ) -> Result<(), serde_json::Error> {
        self.cfg = serde_json::from_value(value.clone())?;
        self.common = serde_json::from_value(value.clone())?;
        self.config_path = config_path;
        Ok(())
    }

    fn integr_settings_as_json(&self) -> serde_json::Value {
        serde_json::to_value(&self.cfg).unwrap()
    }

    fn integr_common(&self) -> IntegrationCommon {
        self.common.clone()
    }

    async fn integr_tools(
        &self,
        _integr_name: &str,
    ) -> Vec<Box<dyn crate::tools::tools_description::Tool + Send>> {
        vec![]
    }

    fn integr_schema(&self) -> &str {
        include_str!("notifier_telegram_schema.yaml")
    }
}

pub async fn backend_from_config(
    gcx: Arc<GlobalContext>,
    config_path: String,
    value: &serde_json::Value,
) -> Result<Box<dyn NotifierBackend>, String> {
    let mut integration = IntegrationNotifierTelegram::default();
    integration
        .integr_settings_apply(gcx.clone(), config_path, value)
        .await
        .map_err(|error| format!("failed to apply Telegram notifier settings: {error}"))?;
    let notifier = TelegramNotifier::new(integration.cfg, gcx.http_client.clone());
    #[cfg(test)]
    {
        let mut notifier = notifier;
        if let Some(api_base) = notifier.cfg.api_base_for_test.clone() {
            notifier.api_base = api_base;
        }
        return Ok(Box::new(notifier));
    }
    #[cfg(not(test))]
    Ok(Box::new(notifier))
}

pub struct TelegramNotifier {
    cfg: TelegramNotifierConfig,
    client: reqwest::Client,
    api_base: String,
}

impl TelegramNotifier {
    pub fn new(cfg: TelegramNotifierConfig, client: reqwest::Client) -> Self {
        Self::with_api_base(cfg, client, TELEGRAM_API_BASE.to_string())
    }

    pub fn with_api_base(
        cfg: TelegramNotifierConfig,
        client: reqwest::Client,
        api_base: String,
    ) -> Self {
        Self {
            cfg,
            client,
            api_base,
        }
    }

    fn endpoint(&self, token: &str) -> String {
        format!(
            "{}/bot{token}/sendMessage",
            self.api_base.trim_end_matches('/'),
        )
    }
}

#[async_trait]
impl NotifierBackend for TelegramNotifier {
    async fn send(&self, target: Option<&str>, text: &str) -> Result<(), String> {
        let token = self.cfg.bot_token.trim();
        if token.is_empty() {
            return Err("Telegram bot token is required".to_string());
        }
        let chat_id = target
            .map(str::trim)
            .filter(|target| !target.is_empty())
            .unwrap_or_else(|| self.cfg.default_chat_id.trim());
        if chat_id.is_empty() {
            return Err("Telegram chat id is required".to_string());
        }

        let request = self
            .client
            .post(self.endpoint(token))
            .json(&json!({"chat_id": chat_id, "text": text}))
            .timeout(TELEGRAM_TIMEOUT);
        let response = tokio::time::timeout(TELEGRAM_TIMEOUT, request.send())
            .await
            .map_err(|_| "Telegram notifier request timed out".to_string())?
            .map_err(|error| redact_token(&format!("Telegram notifier failed: {error}"), token))?;
        if response.status().is_success() {
            return Ok(());
        }
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        Err(redact_token(
            &format!(
                "Telegram notifier returned {status}: {}",
                body.chars().take(500).collect::<String>()
            ),
            token,
        ))
    }
}

fn redact_token(text: &str, token: &str) -> String {
    if token.is_empty() {
        return text.to_string();
    }
    text.replace(token, "[REDACTED]")
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::routing::post;
    use axum::{Json, Router};
    use serde_json::Value;
    use tokio::sync::Mutex as AMutex;

    use super::*;

    #[test]
    fn telegram_schema_validates_and_registration_resolves() {
        let integration = crate::integrations::integration_from_name(INTEGRATION_ID).unwrap();
        let schema: serde_yaml::Value = serde_yaml::from_str(integration.integr_schema()).unwrap();
        let schema_json = serde_json::to_value(schema).unwrap();
        let schema_struct: crate::integrations::yaml_schema::ISchema =
            serde_json::from_value(schema_json.clone()).unwrap();

        assert!(schema_struct.fields.contains_key("bot_token"));
        assert!(schema_struct.fields.contains_key("default_chat_id"));
        assert_eq!(
            schema_json["fields"]["bot_token"]["f_extra"]["password"],
            json!(true)
        );
        assert!(crate::integrations::integrations_list(true).contains(&INTEGRATION_ID));
    }

    #[tokio::test]
    async fn telegram_send_posts_expected_request() {
        let received = Arc::new(AMutex::new(Vec::<Value>::new()));
        let handler_received = received.clone();
        let router = Router::new().route(
            "/botsecret-token/sendMessage",
            post(move |Json(body): Json<Value>| {
                let handler_received = handler_received.clone();
                async move {
                    handler_received.lock().await.push(body);
                    axum::http::StatusCode::OK
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = axum::Server::from_tcp(listener.into_std().unwrap())
            .unwrap()
            .serve(router.into_make_service());
        let server_task = tokio::spawn(server);
        let notifier = TelegramNotifier::with_api_base(
            TelegramNotifierConfig {
                bot_token: "secret-token".to_string(),
                default_chat_id: "default-chat".to_string(),
                ..TelegramNotifierConfig::default()
            },
            reqwest::Client::new(),
            format!("http://127.0.0.1:{port}"),
        );

        notifier
            .send(Some("target-chat"), "hello frogs")
            .await
            .unwrap();

        let received = received.lock().await;
        assert_eq!(received.len(), 1);
        assert_eq!(received[0]["chat_id"], json!("target-chat"));
        assert_eq!(received[0]["text"], json!("hello frogs"));
        server_task.abort();
    }

    #[tokio::test]
    async fn telegram_send_uses_default_target_and_redacts_token_on_failure() {
        let router = Router::new().route(
            "/botsecret-token/sendMessage",
            post(|| async { (axum::http::StatusCode::BAD_REQUEST, "bad secret-token") }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = axum::Server::from_tcp(listener.into_std().unwrap())
            .unwrap()
            .serve(router.into_make_service());
        let server_task = tokio::spawn(server);
        let notifier = TelegramNotifier::with_api_base(
            TelegramNotifierConfig {
                bot_token: "secret-token".to_string(),
                default_chat_id: "default-chat".to_string(),
                ..TelegramNotifierConfig::default()
            },
            reqwest::Client::new(),
            format!("http://127.0.0.1:{port}"),
        );

        let err = notifier.send(None, "hello frogs").await.unwrap_err();

        assert!(!err.contains("secret-token"));
        assert!(err.contains("[REDACTED]"));
        server_task.abort();
    }
}
