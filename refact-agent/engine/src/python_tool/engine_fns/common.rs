//! Helpers shared by several areas.

use serde_json::{json, Value};

use super::{EngineCtx, EngineError};

pub const LIST_DEFAULT: usize = 100;
pub const LIST_MAX: usize = 500;

/// Schema of the `limit` argument of a bounded list.
pub fn limit_schema() -> Value {
    json!({"type":"integer","minimum":1,"maximum":LIST_MAX,"description":"Rows to return; 100 by default, at most 500."})
}

pub fn limit(args: &Value) -> usize {
    args.get("limit")
        .and_then(Value::as_u64)
        .map_or(LIST_DEFAULT, |n| (n as usize).clamp(1, LIST_MAX))
}

pub fn redact(text: &str) -> String {
    crate::tools::tool_buddy_get_logs::redact_sensitive(text)
}

/// Functions that look across chats run only in a root chat a person drives, never in child or agent chats.
pub async fn require_root_chat(ctx: &EngineCtx, name: &str) -> Result<(), EngineError> {
    let context = ctx.ccx.lock().await;
    let agent = context.background_agent_id.is_some()
        || context.subchat_depth > 0
        || context
            .task_meta
            .as_ref()
            .is_some_and(|meta| matches!(meta.role.as_str(), "agent" | "agents"));
    if agent {
        return Err(EngineError::Permission(format!(
            "{name} looks across chats and runs only in a root chat, not in a child or agent chat"
        )));
    }
    Ok(())
}

/// Refuses a project file whose real location is outside the project or inside the engine's
/// config directory, for example through a `.refact` symlink into the global config; the parts of
/// the path that do not exist yet are checked through their nearest existing parent.
pub fn inside_project(
    root: &std::path::Path,
    path: &std::path::Path,
    config_dir: &std::path::Path,
) -> Result<(), EngineError> {
    let refuse = || {
        EngineError::Permission(format!(
            "{} resolves outside this project (a symlink?); scripts write project files only inside the project",
            path.display()
        ))
    };
    let real_root = std::fs::canonicalize(root).map_err(failed)?;
    let mut probe = path.to_path_buf();
    let real = loop {
        match std::fs::canonicalize(&probe) {
            Ok(real) => break real,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if std::fs::symlink_metadata(&probe).is_ok() || !probe.pop() {
                    return Err(refuse());
                }
            }
            Err(e) => return Err(failed(e)),
        }
    };
    let in_config = std::fs::canonicalize(config_dir).is_ok_and(|dir| real.starts_with(dir));
    if !real.starts_with(&real_root) || in_config {
        return Err(refuse());
    }
    Ok(())
}

pub fn failed(error: impl std::fmt::Display) -> EngineError {
    EngineError::Failed(error.to_string())
}

pub fn to_value(value: impl serde::Serialize) -> Result<Value, EngineError> {
    serde_json::to_value(value).map_err(failed)
}

/// Validates a `YYYY-MM-DD` date argument.
pub fn date(args: &Value, key: &str) -> Result<Option<String>, EngineError> {
    match args.get(key).and_then(Value::as_str) {
        None => Ok(None),
        Some(text) => chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d")
            .map(|_| Some(text.to_string()))
            .map_err(|_| {
                EngineError::Type(format!("{key} must be a date YYYY-MM-DD, got {text:?}"))
            }),
    }
}

pub fn date_range(args: &Value) -> Result<(Option<String>, Option<String>), EngineError> {
    let (from, to) = (date(args, "from")?, date(args, "to")?);
    if let (Some(from), Some(to)) = (&from, &to) {
        if from > to {
            return Err(EngineError::Type(format!("from {from} is after to {to}")));
        }
    }
    Ok((from, to))
}

/// Path privacy for the chat's model destination: answers whether a path may appear in a result.
pub struct PathFilter {
    snapshot: std::sync::Arc<crate::privacy::classification_cache::Snapshot>,
    destination: refact_privacy::Destination,
    gcx: std::sync::Arc<crate::global_context::GlobalContext>,
}

impl PathFilter {
    /// Fails when the privacy policy cannot be loaded, so callers never return unfiltered paths.
    pub async fn new(ctx: &EngineCtx) -> Result<Self, EngineError> {
        let model = ctx.ccx.lock().await.current_model.clone();
        let gcx = ctx.app.gcx.clone();
        crate::privacy::load_privacy_if_needed(gcx.clone()).await;
        let snapshot = crate::privacy::classification_cache::snapshot(&gcx).map_err(|e| {
            EngineError::Failed(format!(
                "the privacy policy could not be loaded ({e}); nothing was returned"
            ))
        })?;
        Ok(Self {
            snapshot,
            destination: crate::privacy::records::provider_destination(&model),
            gcx,
        })
    }

    #[cfg(test)]
    pub(crate) async fn new_for_test(app: &crate::app_state::AppState, model: &str) -> Self {
        let gcx = app.gcx.clone();
        crate::privacy::load_privacy_if_needed(gcx.clone()).await;
        Self {
            snapshot: crate::privacy::classification_cache::snapshot(&gcx).unwrap(),
            destination: crate::privacy::records::provider_destination(model),
            gcx,
        }
    }

    pub fn allows(&self, path: &str) -> bool {
        let zone = self
            .snapshot
            .classify(&self.gcx, std::path::Path::new(path))
            .zone;
        zone.name != "blocked" && self.destination.matches_send_to(&zone.send_to)
    }

    /// False when any absolute path token of the text is not allowed.
    pub fn allows_text(&self, text: &str) -> bool {
        text.split(|c: char| {
            c.is_whitespace()
                || matches!(
                    c,
                    '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '<' | '>' | ',' | ';' | '='
                )
        })
        .map(|token| token.trim_end_matches([':', '.']))
        .filter(|token| {
            token.starts_with('/')
                || token.starts_with('~')
                || (token.len() > 2
                    && token.as_bytes()[1] == b':'
                    && matches!(token.as_bytes()[2], b'\\' | b'/'))
        })
        .all(|token| self.allows(token))
    }
}
