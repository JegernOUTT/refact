use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::retry::RetryConfig;

pub const DEFAULT_RECURRING_AUTO_EXPIRE_AFTER_MS: u64 = 30 * 24 * 60 * 60 * 1000;
pub const DEFAULT_SCHEDULER_MAX_JOBS: u32 = 50;
pub const DEFAULT_SCHEDULER_MAX_CONCURRENT_RUNS: usize = 8;
pub const DEFAULT_MISSED_GRACE_MIN_MS: u64 = 120 * 1000;
pub const DEFAULT_MISSED_GRACE_MAX_MS: u64 = 2 * 60 * 60 * 1000;
pub const DURABLE_DISABLED_NOTE: &str = "durable schedules disabled by config";
pub const SCHEDULER_DISABLED_ERROR: &str = "scheduler is disabled";
pub const SCHEDULER_DISABLE_ENV: &str = "REFACT_DISABLE_SCHEDULER";
pub const RECENT_RUNS_CAP: usize = 20;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SchedulerConfig {
    #[serde(default = "default_scheduler_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub disable_durable: bool,
    #[serde(default = "default_scheduler_max_jobs")]
    pub max_jobs: u32,
    #[serde(default = "default_scheduler_max_concurrent_runs")]
    pub max_concurrent_runs: usize,
    #[serde(default = "default_scheduler_recent_runs_cap")]
    pub recent_runs_cap: usize,
    #[serde(default = "default_missed_grace_min_ms")]
    pub missed_grace_min_ms: u64,
    #[serde(default = "default_missed_grace_max_ms")]
    pub missed_grace_max_ms: u64,
    #[serde(default)]
    pub retry: RetryConfig,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            disable_durable: false,
            max_jobs: DEFAULT_SCHEDULER_MAX_JOBS,
            max_concurrent_runs: DEFAULT_SCHEDULER_MAX_CONCURRENT_RUNS,
            recent_runs_cap: RECENT_RUNS_CAP,
            missed_grace_min_ms: DEFAULT_MISSED_GRACE_MIN_MS,
            missed_grace_max_ms: DEFAULT_MISSED_GRACE_MAX_MS,
            retry: RetryConfig::default(),
        }
    }
}

impl SchedulerConfig {
    pub fn with_startup_overrides(mut self, no_scheduler: bool) -> Self {
        if no_scheduler || scheduler_disabled_by_env() {
            self.enabled = false;
        }
        self
    }

    pub fn runner_enabled(&self) -> bool {
        self.enabled
    }
}

#[cfg(test)]
pub fn test_scheduler_config_with<F>(configure: F) -> SchedulerConfig
where
    F: FnOnce(&mut SchedulerConfig),
{
    let mut config = SchedulerConfig::default();
    configure(&mut config);
    config
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CronCreatePolicy {
    pub durable: bool,
    pub note: Option<String>,
}

pub fn cron_create_policy(
    config: &SchedulerConfig,
    requested_durable: bool,
) -> Result<CronCreatePolicy, String> {
    if !config.enabled {
        return Err(SCHEDULER_DISABLED_ERROR.to_string());
    }
    if requested_durable && config.disable_durable {
        return Ok(CronCreatePolicy {
            durable: false,
            note: Some(DURABLE_DISABLED_NOTE.to_string()),
        });
    }
    Ok(CronCreatePolicy {
        durable: requested_durable,
        note: None,
    })
}

pub fn scheduler_disabled_by_env() -> bool {
    std::env::var(SCHEDULER_DISABLE_ENV).ok().as_deref() == Some("1")
}

fn default_scheduler_enabled() -> bool {
    true
}

fn default_scheduler_max_jobs() -> u32 {
    DEFAULT_SCHEDULER_MAX_JOBS
}

fn default_scheduler_max_concurrent_runs() -> usize {
    DEFAULT_SCHEDULER_MAX_CONCURRENT_RUNS
}

fn default_scheduler_recent_runs_cap() -> usize {
    RECENT_RUNS_CAP
}

fn default_missed_grace_min_ms() -> u64 {
    DEFAULT_MISSED_GRACE_MIN_MS
}

fn default_missed_grace_max_ms() -> u64 {
    DEFAULT_MISSED_GRACE_MAX_MS
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(from = "RawJob")]
pub struct Job {
    pub id: String,
    pub description: String,
    #[serde(default = "default_job_enabled")]
    pub enabled: bool,
    pub durable: bool,
    pub created_at_ms: u64,
    // Keep recurring explicit so Cron triggers preserve legacy one-shot behavior.
    pub recurring: bool,
    pub trigger: Trigger,
    pub action: Action,
    pub delivery: Delivery,
    pub last_fired_at_ms: Option<u64>,
    pub fire_count: u32,
    pub last_status: Option<String>,
    pub last_error: Option<String>,
    #[serde(default)]
    pub last_delivery_error: Option<String>,
    pub recent_runs: Vec<CronRunRecord>,
    pub paused_at_ms: Option<u64>,
    pub trigger_at_ms: Option<u64>,
    pub auto_expire_after_ms: u64,
    #[serde(default)]
    pub retry_attempts: u32,
}

impl Job {
    pub fn new_cron_agent_chat(
        cron: String,
        prompt: String,
        description: String,
        recurring: bool,
        durable: bool,
        created_at_ms: u64,
    ) -> Self {
        Self {
            id: format!("cron_{}", Uuid::now_v7()),
            description,
            enabled: true,
            durable,
            created_at_ms,
            recurring,
            trigger: Trigger::Cron {
                expr: cron,
                tz: None,
            },
            action: Action::AgentTurn {
                prompt,
                target: AgentTarget::ExistingChat {
                    chat_id: String::new(),
                },
                mode: None,
                model: None,
                tools: None,
            },
            delivery: Delivery::Chat,
            last_fired_at_ms: None,
            fire_count: 0,
            last_status: None,
            last_error: None,
            last_delivery_error: None,
            recent_runs: Vec::new(),
            paused_at_ms: None,
            trigger_at_ms: None,
            auto_expire_after_ms: default_auto_expire_after_ms(recurring),
            retry_attempts: 0,
        }
    }

    pub fn cron_expr(&self) -> Option<&str> {
        match &self.trigger {
            Trigger::Cron { expr, .. } => Some(expr),
            _ => None,
        }
    }

    pub fn prompt(&self) -> Option<&str> {
        match &self.action {
            Action::AgentTurn { prompt, .. } => Some(prompt),
            _ => None,
        }
    }

    pub fn command_spec(&self) -> Option<CommandSpec> {
        match &self.action {
            Action::Command {
                argv,
                target,
                cwd,
                env,
                timeout_secs,
            } => Some(CommandSpec {
                argv: argv.clone(),
                target: target.clone(),
                cwd: cwd.clone(),
                env: env.clone(),
                timeout_secs: *timeout_secs,
            }),
            _ => None,
        }
    }

    pub fn action_kind(&self) -> &'static str {
        match &self.action {
            Action::AgentTurn { .. } => "agent_turn",
            Action::Command { .. } => "command",
        }
    }

    pub fn chat_id(&self) -> Option<&str> {
        match &self.action {
            Action::AgentTurn {
                target: AgentTarget::ExistingChat { chat_id },
                ..
            } if !chat_id.is_empty() => Some(chat_id),
            Action::Command {
                target: AgentTarget::ExistingChat { chat_id },
                ..
            } if !chat_id.is_empty() => Some(chat_id),
            _ => None,
        }
    }

    pub fn mode(&self) -> Option<&str> {
        match &self.action {
            Action::AgentTurn { mode, .. } => mode.as_deref().filter(|mode| !mode.is_empty()),
            _ => None,
        }
    }

    pub fn is_paused(&self) -> bool {
        !self.enabled || self.paused_at_ms.is_some()
    }

    pub fn set_existing_chat(&mut self, chat_id: Option<String>) {
        match &mut self.action {
            Action::AgentTurn { target, .. } | Action::Command { target, .. } => {
                *target = AgentTarget::ExistingChat {
                    chat_id: chat_id.unwrap_or_default(),
                };
            }
        }
    }

    pub fn set_isolated(&mut self) {
        match &mut self.action {
            Action::AgentTurn { target, .. } | Action::Command { target, .. } => {
                *target = AgentTarget::Isolated;
            }
        }
    }

    pub fn set_mode(&mut self, mode: Option<String>) {
        if let Action::AgentTurn { mode: target, .. } = &mut self.action {
            *target = mode;
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Trigger {
    Cron { expr: String, tz: Option<String> },
    Interval { every_ms: u64 },
    Once { at_ms: u64 },
    Manual,
    Webhook { hook_id: String },
    OnProcessExit { match_kind: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    AgentTurn {
        prompt: String,
        target: AgentTarget,
        mode: Option<String>,
        model: Option<String>,
        tools: Option<Vec<String>>,
    },
    Command {
        argv: Vec<String>,
        #[serde(default = "default_command_target")]
        target: AgentTarget,
        cwd: Option<String>,
        env: Option<BTreeMap<String, String>>,
        timeout_secs: Option<u64>,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CommandSpec {
    pub argv: Vec<String>,
    #[serde(default = "default_command_target")]
    pub target: AgentTarget,
    pub cwd: Option<String>,
    pub env: Option<BTreeMap<String, String>>,
    pub timeout_secs: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentTarget {
    ExistingChat { chat_id: String },
    Isolated,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Delivery {
    Chat,
    Webhook {
        url: String,
        token: Option<String>,
    },
    Notifier {
        integration_id: String,
        target: Option<String>,
    },
    None,
}

pub fn delivery_from_value(value: &Value) -> Result<Delivery, String> {
    match value {
        Value::String(kind) => delivery_from_parts(kind, None, None, None, None),
        Value::Object(map) => {
            let kind = map
                .get("kind")
                .or_else(|| map.get("type"))
                .and_then(Value::as_str)
                .unwrap_or("webhook");
            let url = map.get("url").and_then(Value::as_str);
            let token = map.get("token").and_then(Value::as_str);
            let integration_id = map
                .get("integration_id")
                .or_else(|| map.get("integration"))
                .and_then(Value::as_str);
            let target = map.get("target").and_then(Value::as_str);
            delivery_from_parts(kind, url, token, integration_id, target)
        }
        Value::Null => Ok(Delivery::Chat),
        other => Err(format!("argument `delivery` is invalid: {other:?}")),
    }
}

fn delivery_from_parts(
    kind: &str,
    url: Option<&str>,
    token: Option<&str>,
    integration_id: Option<&str>,
    target: Option<&str>,
) -> Result<Delivery, String> {
    match kind.trim().to_ascii_lowercase().as_str() {
        "chat" => Ok(Delivery::Chat),
        "none" => Ok(Delivery::None),
        "webhook" => {
            let url = url
                .map(str::trim)
                .filter(|url| !url.is_empty())
                .ok_or_else(|| "delivery webhook requires `url`".to_string())?;
            Ok(Delivery::Webhook {
                url: url.to_string(),
                token: token
                    .map(str::trim)
                    .filter(|token| !token.is_empty())
                    .map(str::to_string),
            })
        }
        "notifier" => {
            let integration_id = integration_id
                .map(str::trim)
                .filter(|integration_id| !integration_id.is_empty())
                .ok_or_else(|| "delivery notifier requires `integration_id`".to_string())?;
            Ok(Delivery::Notifier {
                integration_id: integration_id.to_string(),
                target: target
                    .map(str::trim)
                    .filter(|target| !target.is_empty())
                    .map(str::to_string),
            })
        }
        other => Err(format!("unsupported delivery `{other}`")),
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CronRunRecord {
    pub at_ms: u64,
    pub status: String,
    pub error: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct RawJob {
    id: Option<String>,
    description: Option<String>,
    enabled: Option<bool>,
    durable: Option<bool>,
    created_at_ms: Option<u64>,
    recurring: Option<bool>,
    trigger: Option<Trigger>,
    action: Option<Action>,
    delivery: Option<Delivery>,
    last_fired_at_ms: Option<u64>,
    fire_count: Option<u32>,
    last_status: Option<String>,
    last_error: Option<String>,
    last_delivery_error: Option<String>,
    recent_runs: Option<Vec<CronRunRecord>>,
    paused_at_ms: Option<u64>,
    trigger_at_ms: Option<u64>,
    auto_expire_after_ms: Option<u64>,
    retry_attempts: Option<u32>,
    cron: Option<String>,
    prompt: Option<String>,
    chat_id: Option<String>,
    mode: Option<String>,
}

impl From<RawJob> for Job {
    fn from(raw: RawJob) -> Self {
        let trigger = raw.trigger.unwrap_or_else(|| legacy_trigger(raw.cron));
        let recurring = raw
            .recurring
            .unwrap_or_else(|| default_recurring_for_trigger(&trigger));
        let auto_expire_after_ms = raw
            .auto_expire_after_ms
            .unwrap_or_else(|| default_auto_expire_after_ms(recurring));

        Self {
            id: raw.id.unwrap_or_else(new_job_id),
            description: raw.description.unwrap_or_default(),
            enabled: raw.enabled.unwrap_or_else(default_job_enabled),
            durable: raw.durable.unwrap_or_default(),
            created_at_ms: raw.created_at_ms.unwrap_or_default(),
            recurring,
            trigger,
            action: raw
                .action
                .unwrap_or_else(|| legacy_action(raw.prompt, raw.chat_id, raw.mode)),
            delivery: raw.delivery.unwrap_or(Delivery::Chat),
            last_fired_at_ms: raw.last_fired_at_ms,
            fire_count: raw.fire_count.unwrap_or_default(),
            last_status: raw.last_status,
            last_error: raw.last_error,
            last_delivery_error: raw.last_delivery_error,
            recent_runs: raw.recent_runs.unwrap_or_default(),
            paused_at_ms: raw.paused_at_ms,
            trigger_at_ms: raw.trigger_at_ms,
            auto_expire_after_ms,
            retry_attempts: raw.retry_attempts.unwrap_or_default(),
        }
    }
}

fn legacy_trigger(cron: Option<String>) -> Trigger {
    cron.map(|expr| Trigger::Cron { expr, tz: None })
        .unwrap_or(Trigger::Manual)
}

fn legacy_action(prompt: Option<String>, chat_id: Option<String>, mode: Option<String>) -> Action {
    Action::AgentTurn {
        prompt: prompt.unwrap_or_default(),
        target: AgentTarget::ExistingChat {
            chat_id: chat_id.unwrap_or_default(),
        },
        mode,
        model: None,
        tools: None,
    }
}

fn new_job_id() -> String {
    format!("job_{}", Uuid::now_v7())
}

fn default_job_enabled() -> bool {
    true
}

fn default_recurring_for_trigger(trigger: &Trigger) -> bool {
    !matches!(trigger, Trigger::Once { .. })
}

fn default_command_target() -> AgentTarget {
    AgentTarget::ExistingChat {
        chat_id: String::new(),
    }
}

fn default_auto_expire_after_ms(recurring: bool) -> u64 {
    if recurring {
        DEFAULT_RECURRING_AUTO_EXPIRE_AFTER_MS
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn nested_job() -> Job {
        Job {
            id: "job-1".to_string(),
            description: "Check frogs".to_string(),
            enabled: true,
            durable: true,
            created_at_ms: 1_000,
            recurring: true,
            trigger: Trigger::Cron {
                expr: "*/5 * * * *".to_string(),
                tz: Some("UTC".to_string()),
            },
            action: Action::AgentTurn {
                prompt: "Check the frogs".to_string(),
                target: AgentTarget::ExistingChat {
                    chat_id: "chat-1".to_string(),
                },
                mode: Some("agent".to_string()),
                model: Some("model-1".to_string()),
                tools: Some(vec!["cat".to_string()]),
            },
            delivery: Delivery::Chat,
            last_fired_at_ms: Some(2_000),
            fire_count: 3,
            last_status: Some("ok".to_string()),
            last_error: None,
            last_delivery_error: None,
            recent_runs: vec![CronRunRecord {
                at_ms: 2_000,
                status: "ok".to_string(),
                error: None,
            }],
            paused_at_ms: None,
            trigger_at_ms: Some(3_000),
            auto_expire_after_ms: DEFAULT_RECURRING_AUTO_EXPIRE_AFTER_MS,
            retry_attempts: 0,
        }
    }

    #[test]
    fn legacy_flat_json_maps_to_cron_agentturn_chat() {
        let value = json!({
            "id": "cron_legacy",
            "cron": "7 * * * *",
            "prompt": "Check the frogs",
            "description": "Hourly frog check",
            "recurring": false,
            "durable": true,
            "created_at_ms": 123,
            "chat_id": "chat-1",
            "mode": "agent",
            "last_fired_at_ms": null,
            "fire_count": 0,
            "auto_expire_after_ms": 0
        });

        let job: Job = serde_json::from_value(value).unwrap();

        assert_eq!(
            job.trigger,
            Trigger::Cron {
                expr: "7 * * * *".to_string(),
                tz: None,
            }
        );
        assert_eq!(
            job.action,
            Action::AgentTurn {
                prompt: "Check the frogs".to_string(),
                target: AgentTarget::ExistingChat {
                    chat_id: "chat-1".to_string(),
                },
                mode: Some("agent".to_string()),
                model: None,
                tools: None,
            }
        );
        assert_eq!(job.delivery, Delivery::Chat);
        assert!(!job.recurring);
        assert_eq!(job.auto_expire_after_ms, 0);
        assert!(job.enabled);
    }

    #[test]
    fn new_nested_json_round_trips() {
        let job = nested_job();
        let json = serde_json::to_string(&job).unwrap();
        let round_tripped: Job = serde_json::from_str(&json).unwrap();

        assert_eq!(round_tripped, job);
    }

    #[test]
    fn notifier_delivery_from_value_accepts_target() {
        let delivery = delivery_from_value(&json!({
            "kind": "notifier",
            "integration_id": "notifier_telegram",
            "target": "chat-1",
        }))
        .unwrap();

        assert_eq!(
            delivery,
            Delivery::Notifier {
                integration_id: "notifier_telegram".to_string(),
                target: Some("chat-1".to_string()),
            }
        );
    }

    #[test]
    fn delivery_from_value_accepts_kind_and_type_aliases() {
        assert_eq!(
            delivery_from_value(&json!({"kind":"chat"})).unwrap(),
            Delivery::Chat
        );
        assert_eq!(
            delivery_from_value(&json!({"type":"chat"})).unwrap(),
            Delivery::Chat
        );
        assert_eq!(
            delivery_from_value(&json!({"kind":"webhook","url":"https://example.test/hook"}))
                .unwrap(),
            Delivery::Webhook {
                url: "https://example.test/hook".to_string(),
                token: None,
            }
        );
        assert_eq!(
            delivery_from_value(&json!({"type":"webhook","url":"https://example.test/hook"}))
                .unwrap(),
            Delivery::Webhook {
                url: "https://example.test/hook".to_string(),
                token: None,
            }
        );
        assert_eq!(
            delivery_from_value(&json!({"kind":"notifier","integration_id":"notifier_telegram"}))
                .unwrap(),
            Delivery::Notifier {
                integration_id: "notifier_telegram".to_string(),
                target: None,
            }
        );
        assert_eq!(
            delivery_from_value(&json!({"type":"notifier","integration_id":"notifier_telegram"}))
                .unwrap(),
            Delivery::Notifier {
                integration_id: "notifier_telegram".to_string(),
                target: None,
            }
        );
    }

    #[test]
    fn notifier_delivery_requires_integration_id() {
        let err = delivery_from_value(&json!({"kind": "notifier"})).unwrap_err();

        assert_eq!(err, "delivery notifier requires `integration_id`");
    }

    #[test]
    fn serialize_emits_nested_shape() {
        let serialized = serde_json::to_value(nested_job()).unwrap();

        assert_eq!(serialized["trigger"]["kind"], json!("cron"));
        assert_eq!(serialized["trigger"]["expr"], json!("*/5 * * * *"));
        assert_eq!(serialized["action"]["kind"], json!("agent_turn"));
        assert_eq!(
            serialized["action"]["target"]["kind"],
            json!("existing_chat")
        );
        assert_eq!(serialized["delivery"]["kind"], json!("chat"));
        assert!(serialized.get("cron").is_none());
        assert!(serialized.get("prompt").is_none());
    }

    #[test]
    fn recent_runs_are_preserved_on_deserialize() {
        let runs = (0..25)
            .map(|idx| json!({"at_ms": idx, "status": "ok", "error": null}))
            .collect::<Vec<_>>();
        let mut value = serde_json::to_value(nested_job()).unwrap();
        value["recent_runs"] = json!(runs);

        let job: Job = serde_json::from_value(value).unwrap();

        assert_eq!(job.recent_runs.len(), 25);
        assert_eq!(job.recent_runs[0].at_ms, 0);
        assert_eq!(job.recent_runs[24].at_ms, 24);
    }
}
