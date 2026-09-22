use std::sync::Arc;
use crate::global_context::GlobalContext;
use crate::yaml_configs::customization_types::*;

fn normalize_kind(kind: &str) -> &str {
    if kind == "delegates" {
        "subagents"
    } else {
        kind
    }
}

pub async fn write_file_atomic(path: &std::path::Path, content: &str) -> std::io::Result<()> {
    let path = path.to_path_buf();
    let bytes = content.as_bytes().to_vec();
    tokio::task::spawn_blocking(move || {
        let expected = match std::fs::read(&path) {
            Ok(bytes) => Some(version(&bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        replace_checked(&path, expected.as_deref(), &bytes)
    })
    .await
    .map_err(std::io::Error::other)?
}

pub fn version(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn replace_checked(
    path: &std::path::Path,
    expected: Option<&str>,
    bytes: &[u8],
) -> std::io::Result<()> {
    use std::io::Write;
    static MUTATION: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _guard = MUTATION
        .lock()
        .map_err(|_| std::io::Error::other("customization mutation lock poisoned"))?;
    let check = || -> std::io::Result<()> {
        for ancestor in path.ancestors() {
            if let Ok(metadata) = std::fs::symlink_metadata(ancestor) {
                if metadata.file_type().is_symlink() {
                    return Err(std::io::Error::other("customization symlink refused"));
                }
            }
        }
        let current = match std::fs::read(path) {
            Ok(bytes) => Some(version(&bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        if current.as_deref() != expected {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "customization version changed",
            ));
        }
        Ok(())
    };
    check()?;
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other("customization parent missing"))?;
    std::fs::create_dir_all(parent)?;
    let temp = parent.join(format!(".customization-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        check()?;
        std::fs::rename(&temp, path)?;
        #[cfg(unix)]
        {
            std::fs::File::open(parent)?.sync_all()?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

pub(crate) fn validate_kind(kind: &str) -> std::result::Result<&str, String> {
    match kind {
        "modes" | "subagents" | "delegates" | "toolbox_commands" | "code_lens" => Ok(kind),
        "hooks" => Err("hooks are managed by ext hooks routes".to_string()),
        _ => Err(format!("invalid kind: {}", kind)),
    }
}

pub(crate) fn validate_id(id: &str) -> std::result::Result<(), String> {
    if id.is_empty() {
        return Err("id cannot be empty".to_string());
    }
    if id.contains('/') || id.contains('\\') || id.contains("..") {
        return Err("id contains invalid characters".to_string());
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
    {
        return Err(
            "id must contain only lowercase letters, digits, underscore, or hyphen".to_string(),
        );
    }
    Ok(())
}

const MAX_SUPPORTED_SCHEMA_VERSION: u32 = 100;

pub(crate) fn validate_config(
    kind: &str,
    config: &serde_json::Value,
    expected_id: &str,
) -> std::result::Result<(), String> {
    let kind = normalize_kind(kind);
    let config_id = config.get("id").and_then(|v| v.as_str()).unwrap_or("");
    if config_id != expected_id {
        return Err(format!(
            "config id '{}' does not match expected '{}'",
            config_id, expected_id
        ));
    }

    let schema_version = config
        .get("schema_version")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| "missing or invalid schema_version field".to_string())?;
    if schema_version == 0 || schema_version > MAX_SUPPORTED_SCHEMA_VERSION as u64 {
        return Err(format!(
            "schema_version {} is not supported (must be 1..={})",
            schema_version, MAX_SUPPORTED_SCHEMA_VERSION
        ));
    }

    let json_str = serde_json::to_string(config).map_err(|e| e.to_string())?;
    match kind {
        "modes" => {
            serde_json::from_str::<ModeConfig>(&json_str).map_err(|e| e.to_string())?;
        }
        "subagents" => {
            serde_json::from_str::<SubagentConfig>(&json_str).map_err(|e| e.to_string())?;
        }
        "toolbox_commands" => {
            serde_json::from_str::<ToolboxCommandConfig>(&json_str).map_err(|e| e.to_string())?;
        }
        "code_lens" => {
            serde_json::from_str::<CodeLensConfig>(&json_str).map_err(|e| e.to_string())?;
        }
        _ => {
            return Err(format!("unknown kind: {}", kind));
        }
    }
    Ok(())
}

pub fn merge_preserving_unknown(
    original: &serde_json::Value,
    proposal: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let (Some(original), Some(proposal)) = (original.as_object(), proposal.as_object()) else {
        return Err("customization must be a mapping".into());
    };
    let mut merged = original.clone();
    for (key, value) in proposal {
        let next = if value.is_object() && merged.get(key).is_some_and(serde_json::Value::is_object)
        {
            merge_preserving_unknown(&merged[key], value)?
        } else {
            value.clone()
        };
        merged.insert(key.clone(), next);
    }
    Ok(serde_json::Value::Object(merged))
}

pub fn reject_authority_widening(
    original: &serde_json::Value,
    proposal: &serde_json::Value,
) -> Result<(), String> {
    if let Some(map) = proposal.as_object() {
        for (key, next) in map {
            let old = original.get(key).unwrap_or(&serde_json::Value::Null);
            if [
                "tools",
                "allowed_tools",
                "model",
                "model_type",
                "model_name",
                "access",
                "permissions",
                "allow",
                "deny",
                "confirmation",
                "sandbox",
                "auto_approve",
            ]
            .contains(&key.as_str())
                && old != next
            {
                let narrowing = ["tools", "allowed_tools"].contains(&key.as_str())
                    && next.as_array().is_some_and(|values| {
                        values.is_empty()
                            || old
                                .as_array()
                                .is_some_and(|previous| values.iter().all(|v| previous.contains(v)))
                    });
                if !narrowing {
                    return Err(format!(
                        "authority field requires separate explicit review: {key}"
                    ));
                }
            }
            if next.is_object() {
                reject_authority_widening(old, next)?;
            }
        }
    }
    Ok(())
}

pub async fn reload_project(gcx: Arc<GlobalContext>, root: &std::path::Path) -> Result<(), String> {
    crate::yaml_configs::customization_registry::invalidate_all_registry_caches(gcx.clone()).await;
    let registry = crate::yaml_configs::customization_registry::load_merged_registry(
        &gcx.config_dir,
        Some(root),
    )
    .await;
    if !registry.errors.is_empty() {
        return Err("customization registry reload has validation errors".into());
    }
    gcx.project_registry_cache
        .write()
        .map_err(|_| "customization cache unavailable".to_string())?
        .insert(root.to_path_buf(), registry);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_fields_survive_and_authority_changes_fail_closed() {
        let old = serde_json::json!({"unknown": {"preserved": true}, "tools": ["read"], "subchat": {"model": "reviewed", "extra": 1}});
        let merged = merge_preserving_unknown(&old, &serde_json::json!({"title":"new"})).unwrap();
        assert_eq!(merged["unknown"], old["unknown"]);
        assert!(reject_authority_widening(&old, &merged).is_ok());
        assert!(
            reject_authority_widening(&old, &serde_json::json!({"tools":["read", "shell"]}))
                .is_err()
        );
        assert!(reject_authority_widening(
            &old,
            &serde_json::json!({"subchat":{"model":"unreviewed"}})
        )
        .is_err());
        assert!(reject_authority_widening(&old, &serde_json::json!({"tools":[]})).is_ok());
    }
}
