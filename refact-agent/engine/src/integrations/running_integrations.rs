use std::sync::Arc;
use indexmap::IndexMap;

use crate::custom_error::YamlError;
use crate::global_context::GlobalContext;
use crate::integrations::integr_abstract::IntegrationTrait;

/// Loads and set up integrations from config files.
///
/// If `include_paths_matching` is `None`, all integrations are loaded,
/// otherwise only those matching `include_paths_matching` glob patterns.
pub async fn load_integrations(
    gcx: Arc<GlobalContext>,
    include_paths_matching: &[String],
) -> (
    IndexMap<String, Box<dyn IntegrationTrait + Send + Sync>>,
    Vec<YamlError>,
) {
    let active_project_path = crate::files_correction::get_active_project_path(gcx.clone()).await;
    let (config_dirs, global_config_dir) =
        crate::integrations::setting_up_integrations::get_config_dirs(
            gcx.clone(),
            &active_project_path,
        )
        .await;
    let (integrations_yaml_path, is_inside_container, allow_experimental) = {
        (
            gcx.cmdline.integrations_yaml.clone(),
            gcx.cmdline.inside_container,
            gcx.cmdline.experimental,
        )
    };

    let mut error_log: Vec<YamlError> = Vec::new();
    let lst: Vec<&str> = crate::integrations::integrations_list(allow_experimental);
    let vars_for_replacements =
        crate::integrations::setting_up_integrations::get_vars_for_replacements(
            gcx.clone(),
            &mut error_log,
        )
        .await;
    let records = crate::integrations::setting_up_integrations::read_integrations_d(
        &config_dirs,
        &global_config_dir,
        &integrations_yaml_path,
        &vars_for_replacements,
        &lst,
        &mut error_log,
        include_paths_matching,
        false,
    );

    let mut integrations_map = IndexMap::new();
    if let Some(token) = reconcile_needed(&gcx).await {
        reconcile_mcp_sessions(gcx.clone(), &config_dirs, &global_config_dir).await;
        commit_reconcile(token);
    }
    for rec in records {
        if !is_inside_container && !rec.on_your_laptop {
            continue;
        }
        if is_inside_container && !rec.when_isolated {
            continue;
        }
        if !rec.integr_config_exists {
            continue;
        }
        let mut integr = match crate::integrations::integration_from_name(&rec.integr_name) {
            Ok(x) => x,
            Err(e) => {
                tracing::error!("don't have integration {}: {}", rec.integr_name, e);
                continue;
            }
        };
        let should_be_fine = integr
            .integr_settings_apply(
                gcx.clone(),
                rec.integr_config_path.clone(),
                &rec.config_unparsed,
            )
            .await;
        if let Err(err) = should_be_fine {
            error_log.push(YamlError {
                path: rec.integr_config_path.clone(),
                error_line: err.line(),
                error_msg: format!("failed to apply settings: {}", err),
            });
        }
        integrations_map.insert(rec.integr_name.clone(), integr);
    }
    for e in error_log.iter() {
        tracing::error!("{e}");
    }
    (integrations_map, error_log)
}

type ReconcileEntry = (std::sync::Weak<GlobalContext>, (u64, usize));

static LAST_RECONCILE: std::sync::Mutex<Option<std::collections::HashMap<usize, ReconcileEntry>>> =
    std::sync::Mutex::new(None);

/// Token identifying the state a reconcile pass ran against; recorded only after the pass
/// completes, so a cancelled `load_integrations` does not suppress the next reconcile.
struct ReconcileToken {
    key: usize,
    owner: std::sync::Weak<GlobalContext>,
    state: (u64, usize),
}

fn commit_reconcile(token: ReconcileToken) {
    let mut last = match LAST_RECONCILE.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let last = last.get_or_insert_with(std::collections::HashMap::new);
    last.retain(|_, (owner, _)| owner.strong_count() > 0);
    if last.len() >= 64 {
        last.clear();
    }
    last.insert(token.key, (token.owner, token.state));
}

/// Reconcile only when the integrations generation or the session count moved since the
/// last completed pass for this context; an empty session map never needs it.
async fn reconcile_needed(gcx: &Arc<GlobalContext>) -> Option<ReconcileToken> {
    let sessions = gcx.integration_sessions.lock().await.len();
    if sessions == 0 {
        return None;
    }
    let generation = gcx
        .tool_catalog_generations
        .integrations
        .load(std::sync::atomic::Ordering::SeqCst);
    let last = match LAST_RECONCILE.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let key = Arc::as_ptr(gcx) as usize;
    let state = (generation, sessions);
    let unchanged = last
        .as_ref()
        .and_then(|last| last.get(&key))
        .is_some_and(|(owner, recorded)| owner.strong_count() > 0 && *recorded == state);
    if unchanged {
        return None;
    }
    Some(ReconcileToken {
        key,
        owner: Arc::downgrade(gcx),
        state,
    })
}

async fn reconcile_mcp_sessions(
    gcx: Arc<GlobalContext>,
    config_dirs: &[std::path::PathBuf],
    global_config_dir: &std::path::Path,
) {
    let sessions: Vec<_> = gcx
        .integration_sessions
        .lock()
        .await
        .values()
        .cloned()
        .collect();
    for session in sessions {
        let path = {
            let mut locked = session.lock().await;
            locked
                .as_any_mut()
                .downcast_mut::<crate::integrations::mcp::session_mcp::SessionMCP>()
                .filter(|session| session.launched_cfg.get("operator_tools").is_none())
                .map(|session| session.config_path.clone())
        };
        if let Some(path) = path {
            let config = std::path::Path::new(&path);
            let is_config = config.extension().is_some_and(|ext| ext == "yaml")
                && config
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .is_some_and(|stem| stem.starts_with("mcp_"))
                && config.parent().is_some_and(|parent| {
                    config_dirs
                        .iter()
                        .any(|dir| parent == dir.join("integrations.d"))
                        || parent == global_config_dir.join("integrations.d")
                });
            if is_config
                && matches!(tokio::fs::metadata(config).await,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound)
            {
                crate::integrations::mcp::integr_mcp_common::remove_mcp_session_if_config_missing(
                    gcx.clone(),
                    &path,
                )
                .await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn reconcile_removes_missing_native_config_but_preserves_restricted_session() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let dir = tempfile::tempdir().unwrap();
        let native = dir.path().join("integrations.d/mcp_http_native.yaml");
        let restricted = dir.path().join("integrations.d/mcp_http_restricted.yaml");
        for (path, cfg) in [
            (&native, serde_json::json!({})),
            (&restricted, serde_json::json!({"operator_tools":["read"]})),
        ] {
            let path = path.to_string_lossy().into_owned();
            let mut session = crate::integrations::mcp::session_mcp::SessionMCP::detached(&path);
            session.launched_cfg = cfg;
            gcx.integration_sessions
                .lock()
                .await
                .insert(path, Arc::new(tokio::sync::Mutex::new(Box::new(session))));
        }
        assert_eq!(gcx.integration_sessions.lock().await.len(), 2);
        reconcile_mcp_sessions(gcx.clone(), &[dir.path().to_path_buf()], dir.path()).await;
        let sessions = gcx.integration_sessions.lock().await;
        assert!(!sessions.contains_key(native.to_str().unwrap()));
        assert!(sessions.contains_key(restricted.to_str().unwrap()));
    }

    #[tokio::test]
    async fn reconcile_gate_skips_empty_and_unchanged_state() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        assert!(reconcile_needed(&gcx).await.is_none());
        let path = "/tmp/mcp_http_reconcile_gate_fixture.yaml";
        let session = crate::integrations::mcp::session_mcp::SessionMCP::detached(path);
        gcx.integration_sessions.lock().await.insert(
            path.into(),
            Arc::new(tokio::sync::Mutex::new(Box::new(session))),
        );
        commit_reconcile(reconcile_needed(&gcx).await.unwrap());
        assert!(reconcile_needed(&gcx).await.is_none());
        gcx.tool_catalog_generations.advance_integrations();
        commit_reconcile(reconcile_needed(&gcx).await.unwrap());
        assert!(reconcile_needed(&gcx).await.is_none());
        gcx.integration_sessions.lock().await.clear();
    }

    #[tokio::test]
    async fn reconcile_gate_reruns_after_cancelled_pass() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let path = "/tmp/mcp_http_reconcile_cancel_fixture.yaml";
        let session = crate::integrations::mcp::session_mcp::SessionMCP::detached(path);
        gcx.integration_sessions.lock().await.insert(
            path.into(),
            Arc::new(tokio::sync::Mutex::new(Box::new(session))),
        );
        let (obtained_tx, obtained_rx) = tokio::sync::oneshot::channel();
        let cancelled = tokio::spawn({
            let gcx = gcx.clone();
            async move {
                let token = reconcile_needed(&gcx).await;
                let _ = obtained_tx.send(token.is_some());
                if let Some(token) = token {
                    std::future::pending::<()>().await;
                    commit_reconcile(token);
                }
            }
        });
        assert!(
            obtained_rx.await.unwrap(),
            "the first pass must need a reconcile"
        );
        cancelled.abort();
        let _ = cancelled.await;
        assert!(reconcile_needed(&gcx).await.is_some());
        gcx.integration_sessions.lock().await.clear();
    }

    #[tokio::test]
    async fn reconcile_gate_ignores_entries_from_dropped_contexts() {
        let dead_owner = std::sync::Weak::<GlobalContext>::new();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let path = "/tmp/mcp_http_reconcile_dropped_fixture.yaml";
        let session = crate::integrations::mcp::session_mcp::SessionMCP::detached(path);
        gcx.integration_sessions.lock().await.insert(
            path.into(),
            Arc::new(tokio::sync::Mutex::new(Box::new(session))),
        );
        let token = reconcile_needed(&gcx).await.unwrap();
        {
            let mut last = LAST_RECONCILE
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            last.get_or_insert_with(std::collections::HashMap::new)
                .insert(token.key, (dead_owner, token.state));
        }
        assert!(reconcile_needed(&gcx).await.is_some());
        gcx.integration_sessions.lock().await.clear();
    }
}
