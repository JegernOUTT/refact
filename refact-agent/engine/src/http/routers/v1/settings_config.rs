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

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum TrajectorySettingsPost {
    Direct(crate::runtime_settings::TrajectoryRuntimeSettings),
    Wrapped {
        config: crate::runtime_settings::TrajectoryRuntimeSettings,
    },
}

impl TrajectorySettingsPost {
    fn into_config(self) -> crate::runtime_settings::TrajectoryRuntimeSettings {
        match self {
            Self::Direct(config) | Self::Wrapped { config } => config,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct TrajectorySettingsResponse {
    path: String,
    config: crate::runtime_settings::TrajectoryRuntimeSettings,
    current: crate::runtime_settings::TrajectoryRuntimeSettings,
    defaults: crate::runtime_settings::TrajectoryRuntimeSettings,
    fields: Vec<TrajectorySettingField>,
    environment_precedence: &'static str,
}

#[derive(Debug, Serialize)]
struct TrajectorySettingField {
    name: &'static str,
    value_type: &'static str,
    minimum: Option<u64>,
    maximum: Option<u64>,
    apply_mode: &'static str,
}

fn trajectory_settings_fields() -> Vec<TrajectorySettingField> {
    let live_usize = |name, minimum, maximum| TrajectorySettingField {
        name,
        value_type: "integer",
        minimum: Some(minimum),
        maximum: Some(maximum),
        apply_mode: "live",
    };
    let restart_usize = |name, minimum, maximum| TrajectorySettingField {
        name,
        value_type: "integer",
        minimum: Some(minimum),
        maximum: Some(maximum),
        apply_mode: "restart_required",
    };
    let restart_bool = |name| TrajectorySettingField {
        name,
        value_type: "boolean",
        minimum: None,
        maximum: None,
        apply_mode: "restart_required",
    };
    let live_bool = |name| TrajectorySettingField {
        name,
        value_type: "boolean",
        minimum: None,
        maximum: None,
        apply_mode: "live",
    };
    vec![
        live_usize("internal_traces_keep_per_folder", 10, 10_000),
        live_usize("internal_trace_prune_interval_secs", 60, 86_400),
        live_usize("buddy_conversations_keep", 10, 100_000),
        live_usize("buddy_conversations_prune_interval_secs", 60, 86_400),
        live_usize("buddy_conversations_prune_min_age_secs", 60, 365 * 86_400),
        live_usize("session_idle_timeout_secs", 60, 86_400),
        live_usize("session_cleanup_interval_secs", 10, 86_400),
        live_usize("stream_idle_timeout_secs", 10, 86_400),
        live_usize("stream_total_timeout_secs", 60, 172_800),
        live_usize("max_queue_size", 1, 10_000),
        restart_usize("event_channel_capacity", 16, 1_000_000),
        live_usize("recent_request_ids_capacity", 1, 100_000),
        live_usize("max_parallel_tools", 1, 10_000),
        live_usize("max_images_per_message", 1, 1_000),
        live_usize("max_file_size", 1_024, 50_000_000),
        live_usize("auto_enrichment_total_token_cap", 64, 32_000),
        live_usize("auto_enrichment_card_token_cap", 32, 16_000),
        live_usize("auto_enrichment_knowledge_top_n", 1, 20),
        live_usize("auto_enrichment_trajectory_top_n", 1, 20),
        restart_bool("trajectory_writer_enabled"),
        restart_bool("trajectory_index_coordinator_enabled"),
        live_bool("trajectory_watcher_self_write_enabled"),
        restart_bool("tool_catalog_snapshots_enabled"),
        restart_bool("vecdb_path_coalescing_enabled"),
        live_usize("pp_max_tool_budget_tokens", 4_096, 2_000_000),
        live_usize("pp_max_per_file_budget_tokens", 1_024, 2_000_000),
        live_usize("pp_max_line_length_chars", 80, 1_000_000),
        live_usize("pp_tokens_for_text_percent", 1, 100),
        live_usize("git_intel_max_commits", 100, 1_000_000),
        live_usize("git_intel_deep_walk_limit", 100, 5_000_000),
        live_usize("git_intel_max_files_per_commit_cochange", 10, 100_000),
        live_usize("git_intel_max_files_per_commit_entropy", 5, 100_000),
        live_usize("codegraph_dead_code_max_results", 50, 1_000_000),
        live_usize("codegraph_exec_flow_max_nodes", 50, 1_000_000),
        live_usize("vecdb_trajectory_split_bytes", 256, 1_048_576),
        live_usize("cat_max_input_paths", 1, 100_000),
        live_usize("cat_max_lines", 100, 1_000_000),
        live_usize("cat_max_file_bytes", 4_096, 268_435_456),
        live_usize("cat_max_expanded_files", 1, 100_000),
        live_usize("get_logs_max_tail_bytes", 4_096, 268_435_456),
        live_usize("planner_qna_question_limit", 200, 1_000_000),
        live_usize("planner_qna_answer_limit", 200, 1_000_000),
        live_usize("hist_search_preview_chars", 100, 100_000),
        live_usize("agent_diff_max_output_bytes", 4_096, 268_435_456),
        live_usize("process_subscribe_preview_bytes", 50, 100_000),
        live_usize("review_diff_char_cap", 4_096, 8_388_608),
        live_usize("review_max_diff_patch_bytes", 4_096, 268_435_456),
    ]
}

fn trajectory_settings_response(
    path: PathBuf,
    config: crate::runtime_settings::TrajectoryRuntimeSettings,
) -> Json<TrajectorySettingsResponse> {
    Json(TrajectorySettingsResponse {
        path: path.to_string_lossy().to_string(),
        config,
        current: crate::runtime_settings::current(),
        defaults: crate::runtime_settings::TrajectoryRuntimeSettings::default(),
        fields: trajectory_settings_fields(),
        environment_precedence: "REFACT_TRAJECTORY_WRITER, REFACT_TRAJECTORY_INDEX_COORDINATOR, REFACT_TRAJECTORY_WATCHER_SELF_WRITE, REFACT_TOOL_CATALOG_SNAPSHOTS, and REFACT_VECDB_PATH_COALESCING take precedence over persisted rollout switches. Persisted writer, index coordinator, tool catalog, and VecDB changes apply after restart; watcher self-write suppression applies live.",
    })
}

pub async fn handle_v1_trajectory_settings_get(
    State(app): State<AppState>,
) -> Result<Json<TrajectorySettingsResponse>, ScratchError> {
    let path = crate::runtime_settings::settings_path(&app.paths.config_dir);
    let config = crate::runtime_settings::load_from_path(&path)
        .await
        .map_err(server_error)?;
    Ok(trajectory_settings_response(path, config))
}

pub async fn handle_v1_trajectory_settings_post(
    State(app): State<AppState>,
    body: hyper::body::Bytes,
) -> Result<Json<TrajectorySettingsResponse>, ScratchError> {
    let settings = serde_json::from_slice::<TrajectorySettingsPost>(&body)
        .map_err(|error| {
            ScratchError::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("invalid trajectory settings payload: {error}"),
            )
        })?
        .into_config();
    crate::runtime_settings::validate(&settings).map_err(bad_request)?;
    let path = crate::runtime_settings::settings_path(&app.paths.config_dir);
    let yaml = serde_yaml::to_string(&settings)
        .map_err(|error| server_error(format!("cannot serialize trajectory settings: {error}")))?;
    atomic_write(&path, &yaml).await?;
    crate::runtime_settings::install_live(&settings);
    Ok(trajectory_settings_response(path, settings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::sync::Arc;

    async fn trajectory_settings_app() -> AppState {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        AppState::from_gcx(gcx).await
    }

    /// The field metadata table and `runtime_settings::validate` are two
    /// hand-maintained parallel lists. If they drift, the GUI either blocks
    /// values the engine accepts or offers values the engine rejects with a
    /// 400. This test is the only thing keeping them in agreement.
    #[test]
    fn trajectory_settings_fields_match_struct_and_validate_bounds() {
        let defaults = crate::runtime_settings::TrajectoryRuntimeSettings::default();
        let defaults_json = serde_json::to_value(&defaults).unwrap();
        let serialized_names: std::collections::BTreeSet<String> = defaults_json
            .as_object()
            .expect("settings serialize as a JSON object")
            .keys()
            .cloned()
            .collect();

        let fields = trajectory_settings_fields();
        let field_names: std::collections::BTreeSet<String> =
            fields.iter().map(|field| field.name.to_string()).collect();
        assert_eq!(
            field_names.len(),
            fields.len(),
            "trajectory_settings_fields() contains duplicate names"
        );
        assert_eq!(
            field_names, serialized_names,
            "trajectory_settings_fields() must list exactly the serialized settings fields"
        );

        // Every advertised min/max must be precisely what validate() enforces.
        for field in &fields {
            let (minimum, maximum) = match (field.minimum, field.maximum) {
                (Some(minimum), Some(maximum)) => (minimum, maximum),
                _ => {
                    assert_eq!(
                        field.value_type, "boolean",
                        "{} has no bounds but is not a boolean",
                        field.name
                    );
                    continue;
                }
            };
            assert_eq!(field.value_type, "integer");
            assert!(minimum < maximum, "{} has an empty range", field.name);

            let with_value = |value: u64| {
                let mut json = defaults_json.clone();
                json[field.name] = serde_json::json!(value);
                let settings: crate::runtime_settings::TrajectoryRuntimeSettings =
                    serde_json::from_value(json).unwrap_or_else(|e| {
                        panic!("{} is not settable to {value}: {e}", field.name)
                    });
                crate::runtime_settings::validate(&settings)
            };
            let range_error = format!("{} must be between {} and {}", field.name, minimum, maximum);

            // Boundaries are accepted (a cross-field rule may still complain,
            // but never about this field's range).
            for accepted in [minimum, maximum] {
                if let Err(error) = with_value(accepted) {
                    assert!(
                        !error.starts_with(&range_error),
                        "{} rejects in-range value {accepted}: {error}",
                        field.name
                    );
                }
            }
            // Just outside the boundaries is rejected with exactly these bounds.
            for rejected in [minimum - 1, maximum + 1] {
                let error = with_value(rejected).unwrap_err();
                assert_eq!(
                    error,
                    format!("{range_error}; got {rejected}"),
                    "{} bounds disagree between fields() and validate()",
                    field.name
                );
            }
        }
    }

    #[tokio::test]
    #[serial(runtime_settings)]
    async fn trajectory_settings_defaults_round_trip_and_persist() {
        crate::runtime_settings::reset_for_test();
        let app = trajectory_settings_app().await;
        let initial = handle_v1_trajectory_settings_get(State(app.clone()))
            .await
            .unwrap()
            .0;
        assert_eq!(
            initial.config,
            crate::runtime_settings::TrajectoryRuntimeSettings::default()
        );
        assert_eq!(initial.fields.len(), 47);
        assert!(initial
            .fields
            .iter()
            .any(|field| field.name == "event_channel_capacity"
                && field.apply_mode == "restart_required"));
        for name in [
            "trajectory_writer_enabled",
            "trajectory_index_coordinator_enabled",
            "trajectory_watcher_self_write_enabled",
            "tool_catalog_snapshots_enabled",
            "vecdb_path_coalescing_enabled",
        ] {
            assert!(initial
                .fields
                .iter()
                .any(|field| field.name == name && field.value_type == "boolean"));
        }
        assert!(initial.fields.iter().any(|field| field.name
            == "trajectory_watcher_self_write_enabled"
            && field.apply_mode == "live"));
        assert!(initial.defaults.trajectory_writer_enabled);
        assert!(initial.defaults.trajectory_index_coordinator_enabled);
        assert!(initial.defaults.trajectory_watcher_self_write_enabled);
        assert!(initial.defaults.tool_catalog_snapshots_enabled);
        assert!(initial.defaults.vecdb_path_coalescing_enabled);

        let mut updated = initial.config.clone();
        updated.internal_traces_keep_per_folder = 25;
        updated.session_idle_timeout_secs = 120;
        updated.auto_enrichment_total_token_cap = 640;
        updated.auto_enrichment_card_token_cap = 320;
        let saved = handle_v1_trajectory_settings_post(
            State(app.clone()),
            hyper::body::Bytes::from(serde_json::to_vec(&updated).unwrap()),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(saved.config, updated);
        assert_eq!(
            crate::runtime_settings::current().session_idle_timeout_secs,
            120
        );
        let reloaded = handle_v1_trajectory_settings_get(State(app))
            .await
            .unwrap()
            .0;
        assert_eq!(reloaded.config, updated);
    }

    #[tokio::test]
    #[serial(runtime_settings)]
    async fn trajectory_settings_validation_preserves_previous_configuration() {
        crate::runtime_settings::reset_for_test();
        let app = trajectory_settings_app().await;
        let mut valid = crate::runtime_settings::TrajectoryRuntimeSettings::default();
        valid.max_queue_size = 7;
        let _ = handle_v1_trajectory_settings_post(
            State(app.clone()),
            hyper::body::Bytes::from(serde_json::to_vec(&valid).unwrap()),
        )
        .await
        .unwrap();
        let mut invalid = valid.clone();
        invalid.max_queue_size = 0;
        let error = handle_v1_trajectory_settings_post(
            State(app.clone()),
            hyper::body::Bytes::from(serde_json::to_vec(&invalid).unwrap()),
        )
        .await
        .unwrap_err();
        assert_eq!(error.status_code, StatusCode::BAD_REQUEST);
        assert!(error.message.contains("max_queue_size"));
        assert_eq!(
            handle_v1_trajectory_settings_get(State(app))
                .await
                .unwrap()
                .0
                .config,
            valid
        );
    }

    #[tokio::test]
    #[serial(runtime_settings)]
    async fn trajectory_settings_live_values_apply_and_restart_values_wait() {
        crate::runtime_settings::reset_for_test();
        let app = trajectory_settings_app().await;
        let initial_capacity = refact_chat_history::config::limits().event_channel_capacity;
        let mut updated = crate::runtime_settings::TrajectoryRuntimeSettings::default();
        updated.max_queue_size = 9;
        updated.event_channel_capacity = initial_capacity + 100;
        updated.auto_enrichment_total_token_cap = 800;
        updated.auto_enrichment_card_token_cap = 400;
        updated.trajectory_writer_enabled = false;
        updated.trajectory_index_coordinator_enabled = false;
        updated.trajectory_watcher_self_write_enabled = false;
        updated.tool_catalog_snapshots_enabled = false;
        updated.vecdb_path_coalescing_enabled = false;
        let saved = handle_v1_trajectory_settings_post(
            State(app.clone()),
            hyper::body::Bytes::from(serde_json::to_vec(&updated).unwrap()),
        )
        .await
        .unwrap();
        assert_eq!(saved.0.config, updated);
        assert_eq!(refact_chat_api::max_queue_size(), 9);
        assert_eq!(
            refact_chat_history::config::limits().event_channel_capacity,
            initial_capacity
        );
        assert_eq!(
            crate::runtime_settings::current().auto_enrichment_total_token_cap,
            800
        );
        assert!(crate::runtime_settings::current().trajectory_writer_enabled);
        assert!(crate::runtime_settings::current().trajectory_index_coordinator_enabled);
        assert!(!crate::runtime_settings::current().trajectory_watcher_self_write_enabled);
        assert!(crate::runtime_settings::current().tool_catalog_snapshots_enabled);
        assert!(crate::runtime_settings::current().vecdb_path_coalescing_enabled);

        let mut mixed = updated.clone();
        mixed.trajectory_writer_enabled = true;
        mixed.trajectory_index_coordinator_enabled = false;
        mixed.trajectory_watcher_self_write_enabled = true;
        mixed.tool_catalog_snapshots_enabled = false;
        mixed.vecdb_path_coalescing_enabled = true;
        let mixed_saved = handle_v1_trajectory_settings_post(
            State(app.clone()),
            hyper::body::Bytes::from(serde_json::to_vec(&mixed).unwrap()),
        )
        .await
        .unwrap();
        assert_eq!(mixed_saved.0.config, mixed);
        assert!(crate::runtime_settings::current().trajectory_writer_enabled);
        assert!(crate::runtime_settings::current().trajectory_index_coordinator_enabled);
        assert!(crate::runtime_settings::current().trajectory_watcher_self_write_enabled);
        assert!(crate::runtime_settings::current().tool_catalog_snapshots_enabled);
        assert!(crate::runtime_settings::current().vecdb_path_coalescing_enabled);
        assert_eq!(
            handle_v1_trajectory_settings_get(State(app))
                .await
                .unwrap()
                .0
                .config,
            mixed
        );
    }

    #[test]
    #[serial(runtime_settings)]
    fn trajectory_settings_environment_rollout_override_wins() {
        let cases = [
            (
                crate::chat::trajectories::TRAJECTORY_WRITER_ENV,
                crate::chat::trajectories::trajectory_writer_rollout_enabled_for
                    as fn(Option<&str>) -> bool,
            ),
            (
                crate::chat::trajectory_index::TRAJECTORY_INDEX_COORDINATOR_ENV,
                crate::chat::trajectory_index::trajectory_index_coordinator_rollout_enabled_for,
            ),
            (
                crate::chat::trajectories::TRAJECTORY_WATCHER_SELF_WRITE_ENV,
                crate::chat::trajectories::trajectory_watcher_self_write_rollout_enabled_for,
            ),
            (
                crate::app_state::TOOL_CATALOG_SNAPSHOTS_ENV,
                crate::app_state::tool_catalog_snapshot_rollout_enabled_for,
            ),
        ];
        for (key, enabled) in cases {
            let previous = std::env::var_os(key);
            std::env::set_var(key, "0");
            assert!(!enabled(std::env::var(key).ok().as_deref()));
            std::env::set_var(key, "on");
            assert!(enabled(std::env::var(key).ok().as_deref()));
            if let Some(previous) = previous {
                std::env::set_var(key, previous);
            } else {
                std::env::remove_var(key);
            }
        }
        let previous = std::env::var_os(refact_vecdb::vdb_thread::VECDB_PATH_COALESCING_ENV);
        std::env::set_var(refact_vecdb::vdb_thread::VECDB_PATH_COALESCING_ENV, "0");
        assert!(!refact_vecdb::vdb_thread::vecdb_path_coalescing_rollout_enabled());
        std::env::set_var(refact_vecdb::vdb_thread::VECDB_PATH_COALESCING_ENV, "on");
        assert!(refact_vecdb::vdb_thread::vecdb_path_coalescing_rollout_enabled());
        if let Some(previous) = previous {
            std::env::set_var(
                refact_vecdb::vdb_thread::VECDB_PATH_COALESCING_ENV,
                previous,
            );
        } else {
            std::env::remove_var(refact_vecdb::vdb_thread::VECDB_PATH_COALESCING_ENV);
        }
    }

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
