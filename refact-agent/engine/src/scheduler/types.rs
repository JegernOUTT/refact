use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const DEFAULT_RECURRING_AUTO_EXPIRE_AFTER_MS: u64 = 30 * 24 * 60 * 60 * 1000;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ScheduledTask {
    pub id: String,
    pub cron: String,
    pub prompt: String,
    pub description: String,
    pub recurring: bool,
    pub durable: bool,
    pub created_at_ms: u64,
    pub chat_id: Option<String>,
    pub mode: Option<String>,
    pub last_fired_at_ms: Option<u64>,
    pub fire_count: u32,
    pub auto_expire_after_ms: u64,
}

impl ScheduledTask {
    pub fn new(
        cron: String,
        prompt: String,
        description: String,
        recurring: bool,
        durable: bool,
        created_at_ms: u64,
    ) -> Self {
        Self {
            id: format!("cron_{}", Uuid::now_v7()),
            cron,
            prompt,
            description,
            recurring,
            durable,
            created_at_ms,
            chat_id: None,
            mode: None,
            last_fired_at_ms: None,
            fire_count: 0,
            auto_expire_after_ms: if recurring {
                DEFAULT_RECURRING_AUTO_EXPIRE_AFTER_MS
            } else {
                0
            },
        }
    }
}
