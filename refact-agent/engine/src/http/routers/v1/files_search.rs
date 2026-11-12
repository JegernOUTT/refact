//! Local metadata-only search over the bounded inventory. Never consult editor buffers.
use std::cmp::Ordering;
use std::path::{Path, PathBuf};

use axum::extract::{Query, State};
use axum::Json;
use hyper::StatusCode;
use serde::{Deserialize, Serialize};

use super::files::{
    blocklisted_entry, hidden_or_heavy_directory, ResolvedPrivacyZone, TreePrivacyResolver,
};
use crate::app_state::AppState;
use crate::custom_error::ScratchError;
use crate::file_index::{FileIndex, FileIndexState};
use crate::files_blocklist::IndexingEverywhere;

#[derive(Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchSort {
    #[default]
    Name,
    Type,
    Size,
}
#[derive(Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchDirection {
    #[default]
    Asc,
    Desc,
}
#[derive(Default, Deserialize)]
pub struct SearchQuery {
    #[serde(default)]
    q: String,
    root: Option<String>,
    limit: Option<usize>,
    #[serde(default)]
    sort: SearchSort,
    #[serde(default)]
    direction: SearchDirection,
    #[serde(default)]
    show_ignored: bool,
}
#[derive(Serialize)]
pub struct SearchEntry {
    name: String,
    path: String,
    relative_path: String,
    root: String,
    kind: &'static str,
    size: Option<u64>,
    ignored: bool,
    privacy_zone: Option<ResolvedPrivacyZone>,
}
#[derive(Serialize)]
pub struct SearchResponse {
    entries: Vec<SearchEntry>,
    truncated: bool,
    index_state: FileIndexState,
    complete: bool,
}

fn visible_path(indexing: &IndexingEverywhere, root: &Path, path: &Path, is_dir: bool) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    if relative.as_os_str().is_empty() {
        return false;
    }
    let mut parent = root.to_path_buf();
    let count = relative.components().count();
    for (i, component) in relative.components().enumerate() {
        let std::path::Component::Normal(name) = component else {
            return false;
        };
        let directory = i + 1 < count || is_dir;
        let child = parent.join(name);
        if hidden_or_heavy_directory(&name.to_string_lossy(), directory)
            || blocklisted_entry(indexing, &parent, &child, directory)
        {
            return false;
        }
        parent = child;
    }
    true
}

fn compare_entries(a: &SearchEntry, b: &SearchEntry, query: &SearchQuery) -> Ordering {
    let folders = (b.kind == "dir").cmp(&(a.kind == "dir"));
    if folders != Ordering::Equal {
        return folders;
    }
    // Missing sizes stay last in either direction.
    if matches!(query.sort, SearchSort::Size) {
        let missing = a.size.is_none().cmp(&b.size.is_none());
        if missing != Ordering::Equal {
            return missing;
        }
    }
    let primary = match query.sort {
        SearchSort::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        SearchSort::Type => Path::new(&a.name)
            .extension()
            .unwrap_or_default()
            .to_ascii_lowercase()
            .cmp(
                &Path::new(&b.name)
                    .extension()
                    .unwrap_or_default()
                    .to_ascii_lowercase(),
            ),
        SearchSort::Size => a.size.cmp(&b.size),
    };
    let primary = if matches!(query.direction, SearchDirection::Desc) {
        primary.reverse()
    } else {
        primary
    };
    primary
        .then_with(|| a.root.cmp(&b.root))
        .then_with(|| a.relative_path.cmp(&b.relative_path))
}

fn search_inventory(
    index: &FileIndex,
    roots: &[PathBuf],
    indexing: &IndexingEverywhere,
    privacy: &TreePrivacyResolver,
    query: &SearchQuery,
) -> SearchResponse {
    let generation = index.generation();
    let status = index.status();
    let needle = query.q.to_lowercase();
    let limit = query.limit.unwrap_or(200).min(1000);
    let mut entries = Vec::with_capacity(limit + 1);
    let mut matches = 0usize;
    let mut uncertain = !status.complete || !index.covers_roots(roots);
    let repositories: Vec<_> = roots
        .iter()
        .map(|root| git2::Repository::discover(root).ok())
        .collect();
    for (path, facts) in index.snapshot() {
        // Inventory can contain previous roots, config or externally opened files. Scope
        // is always the CURRENT canonical project roots, not the index's historical scope.
        let Some((root_index, root)) = roots
            .iter()
            .enumerate()
            .filter(|(_, root)| path.starts_with(root))
            .max_by_key(|(_, root)| root.components().count())
        else {
            continue;
        };
        let is_dir = facts.is_none();
        let relative_path = path
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        if !relative_path.to_lowercase().contains(&needle) {
            continue;
        }
        if !visible_path(indexing, root, &path, is_dir) {
            continue;
        }
        let ignored = repositories[root_index]
            .as_ref()
            .and_then(|repository| {
                path.strip_prefix(repository.workdir()?)
                    .ok()
                    .and_then(|relative| repository.status_should_ignore(relative).ok())
            })
            .unwrap_or(false);
        if ignored && !query.show_ignored {
            continue;
        }
        let candidate = SearchEntry {
            name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            path: path.to_string_lossy().into_owned(),
            relative_path,
            root: root.to_string_lossy().into_owned(),
            kind: if is_dir { "dir" } else { "file" },
            size: facts.map(|facts| facts.size),
            ignored,
            privacy_zone: None,
        };
        matches += 1;
        let position = entries
            .binary_search_by(|entry| compare_entries(entry, &candidate, query))
            .unwrap_or_else(|position| position);
        if position <= limit {
            entries.insert(position, candidate);
            entries.truncate(limit + 1);
        }
    }
    let truncated = matches > limit;
    entries.truncate(limit);
    // Stat/canonical/privacy work is bounded to finalists, never every matching name.
    // A stale finalist is omitted and completeness withheld instead of requesting a walk.
    entries.retain_mut(|entry| {
        let path = Path::new(&entry.path);
        let Ok(metadata) = std::fs::symlink_metadata(path) else {
            uncertain = true;
            return false;
        };
        if metadata.is_dir() != (entry.kind == "dir") || (!metadata.is_dir() && !metadata.is_file())
        {
            uncertain = true;
            return false;
        }
        let Ok(canonical) = std::fs::canonicalize(path) else {
            uncertain = true;
            return false;
        };
        if dunce::simplified(&canonical) != path {
            uncertain = true;
            return false;
        }
        entry.privacy_zone = Some(privacy.zone_for_path(path));
        true
    });
    // A build or watcher update racing the snapshot cannot certify completeness.
    let complete =
        !uncertain && generation == index.generation() && index.state() == FileIndexState::Ready;
    SearchResponse {
        entries,
        truncated,
        index_state: index.state(),
        complete,
    }
}

pub async fn handle_v1_files_search(
    State(app): State<AppState>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<SearchResponse>, ScratchError> {
    if query.q.chars().count() > 256 {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "Search query exceeds 256 characters".into(),
        ));
    }
    let configured = crate::files_correction::get_unscoped_project_dirs(app.gcx.clone()).await;
    let mut roots = Vec::new();
    for root in configured {
        let root = tokio::fs::canonicalize(&root).await.map_err(|_| {
            ScratchError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "Project root unavailable".into(),
            )
        })?;
        roots.push(dunce::simplified(&root).to_path_buf());
    }
    roots.sort();
    roots.dedup();
    if let Some(requested) = &query.root {
        // Exact canonical membership: neither a subdirectory nor arbitrary worktree/config
        // path grants a broader scope. q is ONLY text and never interpreted as a path.
        let requested = tokio::fs::canonicalize(requested).await.map_err(|_| {
            ScratchError::new(
                StatusCode::FORBIDDEN,
                "Root is not a current project root".into(),
            )
        })?;
        let requested = dunce::simplified(&requested);
        if !roots.iter().any(|root| root == requested) {
            return Err(ScratchError::new(
                StatusCode::FORBIDDEN,
                "Root is not a current project root".into(),
            ));
        }
        roots.retain(|root| root == requested);
    }
    let privacy = TreePrivacyResolver::new(app.gcx.clone()).await?;
    let indexing =
        crate::files_blocklist::reload_indexing_everywhere_if_needed(app.gcx.clone()).await;
    let index = app.gcx.file_index.clone();
    crate::files_in_workspace::recover_file_index_watcher_drops(app.gcx.clone()).await;
    tokio::task::spawn_blocking(move || {
        Json(search_inventory(
            &index, &roots, &indexing, &privacy, &query,
        ))
    })
    .await
    .map_err(|_| {
        ScratchError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Search task failed".into(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_index::build_blocking;
    use std::sync::atomic::AtomicBool;

    #[tokio::test]
    async fn search_filters_current_scope_and_hidden_ancestors_without_reading_contents() {
        let workspace =
            tempfile::tempdir_in(dunce::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
        let outside =
            tempfile::tempdir_in(dunce::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
        std::fs::create_dir_all(workspace.path().join("target/nested")).unwrap();
        std::fs::create_dir(workspace.path().join("empty")).unwrap();
        std::fs::write(workspace.path().join("main.rs"), "secret-content").unwrap();
        std::fs::write(workspace.path().join("target/nested/leak.rs"), "excluded").unwrap();
        std::fs::write(outside.path().join("foreign.rs"), "foreign").unwrap();
        let index = FileIndex::default();
        build_blocking(
            &index,
            &[workspace.path().into(), outside.path().into()],
            &AtomicBool::new(false),
        );
        assert!(index.get(&outside.path().join("foreign.rs")).is_some());
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.privacy_settings.write().unwrap() =
            std::sync::Arc::new(crate::privacy::PrivacySettings {
                privacy_rules: crate::privacy::FilePrivacySettings {
                    blocked: Vec::new(),
                    only_send_to_servers_I_control: Vec::new(),
                },
                loaded_ts: u64::MAX / 2,
            });
        *gcx.privacy_policy_load.write().unwrap() = refact_privacy::PolicyLoad::default();
        let privacy = TreePrivacyResolver::new(gcx).await.unwrap();
        let response = search_inventory(
            &index,
            &[workspace.path().into()],
            &IndexingEverywhere::default(),
            &privacy,
            &SearchQuery::default(),
        );
        let names: Vec<_> = response
            .entries
            .iter()
            .map(|entry| entry.relative_path.as_str())
            .collect();
        assert_eq!(names, ["empty", "main.rs"]);
        assert!(response.complete);
        let value = serde_json::to_string(&response).unwrap();
        assert!(!value.contains("secret-content"));
    }

    #[tokio::test]
    async fn search_rejects_explicit_nonmember_root_in_actual_route() {
        use tower::ServiceExt;
        let workspace = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("private.txt"), "exists").unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![workspace.path().into()];
        *gcx.privacy_settings.write().unwrap() =
            std::sync::Arc::new(crate::privacy::PrivacySettings {
                privacy_rules: crate::privacy::FilePrivacySettings {
                    blocked: Vec::new(),
                    only_send_to_servers_I_control: Vec::new(),
                },
                loaded_ts: u64::MAX / 2,
            });
        *gcx.privacy_policy_load.write().unwrap() = refact_privacy::PolicyLoad::default();
        let app = AppState::from_gcx(gcx).await;
        let router = crate::http::routers::make_refact_http_server(app);
        let positive = router
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri(format!(
                        "/v1/files/search?{}",
                        url::form_urlencoded::Serializer::new(String::new())
                            .append_pair("root", &workspace.path().to_string_lossy())
                            .finish()
                    ))
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let positive_status = positive.status();
        let positive_body = hyper::body::to_bytes(positive.into_body()).await.unwrap();
        assert_eq!(
            positive_status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&positive_body)
        );
        let uri = format!(
            "/v1/files/search?{}",
            url::form_urlencoded::Serializer::new(String::new())
                .append_pair("root", &outside.path().to_string_lossy())
                .finish()
        );
        let response = router
            .oneshot(
                axum::http::Request::builder()
                    .uri(uri)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = hyper::body::to_bytes(response.into_body()).await.unwrap();
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{}",
            String::from_utf8_lossy(&body)
        );
        assert!(!String::from_utf8_lossy(&body).contains("private.txt"));
    }

    #[tokio::test]
    async fn search_orders_by_size_before_truncating_case_insensitive_relative_matches() {
        let workspace =
            tempfile::tempdir_in(dunce::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
        std::fs::create_dir(workspace.path().join("Nested")).unwrap();
        std::fs::write(workspace.path().join("Nested/z.txt"), "small").unwrap();
        std::fs::write(workspace.path().join("Nested/a.rs"), "larger bytes").unwrap();
        let index = FileIndex::default();
        build_blocking(&index, &[workspace.path().into()], &AtomicBool::new(false));
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.privacy_settings.write().unwrap() =
            std::sync::Arc::new(crate::privacy::PrivacySettings {
                privacy_rules: crate::privacy::FilePrivacySettings {
                    blocked: Vec::new(),
                    only_send_to_servers_I_control: Vec::new(),
                },
                loaded_ts: u64::MAX / 2,
            });
        *gcx.privacy_policy_load.write().unwrap() = refact_privacy::PolicyLoad::default();
        let privacy = TreePrivacyResolver::new(gcx).await.unwrap();
        let response = search_inventory(
            &index,
            &[workspace.path().into()],
            &IndexingEverywhere::default(),
            &privacy,
            &SearchQuery {
                q: "nEsTeD/".into(),
                sort: SearchSort::Size,
                direction: SearchDirection::Desc,
                limit: Some(1),
                ..Default::default()
            },
        );
        assert_eq!(response.entries.len(), 1);
        assert_eq!(response.entries[0].relative_path, "Nested/a.rs");
        assert_eq!(response.entries[0].size, Some(12));
        assert!(response.truncated);
    }
}
