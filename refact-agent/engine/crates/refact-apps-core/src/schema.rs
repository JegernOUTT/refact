use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppManifest {
    pub schema_version: u32,
    pub app_id: Uuid,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default = "entrypoint")]
    pub entrypoint: String,
    #[serde(default)]
    pub sources: Vec<SourceDefinition>,
    #[serde(default)]
    pub actions: Vec<ActionDefinition>,
    /// Declarations only. Runtime authorization must come from separate user grants.
    #[serde(default)]
    pub permissions: Vec<Permission>,
}

fn entrypoint() -> String {
    "ui/index.html".into()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Permission {
    Network { origin: String },
    Process,
    Internal { topic: String },
    ChatRead,
    ChatWrite,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceDefinition {
    pub source_id: String,
    pub channel: String,
    pub driver: SourceDriver,
    #[serde(default)]
    pub permissions: Vec<Permission>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum SourceDriver {
    Manual,
    FileTail {
        path: String,
        every_ms: u64,
    },
    Interval {
        every_ms: u64,
    },
    HttpPoll {
        url: String,
        every_ms: u64,
    },
    Sse {
        url: String,
    },
    Websocket {
        url: String,
    },
    Process {
        process_id: String,
        #[serde(default)]
        chat_id: Option<String>,
    },
    Internal {
        topic: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionDefinition {
    pub action_id: String,
    pub channel: String,
    #[serde(default)]
    pub operation: ActionOperation,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub permissions: Vec<Permission>,
}

/// Fixed declarative operations only; declarations never confer host authority.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ActionOperation {
    #[default]
    EventEmit,
    StateSet,
    SourceRefresh {
        source_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum StateScope {
    Shared,
    Binding { binding_id: Uuid },
    Standalone,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppState {
    pub revision: u64,
    pub value: serde_json::Value,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            revision: 0,
            value: serde_json::Value::Null,
        }
    }
}

impl AppManifest {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1
            || self.app_id.is_nil()
            || self.name.trim().is_empty()
            || self.name.len() > 200
            || self.description.len() > 8192
        {
            return Err("invalid manifest identity, schema version or description".into());
        }
        crate::validate_resource_path(&self.entrypoint)?;
        if let Some(icon) = &self.icon {
            if let Some(name) = icon.strip_prefix("lucide:") {
                if name.is_empty()
                    || name.len() > 80
                    || !name.split('-').all(|part| {
                        !part.is_empty()
                            && part
                                .bytes()
                                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
                    })
                {
                    return Err("invalid Lucide icon identifier".into());
                }
            } else {
                crate::validate_resource_path(icon)?;
            }
        }
        let mut ids = BTreeSet::new();
        for source in &self.sources {
            validate_identifier(&source.source_id)?;
            validate_identifier(&source.channel)?;
            if !ids.insert(&source.source_id) {
                return Err("duplicate source_id".into());
            }
            match &source.driver {
                SourceDriver::FileTail {
                    path,
                    every_ms,
                } => {
                    crate::validate_resource_path(path)?;
                    if *every_ms < 1000
                        || path.contains('\\')
                        || !std::path::Path::new(path)
                            .components()
                            .all(|c| matches!(c, std::path::Component::Normal(_)))
                    {
                        return Err("invalid file tail path or interval".into());
                    }
                }
                SourceDriver::Interval {
                    every_ms,
                }
                | SourceDriver::HttpPoll {
                    every_ms, ..
                } if *every_ms < 1000 => return Err("source interval must be >=1000ms".into()),
                SourceDriver::Process {
                    process_id,
                    chat_id,
                } if process_id.is_empty()
                    || process_id.len() > 256
                    || chat_id
                        .as_ref()
                        .is_some_and(|id| id.is_empty() || id.len() > 256) =>
                {
                    return Err("invalid process observation target".into())
                }
                _ => {}
            }
        }
        ids.clear();
        for action in &self.actions {
            validate_identifier(&action.action_id)?;
            validate_identifier(&action.channel)?;
            if let ActionOperation::SourceRefresh {
                source_id,
            } = &action.operation
            {
                validate_identifier(source_id)?;
                if !self.sources.iter().any(|s| &s.source_id == source_id) {
                    return Err("action refresh source is not declared".into());
                }
            }
            if !ids.insert(&action.action_id) {
                return Err("duplicate action_id".into());
            }
        }
        Ok(())
    }
}

pub fn validate_identifier(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 80
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("identifier must be 1..80 ASCII letters, digits, '-' or '_'".into());
    }
    Ok(())
}
