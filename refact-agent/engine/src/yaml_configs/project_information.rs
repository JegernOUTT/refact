pub use refact_yaml_configs::project_information::*;

use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;

use crate::global_context::GlobalContext;

async fn get_project_dirs(gcx: Arc<GlobalContext>) -> Vec<PathBuf> {
    crate::files_correction::get_project_dirs(gcx).await
}

async fn get_config_path(gcx: Arc<GlobalContext>) -> Option<PathBuf> {
    let dirs = get_project_dirs(gcx).await;
    dirs.first()
        .map(|d| d.join(".refact").join("project_information.yaml"))
}

pub async fn load_project_information_config(
    gcx: Arc<GlobalContext>,
) -> ProjectInformationConfig {
    let Some(path) = get_config_path(gcx.clone()).await else {
        return ProjectInformationConfig::default();
    };

    match tokio::fs::metadata(&path).await {
        Ok(_) => {}
        Err(_) => {
            let _ = ensure_default_config_exists(gcx).await;
        }
    }

    match tokio::fs::read_to_string(&path).await {
        Ok(content) => serde_yaml::from_str(&content).unwrap_or_default(),
        Err(_) => ProjectInformationConfig::default(),
    }
}

pub async fn save_project_information_config(
    gcx: Arc<GlobalContext>,
    config: &ProjectInformationConfig,
) -> std::io::Result<()> {
    let Some(path) = get_config_path(gcx).await else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "No project directory",
        ));
    };
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let tmp_path = path.with_extension("yaml.tmp");
    let yaml = serde_yaml::to_string(config)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    tokio::fs::write(&tmp_path, &yaml).await?;
    tokio::fs::rename(&tmp_path, &path).await?;
    Ok(())
}

pub async fn ensure_default_config_exists(
    gcx: Arc<GlobalContext>,
) -> std::io::Result<bool> {
    let Some(path) = get_config_path(gcx.clone()).await else {
        return Ok(false);
    };

    if tokio::fs::metadata(&path).await.is_ok() {
        return Ok(false);
    }

    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let config = ProjectInformationConfig::default();
    let tmp_path = path.with_extension("yaml.tmp");
    let yaml = serde_yaml::to_string(&config)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    tokio::fs::write(&tmp_path, &yaml).await?;
    tokio::fs::rename(&tmp_path, &path).await?;

    Ok(true)
}
