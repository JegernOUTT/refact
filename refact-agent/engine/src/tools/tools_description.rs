use std::collections::HashMap;
use std::sync::Arc;
use serde_json::Value;
use async_trait::async_trait;
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatUsage, ContextEnum};
use crate::custom_error::MapErrToString;
use crate::integrations::integr_abstract::IntegrationConfirmation;

pub use refact_tool_api::{
    command_should_be_confirmed_by_user, command_should_be_confirmed_by_user_segment_aware,
    command_should_be_denied, command_should_be_denied_segment_aware, extract_command_segments,
    is_strict_compatible, json_schema_from_params, make_openai_tool_value, structural_flags,
    MatchConfirmDeny, MatchConfirmDenyResult, ToolConfig, ToolDesc, ToolGroupCategory, ToolSource,
    ToolSourceType,
};

const TOOL_CONFIG_CACHE_CAPACITY: usize = 128;

type ToolConfigFingerprint = Option<(std::time::SystemTime, u64)>;

struct CachedToolConfigYaml {
    generation: u64,
    fingerprint: ToolConfigFingerprint,
    last_used: u64,
    value: Arc<serde_yaml::Value>,
}

#[derive(Default)]
struct ToolConfigYamlCache {
    entries: std::sync::Mutex<HashMap<String, CachedToolConfigYaml>>,
    build_locks: std::sync::Mutex<HashMap<String, Arc<std::sync::Mutex<()>>>>,
}

static TOOL_CONFIG_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static TOOL_CONFIG_USE_TICK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[cfg(test)]
static TOOL_CONFIG_YAML_READS: std::sync::OnceLock<std::sync::Mutex<HashMap<String, u64>>> =
    std::sync::OnceLock::new();

impl ToolConfigYamlCache {
    fn get(
        &self,
        config_path: &str,
        generation: u64,
        fingerprint: &ToolConfigFingerprint,
    ) -> Option<Arc<serde_yaml::Value>> {
        if fingerprint.is_none() {
            return None;
        }
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let entry = entries.get_mut(config_path)?;
        if entry.generation != generation || entry.fingerprint != *fingerprint {
            return None;
        }
        entry.last_used = TOOL_CONFIG_USE_TICK.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Some(entry.value.clone())
    }

    fn insert(
        &self,
        config_path: &str,
        generation: u64,
        fingerprint: ToolConfigFingerprint,
        value: Arc<serde_yaml::Value>,
    ) {
        if fingerprint.is_none() {
            return;
        }
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if entries.len() >= TOOL_CONFIG_CACHE_CAPACITY && !entries.contains_key(config_path) {
            let evict = entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(path, _)| path.clone());
            if let Some(evict) = evict {
                entries.remove(&evict);
            }
        }
        entries.insert(
            config_path.to_string(),
            CachedToolConfigYaml {
                generation,
                fingerprint,
                last_used: TOOL_CONFIG_USE_TICK.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                value,
            },
        );
    }

    fn acquire_build_lock(&self, config_path: &str) -> Arc<std::sync::Mutex<()>> {
        let mut locks = self
            .build_locks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if locks.len() >= TOOL_CONFIG_CACHE_CAPACITY {
            locks.retain(|path, lock| path == config_path || Arc::strong_count(lock) > 1);
        }
        locks
            .entry(config_path.to_string())
            .or_insert_with(|| Arc::new(std::sync::Mutex::new(())))
            .clone()
    }
}

fn tool_config_yaml_cache() -> &'static ToolConfigYamlCache {
    static CACHE: std::sync::OnceLock<ToolConfigYamlCache> = std::sync::OnceLock::new();
    CACHE.get_or_init(ToolConfigYamlCache::default)
}

pub fn invalidate_tool_config_cache() {
    TOOL_CONFIG_GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}

fn tool_config_fingerprint(config_path: &str) -> ToolConfigFingerprint {
    let metadata = std::fs::metadata(config_path).ok()?;
    Some((metadata.modified().ok()?, metadata.len()))
}

fn read_tool_config_yaml(config_path: &str) -> Result<String, String> {
    #[cfg(test)]
    count_tool_config_yaml_read(config_path);
    std::fs::read_to_string(config_path).map_err(|e| format!("Error reading config file: {}", e))
}

#[cfg(test)]
fn count_tool_config_yaml_read(config_path: &str) {
    *TOOL_CONFIG_YAML_READS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .entry(config_path.to_string())
        .or_default() += 1;
}

fn load_tool_config_yaml(config_path: &str) -> Result<Arc<serde_yaml::Value>, String> {
    let generation = TOOL_CONFIG_GENERATION.load(std::sync::atomic::Ordering::Acquire);
    let fingerprint = tool_config_fingerprint(config_path);
    let cache = tool_config_yaml_cache();
    if let Some(value) = cache.get(config_path, generation, &fingerprint) {
        return Ok(value);
    }
    let build_lock = cache.acquire_build_lock(config_path);
    let _build_guard = build_lock
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(value) = cache.get(config_path, generation, &fingerprint) {
        return Ok(value);
    }
    let raw = read_tool_config_yaml(config_path)?;
    let parsed: serde_yaml::Value =
        serde_yaml::from_str(&raw).map_err(|e| format!("Error parsing config file: {}", e))?;
    let value = Arc::new(parsed);
    cache.insert(config_path, generation, fingerprint, value.clone());
    Ok(value)
}

pub fn tool_config_from_yaml(config_path: &str, tool_name: &str) -> Result<ToolConfig, String> {
    let yaml = load_tool_config_yaml(config_path)?;
    match yaml.get("tools").and_then(|tools| tools.get(tool_name)) {
        None => Ok(ToolConfig::default()),
        Some(config) => Ok(serde_yaml::from_value(config.clone()).unwrap_or_default()),
    }
}

pub struct ToolGroup {
    pub name: String,
    pub description: String,
    pub category: ToolGroupCategory,
    pub tools: Vec<Box<dyn Tool + Send>>,
}

#[async_trait]
pub trait Tool: Send + Sync {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String>;

    fn tool_description(&self) -> ToolDesc;

    async fn match_against_confirm_deny(
        &self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        args: &HashMap<String, Value>,
    ) -> Result<MatchConfirmDeny, String> {
        let command_to_match = self
            .command_to_match_against_confirm_deny(ccx.clone(), &args)
            .await
            .map_err(|e| format!("Error getting tool command to match: {}", e))?;

        if !command_to_match.is_empty() {
            if let Some(rules) = &self.confirm_deny_rules() {
                tracing::info!(
                    "confirmation: match {:?} against {:?}",
                    command_to_match,
                    rules
                );
                let (is_denied, deny_rule) =
                    command_should_be_denied_segment_aware(&command_to_match, &rules.deny);
                if is_denied {
                    return Ok(MatchConfirmDeny {
                        result: MatchConfirmDenyResult::DENY,
                        command: command_to_match.clone(),
                        rule: deny_rule.clone(),
                    });
                }
                let (needs_confirmation, confirmation_rule) =
                    command_should_be_confirmed_by_user_segment_aware(
                        &command_to_match,
                        &rules.ask_user,
                    );
                if needs_confirmation {
                    return Ok(MatchConfirmDeny {
                        result: MatchConfirmDenyResult::CONFIRMATION,
                        command: command_to_match.clone(),
                        rule: confirmation_rule.clone(),
                    });
                }
            } else {
                tracing::error!("No confirmation info available for {:?}", command_to_match);
            }
            if let Some(flag) = structural_flags(&extract_command_segments(&command_to_match))
                .into_iter()
                .next()
            {
                return Ok(MatchConfirmDeny {
                    result: MatchConfirmDenyResult::CONFIRMATION,
                    command: command_to_match.clone(),
                    rule: flag.to_string(),
                });
            }
        }
        Ok(MatchConfirmDeny {
            result: MatchConfirmDenyResult::PASS,
            command: command_to_match.clone(),
            rule: "".to_string(),
        })
    }

    async fn command_to_match_against_confirm_deny(
        &self,
        _ccx: Arc<AMutex<AtCommandsContext>>,
        _args: &HashMap<String, Value>,
    ) -> Result<String, String> {
        Ok("".to_string())
    }

    fn confirm_deny_rules(&self) -> Option<IntegrationConfirmation> {
        None
    }

    fn has_config_path(&self) -> Option<String> {
        return None;
    }

    fn config(&self) -> Result<ToolConfig, String> {
        let tool_desc = self.tool_description();
        tool_config_from_yaml(&tool_desc.source.config_path, &tool_desc.name)
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    } // "ast", "vecdb"

    #[allow(dead_code)] // Trait method for future usage tracking
    fn usage(&mut self) -> &mut Option<ChatUsage> {
        static mut DEFAULT_USAGE: Option<ChatUsage> = None;
        #[allow(static_mut_refs)]
        unsafe {
            &mut DEFAULT_USAGE
        }
    }
}

pub async fn set_tool_config(
    config_path: String,
    tool_name: String,
    new_config: ToolConfig,
) -> Result<(), String> {
    let config_file = tokio::fs::read_to_string(&config_path)
        .await
        .map_err(|e| format!("Error reading config file: {}", e))?;

    let mut config: serde_yaml::Mapping = serde_yaml::from_str(&config_file)
        .map_err(|e| format!("Error parsing config file: {}", e))?;

    let tools: &mut serde_yaml::Mapping = match config
        .get_mut("tools")
        .and_then(|tools| tools.as_mapping_mut())
    {
        Some(tools) => tools,
        None => {
            config.insert(
                serde_yaml::Value::String("tools".to_string()),
                serde_yaml::Value::Mapping(serde_yaml::Mapping::new()),
            );
            config
                .get_mut("tools")
                .expect("tools was just inserted")
                .as_mapping_mut()
                .expect("tools is a mapping, it was just inserted")
        }
    };

    tools.insert(
        serde_yaml::Value::String(tool_name),
        serde_yaml::to_value(new_config)
            .map_err_with_prefix("ToolConfig should always be serializable.")?,
    );

    tokio::fs::write(config_path, serde_yaml::to_string(&config).unwrap())
        .await
        .map_err(|e| format!("Error writing config file: {}", e))?;
    invalidate_tool_config_cache();

    Ok(())
}

/// A wait-only cancellation signal; observing it never aborts the chat turn.
#[derive(Clone)]
pub(crate) struct WaitInterrupt {
    pub(crate) abort_flag: Arc<std::sync::atomic::AtomicBool>,
    pub(crate) wait_flag: Arc<std::sync::atomic::AtomicBool>,
    pub(crate) notify: Option<Arc<tokio::sync::Notify>>,
}

impl WaitInterrupt {
    pub(crate) async fn from_context(ccx: &Arc<AMutex<AtCommandsContext>>) -> Self {
        let (app, chat_id, abort_flag) = {
            let context = ccx.lock().await;
            (
                context.app.clone(),
                context.chat_id.clone(),
                context.abort_flag.clone(),
            )
        };
        let session = app.chat.sessions.read().await.get(&chat_id).cloned();
        let (wait_flag, notify) = if let Some(session) = session {
            let session = session.lock().await;
            (
                session.wait_interrupt_flag.clone(),
                Some(session.abort_notify.clone()),
            )
        } else {
            (Arc::new(std::sync::atomic::AtomicBool::new(false)), None)
        };
        Self {
            abort_flag,
            wait_flag,
            notify,
        }
    }

    pub(crate) fn interrupted(&self) -> bool {
        self.abort_flag.load(std::sync::atomic::Ordering::SeqCst)
            || self.wait_flag.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub(crate) fn reason(&self, elapsed_seconds: f64) -> String {
        let cause = if self.wait_flag.load(std::sync::atomic::Ordering::SeqCst) {
            "message arrived"
        } else {
            "user aborted"
        };
        format!("interrupted after {elapsed_seconds:.1}s: {cause}")
    }

    pub(crate) async fn wait(&self) {
        loop {
            let notified = self.notify.as_ref().map(|notify| notify.notified());
            tokio::pin!(notified);
            if let Some(notified) = notified.as_mut().as_pin_mut() {
                notified.enable();
            }
            if self.interrupted() {
                return;
            }
            tokio::select! {
                _ = async {
                    if let Some(notified) = notified.as_mut().as_pin_mut() {
                        notified.await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => {},
                _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {},
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeTool {
        name: String,
        config_path: String,
    }

    #[async_trait]
    impl Tool for FakeTool {
        async fn tool_execute(
            &mut self,
            _ccx: Arc<AMutex<AtCommandsContext>>,
            _tool_call_id: &String,
            _args: &HashMap<String, Value>,
        ) -> Result<(bool, Vec<ContextEnum>), String> {
            Ok((false, vec![]))
        }

        fn tool_description(&self) -> ToolDesc {
            ToolDesc {
                name: self.name.clone(),
                experimental: false,
                allow_parallel: true,
                description: String::new(),
                input_schema: serde_json::json!({"type": "object", "properties": {}}),
                output_schema: None,
                annotations: None,
                display_name: self.name.clone(),
                source: ToolSource {
                    source_type: ToolSourceType::Builtin,
                    config_path: self.config_path.clone(),
                },
            }
        }
    }

    fn tool(name: &str, config_path: &str) -> FakeTool {
        FakeTool {
            name: name.to_string(),
            config_path: config_path.to_string(),
        }
    }
}
