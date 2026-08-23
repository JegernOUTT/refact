use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Query, State};
use axum::Json;
use hyper::StatusCode;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use crate::app_state::AppState;
use crate::custom_error::ScratchError;
use crate::files_blocklist::IndexingSettings;

#[derive(Debug, Deserialize)]
pub struct SettingsScopeQuery {
    scope: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct IndexingSettingsConfig {
    #[serde(default)]
    blocklist: Vec<String>,
    #[serde(default)]
    additional_indexing_dirs: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct IndexingSettingsResponse {
    scope: String,
    path: String,
    config: IndexingSettingsConfig,
    project_available: bool,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum IndexingSettingsPost {
    Direct(IndexingSettingsConfig),
    Wrapped { config: IndexingSettingsConfig },
}

impl IndexingSettingsPost {
    fn into_config(self) -> IndexingSettingsConfig {
        match self {
            Self::Direct(config) | Self::Wrapped { config } => config,
        }
    }
}

#[derive(Serialize)]
struct PersistedIndexingSettings {
    blocklist: Vec<String>,
    additional_indexing_dirs: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillsAutoTriggerSetting {
    IndexOnly,
    InjectFull,
    Off,
}

impl Default for SkillsAutoTriggerSetting {
    fn default() -> Self {
        Self::IndexOnly
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SkillsSettings {
    #[serde(default)]
    auto_trigger: SkillsAutoTriggerSetting,
}

fn bad_request(message: impl Into<String>) -> ScratchError {
    ScratchError::new(StatusCode::BAD_REQUEST, message.into())
}

fn server_error(message: impl Into<String>) -> ScratchError {
    ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, message.into())
}

fn requested_scope(query: &SettingsScopeQuery) -> Result<&str, ScratchError> {
    match query.scope.as_deref() {
        Some("global") => Ok("global"),
        Some("project") => Ok("project"),
        Some(other) => Err(bad_request(format!(
            "invalid scope '{}'; expected 'global' or 'project'",
            other
        ))),
        None => Err(bad_request("missing scope; expected 'global' or 'project'")),
    }
}

async fn first_project_root(app: &AppState) -> Result<PathBuf, ScratchError> {
    crate::files_correction::get_project_dirs(app.gcx.clone())
        .await
        .into_iter()
        .next()
        .ok_or_else(|| bad_request("no project root is available"))
}

fn global_indexing_path(app: &AppState) -> PathBuf {
    let configured = app.gcx.cmdline.indexing_yaml.trim();
    if configured.is_empty() {
        app.gcx.config_dir.join("indexing.yaml")
    } else {
        crate::files_correction::canonical_path(configured)
    }
}

async fn indexing_path(
    app: &AppState,
    scope: &str,
) -> Result<(PathBuf, Option<PathBuf>, bool), ScratchError> {
    let project_root = crate::files_correction::get_project_dirs(app.gcx.clone())
        .await
        .into_iter()
        .next();
    if scope == "global" {
        Ok((global_indexing_path(app), None, project_root.is_some()))
    } else {
        let root = project_root.ok_or_else(|| bad_request("no project root is available"))?;
        Ok((root.join(".refact").join("indexing.yaml"), Some(root), true))
    }
}

async fn ensure_default_global(path: &Path, app: &AppState) -> Result<(), ScratchError> {
    if !app.gcx.cmdline.indexing_yaml.trim().is_empty()
        || tokio::fs::try_exists(path)
            .await
            .map_err(|e| server_error(format!("cannot inspect {}: {}", path.display(), e)))?
    {
        return Ok(());
    }
    atomic_write(
        path,
        include_str!("../../../yaml_configs/default_indexing.yaml"),
    )
    .await
}

async fn load_indexing_config(
    path: &Path,
    absent_is_empty: bool,
) -> Result<IndexingSettingsConfig, ScratchError> {
    let raw = match tokio::fs::read_to_string(path).await {
        Ok(raw) => raw,
        Err(e) if absent_is_empty && e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => {
            return Err(server_error(format!(
                "cannot read {}: {}",
                path.display(),
                e
            )))
        }
    };
    let settings = if raw.trim().is_empty() {
        IndexingSettings {
            blocklist: Vec::new(),
            additional_indexing_dirs: Vec::new(),
        }
    } else {
        serde_yaml::from_str::<IndexingSettings>(&raw).map_err(|e| {
            bad_request(format!(
                "invalid indexing YAML in {}: {}",
                path.display(),
                e
            ))
        })?
    };
    Ok(IndexingSettingsConfig {
        blocklist: settings.blocklist,
        additional_indexing_dirs: settings.additional_indexing_dirs,
    })
}

fn normalize_entries(entries: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    entries
        .into_iter()
        .map(|entry| entry.trim().to_string())
        .filter(|entry| !entry.is_empty() && seen.insert(entry.clone()))
        .collect()
}

fn validate_additional_dirs(scope: &str, entries: &[String]) -> Result<(), ScratchError> {
    for entry in entries {
        let uses_home_directory = entry == "~" || entry.starts_with("~/");
        if scope == "project" && uses_home_directory {
            return Err(bad_request(format!(
                "project additional indexing directory cannot use ~/ paths: {}",
                entry
            )));
        }
        let path = if uses_home_directory {
            let home = home::home_dir().ok_or_else(|| {
                server_error("cannot resolve home directory for indexing settings")
            })?;
            home.join(entry.trim_start_matches('~').trim_start_matches('/'))
        } else {
            PathBuf::from(entry)
        };
        if scope == "global" && !path.is_absolute() {
            return Err(bad_request(format!(
                "global additional indexing directory must be absolute or start with ~/: {}",
                entry
            )));
        }
        if path.is_absolute() && !path.is_dir() {
            return Err(bad_request(format!(
                "additional indexing directory is not an existing directory: {}",
                entry
            )));
        }
        if scope == "project"
            && !path.is_absolute()
            && path
                .components()
                .any(|component| matches!(component, Component::ParentDir | Component::RootDir))
        {
            return Err(bad_request(format!(
                "project-relative additional indexing directory cannot escape the project: {}",
                entry
            )));
        }
    }
    Ok(())
}

fn normalize_config(
    scope: &str,
    mut config: IndexingSettingsConfig,
) -> Result<IndexingSettingsConfig, ScratchError> {
    config.blocklist = normalize_entries(config.blocklist);
    config.additional_indexing_dirs = normalize_entries(config.additional_indexing_dirs);
    validate_additional_dirs(scope, &config.additional_indexing_dirs)?;
    Ok(config)
}

async fn atomic_write(path: &Path, content: &str) -> Result<(), ScratchError> {
    let parent = path
        .parent()
        .ok_or_else(|| server_error(format!("{} has no parent directory", path.display())))?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|e| server_error(format!("cannot create {}: {}", parent.display(), e)))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("settings.yaml");
    let tmp = parent.join(format!(
        ".{}.{}.{}.tmp",
        file_name,
        std::process::id(),
        nonce
    ));
    let result = async {
        let mut file = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp)
            .await?;
        file.write_all(content.as_bytes()).await?;
        file.flush().await?;
        file.sync_all().await?;
        tokio::fs::rename(&tmp, path).await
    }
    .await;
    if let Err(error) = result {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(server_error(format!(
            "cannot atomically write {}: {}",
            path.display(),
            error
        )));
    }
    Ok(())
}

fn indexing_response(
    scope: &str,
    path: &Path,
    config: IndexingSettingsConfig,
    project_available: bool,
) -> Json<IndexingSettingsResponse> {
    Json(IndexingSettingsResponse {
        scope: scope.to_string(),
        path: path.to_string_lossy().to_string(),
        config,
        project_available,
    })
}

pub async fn handle_v1_indexing_settings_get(
    State(app): State<AppState>,
    Query(query): Query<SettingsScopeQuery>,
) -> Result<Json<IndexingSettingsResponse>, ScratchError> {
    let scope = requested_scope(&query)?;
    let (path, _, project_available) = indexing_path(&app, scope).await?;
    if scope == "global" {
        ensure_default_global(&path, &app).await?;
    }
    let config = load_indexing_config(&path, scope == "project").await?;
    Ok(indexing_response(scope, &path, config, project_available))
}

pub async fn handle_v1_indexing_settings_post(
    State(app): State<AppState>,
    Query(query): Query<SettingsScopeQuery>,
    body: hyper::body::Bytes,
) -> Result<Json<IndexingSettingsResponse>, ScratchError> {
    let scope = requested_scope(&query)?;
    let post = serde_json::from_slice::<IndexingSettingsPost>(&body).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("invalid indexing settings payload: {}", e),
        )
    })?;
    let (path, _, project_available) = indexing_path(&app, scope).await?;
    let config = normalize_config(scope, post.into_config())?;
    let persisted = PersistedIndexingSettings {
        blocklist: config.blocklist.clone(),
        additional_indexing_dirs: config.additional_indexing_dirs.clone(),
    };
    let yaml = serde_yaml::to_string(&persisted)
        .map_err(|e| server_error(format!("cannot serialize indexing settings: {}", e)))?;
    atomic_write(&path, &yaml).await?;

    crate::files_blocklist::reload_indexing_everywhere_now(app.gcx.clone()).await;
    crate::files_in_workspace::enqueue_all_files_from_workspace_folders(
        app.gcx.clone(),
        true,
        false,
    )
    .await;
    Ok(indexing_response(scope, &path, config, project_available))
}

async fn skills_path(app: &AppState) -> Result<PathBuf, ScratchError> {
    Ok(first_project_root(app)
        .await?
        .join(".refact")
        .join("skills.yaml"))
}

async fn load_skills(path: &Path) -> Result<SkillsSettings, ScratchError> {
    match tokio::fs::read_to_string(path).await {
        Ok(raw) => serde_yaml::from_str(&raw)
            .map_err(|e| bad_request(format!("invalid skills YAML in {}: {}", path.display(), e))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(SkillsSettings::default()),
        Err(e) => Err(server_error(format!(
            "cannot read {}: {}",
            path.display(),
            e
        ))),
    }
}

pub async fn handle_v1_skills_settings_get(
    State(app): State<AppState>,
) -> Result<Json<SkillsSettings>, ScratchError> {
    let path = skills_path(&app).await?;
    Ok(Json(load_skills(&path).await?))
}

pub async fn handle_v1_skills_settings_post(
    State(app): State<AppState>,
    body: hyper::body::Bytes,
) -> Result<Json<SkillsSettings>, ScratchError> {
    let settings = serde_json::from_slice::<SkillsSettings>(&body).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("invalid skills settings payload: {}", e),
        )
    })?;
    let path = skills_path(&app).await?;
    let yaml = serde_yaml::to_string(&settings)
        .map_err(|e| server_error(format!("cannot serialize skills settings: {}", e)))?;
    atomic_write(&path, &yaml).await?;
    Ok(Json(settings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn global_and_project_indexing_load_save_and_absent_project() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let mut gcx = crate::global_context::tests::make_test_gcx().await;
        let mutable = Arc::get_mut(&mut gcx).expect("test owns context");
        mutable.config_dir = temp.path().join("config");
        *mutable.documents_state.workspace_folders.lock().unwrap() = vec![project.clone()];
        let app = AppState::from_gcx(gcx).await;

        let absent = handle_v1_indexing_settings_get(
            State(app.clone()),
            Query(SettingsScopeQuery {
                scope: Some("project".into()),
            }),
        )
        .await
        .unwrap()
        .0;
        assert!(absent.config.blocklist.is_empty());
        assert!(!project.join(".refact/indexing.yaml").exists());

        let body = serde_json::json!({"config": {
            "blocklist": [" target ", "target", ""],
            "additional_indexing_dirs": [" docs ", "docs"]
        }});
        let saved = handle_v1_indexing_settings_post(
            State(app.clone()),
            Query(SettingsScopeQuery {
                scope: Some("project".into()),
            }),
            hyper::body::Bytes::from(serde_json::to_vec(&body).unwrap()),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(saved.config.blocklist, vec!["target"]);
        assert_eq!(saved.config.additional_indexing_dirs, vec!["docs"]);
        let reloaded = load_indexing_config(&project.join(".refact/indexing.yaml"), false)
            .await
            .unwrap();
        assert_eq!(reloaded.blocklist, vec!["target"]);

        let global_body = serde_json::json!({
            "blocklist": [" generated ", "generated"],
            "additional_indexing_dirs": [temp.path().to_string_lossy()]
        });
        let global = handle_v1_indexing_settings_post(
            State(app.clone()),
            Query(SettingsScopeQuery {
                scope: Some("global".into()),
            }),
            hyper::body::Bytes::from(serde_json::to_vec(&global_body).unwrap()),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(global.config.blocklist, vec!["generated"]);
        assert!(Path::new(&global.path).exists());
        let loaded_global = handle_v1_indexing_settings_get(
            State(app),
            Query(SettingsScopeQuery {
                scope: Some("global".into()),
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(loaded_global.config.blocklist, vec!["generated"]);
    }

    #[tokio::test]
    async fn malformed_indexing_payload_and_unknown_skills_policy_are_rejected() {
        let malformed = serde_json::from_slice::<IndexingSettingsPost>(
            br#"{"config":{"blocklist":"bad","additional_indexing_dirs":[]}}"#,
        );
        assert!(malformed.is_err());
        let unknown = serde_json::from_slice::<SkillsSettings>(br#"{"auto_trigger":"sometimes"}"#);
        assert!(unknown.is_err());
    }

    #[test]
    fn project_scope_rejects_home_relative_indexing_directories() {
        let error = validate_additional_dirs("project", &["~/outside".to_string()]).unwrap_err();

        assert_eq!(error.status_code, StatusCode::BAD_REQUEST);
        assert!(error.message.contains("cannot use ~/"));
    }

    #[tokio::test]
    async fn skills_default_and_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let mut gcx = crate::global_context::tests::make_test_gcx().await;
        let mutable = Arc::get_mut(&mut gcx).expect("test owns context");
        *mutable.documents_state.workspace_folders.lock().unwrap() = vec![project.clone()];
        let app = AppState::from_gcx(gcx).await;

        let default = handle_v1_skills_settings_get(State(app.clone()))
            .await
            .unwrap()
            .0;
        assert_eq!(default.auto_trigger, SkillsAutoTriggerSetting::IndexOnly);
        let saved = handle_v1_skills_settings_post(
            State(app.clone()),
            hyper::body::Bytes::from_static(br#"{"auto_trigger":"inject_full"}"#),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(saved.auto_trigger, SkillsAutoTriggerSetting::InjectFull);
        let loaded = handle_v1_skills_settings_get(State(app)).await.unwrap().0;
        assert_eq!(loaded, saved);
    }

    #[tokio::test]
    async fn skills_without_project_root_is_bad_request() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = AppState::from_gcx(gcx).await;
        let error = handle_v1_skills_settings_get(State(app)).await.unwrap_err();
        assert_eq!(error.status_code, StatusCode::BAD_REQUEST);
        assert!(error.message.contains("no project root"));
    }
}
