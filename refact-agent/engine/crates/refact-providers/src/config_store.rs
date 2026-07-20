use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex as AMutex;

use crate::identity::validate_provider_instance_id;

lazy_static::lazy_static! {
    static ref PROVIDER_CONFIG_LOCKS: std::sync::Mutex<HashMap<String, Arc<AMutex<()>>>> =
        std::sync::Mutex::new(HashMap::new());
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn provider_config_path(config_dir: &Path, instance_id: &str) -> PathBuf {
    config_dir
        .join("providers.d")
        .join(format!("{}.yaml", instance_id))
}

fn provider_config_lock(path: &Path) -> Arc<AMutex<()>> {
    let key = path.to_string_lossy().to_string();
    let mut locks = PROVIDER_CONFIG_LOCKS
        .lock()
        .expect("provider config lock table poisoned");
    locks
        .entry(key)
        .or_insert_with(|| Arc::new(AMutex::new(())))
        .clone()
}

async fn read_provider_config_value(path: &Path) -> Result<Option<serde_yaml::Value>, String> {
    let content = match tokio::fs::read_to_string(path).await {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Failed to read config: {}", error)),
    };
    let value = serde_yaml::from_str(&content)
        .map_err(|error| format!("Existing config is invalid YAML: {}", error))?;
    Ok(Some(value))
}

#[cfg(unix)]
async fn set_private_permissions(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let permissions = std::fs::Permissions::from_mode(0o600);
    tokio::fs::set_permissions(path, permissions)
        .await
        .map_err(|error| format!("Failed to set config permissions: {}", error))
}

#[cfg(not(unix))]
async fn set_private_permissions(_path: &Path) -> Result<(), String> {
    Ok(())
}

async fn write_provider_config_value(path: &Path, value: &serde_yaml::Value) -> Result<(), String> {
    let providers_dir = path
        .parent()
        .ok_or_else(|| "Provider config path has no parent directory".to_string())?;
    tokio::fs::create_dir_all(providers_dir)
        .await
        .map_err(|error| format!("Failed to create providers.d: {}", error))?;

    let content = serde_yaml::to_string(value)
        .map_err(|error| format!("Failed to serialize config: {}", error))?;
    let unique_id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp_path = path.with_extension(format!("yaml.tmp.{}.{}", std::process::id(), unique_id));

    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create_new(true).truncate(true);
    #[cfg(unix)]
    {
        options.mode(0o600);
    }

    let mut file = options
        .open(&temp_path)
        .await
        .map_err(|error| format!("Failed to write temp config: {}", error))?;
    file.write_all(content.as_bytes())
        .await
        .map_err(|error| format!("Failed to write temp config: {}", error))?;
    file.flush()
        .await
        .map_err(|error| format!("Failed to write temp config: {}", error))?;
    drop(file);

    set_private_permissions(&temp_path).await?;
    tokio::fs::rename(&temp_path, path)
        .await
        .map_err(|error| format!("Failed to rename config: {}", error))?;
    set_private_permissions(path).await?;

    Ok(())
}

#[allow(dead_code)]
pub async fn write_provider_config(
    config_dir: &Path,
    instance_id: &str,
    settings: serde_yaml::Value,
) -> Result<(), String> {
    validate_provider_instance_id(instance_id)?;
    let path = provider_config_path(config_dir, instance_id);
    let lock = provider_config_lock(&path);
    let _guard = lock.lock().await;
    write_provider_config_value(&path, &settings).await
}

pub async fn delete_provider_config(config_dir: &Path, instance_id: &str) -> Result<(), String> {
    validate_provider_instance_id(instance_id)?;
    let path = provider_config_path(config_dir, instance_id);
    let lock = provider_config_lock(&path);
    let _guard = lock.lock().await;
    match tokio::fs::remove_file(&path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Failed to delete config: {}", error)),
    }
}

pub async fn update_provider_config_with<E, F, M>(
    config_dir: &Path,
    instance_id: &str,
    map_store_error: M,
    update: F,
) -> Result<serde_yaml::Value, E>
where
    F: FnOnce(Option<serde_yaml::Value>) -> Result<serde_yaml::Value, E>,
    M: Fn(String) -> E,
{
    validate_provider_instance_id(instance_id).map_err(&map_store_error)?;
    let path = provider_config_path(config_dir, instance_id);
    let lock = provider_config_lock(&path);
    let _guard = lock.lock().await;
    let existing = match read_provider_config_value(&path).await {
        Ok(existing) => existing,
        Err(error) => {
            let mapped = if error.contains("invalid YAML") {
                format!(
                    "Existing config is invalid YAML: {}. Fix manually or delete the file.",
                    error.trim_start_matches("Existing config is invalid YAML: ")
                )
            } else {
                error
            };
            return Err(map_store_error(mapped));
        }
    };
    let updated = update(existing)?;
    write_provider_config_value(&path, &updated)
        .await
        .map_err(map_store_error)?;
    Ok(updated)
}

pub async fn update_provider_config<F>(
    config_dir: &Path,
    instance_id: &str,
    update: F,
) -> Result<serde_yaml::Value, String>
where
    F: FnOnce(Option<serde_yaml::Value>) -> Result<serde_yaml::Value, String>,
{
    update_provider_config_with(config_dir, instance_id, |error| error, update).await
}

pub async fn update_provider_config_if<F>(
    config_dir: &Path,
    instance_id: &str,
    update: F,
) -> Result<Option<serde_yaml::Value>, String>
where
    F: FnOnce(Option<serde_yaml::Value>) -> Result<Option<serde_yaml::Value>, String>,
{
    validate_provider_instance_id(instance_id)?;
    let path = provider_config_path(config_dir, instance_id);
    let lock = provider_config_lock(&path);
    let _guard = lock.lock().await;
    let existing = read_provider_config_value(&path).await?;
    let Some(updated) = update(existing)? else {
        return Ok(None);
    };
    write_provider_config_value(&path, &updated).await?;
    Ok(Some(updated))
}
