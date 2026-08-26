use std::sync::{Arc, Mutex, OnceLock};
use std::path::{Path, PathBuf};
use tokio::time::Duration;
use tokio::fs;
use std::time::SystemTime;
use std::collections::HashMap;
use crate::files_correction::{canonical_path, canonicalize_normalized_path};
use crate::global_context::GlobalContext;
use crate::files_correction::any_glob_matches_path;

pub use refact_files::indexing_settings::{IndexingEverywhere, IndexingSettings};

// TODO:
// remove debug prints
// react on .git appearing / disappearing => reindex all
// react on indexing.yaml additional_indexing_dirs change => reindex all
// make sure "git ls" lists unstaged files

// Testing:
// ignored file initial indexing doesn't happen
// ignored file add / remove file events don't do anything
// a file in an ignored dir, same tests
// changes in indexing.yaml loaded (almost) immediately

const INDEXING_TOO_OLD: Duration = Duration::from_secs(30);

#[derive(Clone, PartialEq, Eq)]
struct IndexingFileStamp {
    path: PathBuf,
    modified: Option<SystemTime>,
    len: Option<u64>,
}

struct IndexingCheckState {
    checked_ts: u64,
    loaded_ts: u64,
    stamps: Vec<IndexingFileStamp>,
}

static INDEXING_CHECK_STATES: OnceLock<Mutex<HashMap<usize, IndexingCheckState>>> = OnceLock::new();

async fn indexing_file_stamps(paths: &[PathBuf]) -> Vec<IndexingFileStamp> {
    let mut stamps = Vec::with_capacity(paths.len());
    for path in paths {
        let metadata = fs::metadata(path).await.ok();
        stamps.push(IndexingFileStamp {
            path: path.clone(),
            modified: metadata.as_ref().and_then(|metadata| metadata.modified().ok()),
            len: metadata.as_ref().map(|metadata| metadata.len()),
        });
    }
    stamps
}

pub async fn load_indexing_yaml(
    indexing_yaml_path: &Path,
    relative_path_base: Option<&PathBuf>,
) -> Result<IndexingSettings, String> {
    let content = fs::read_to_string(&indexing_yaml_path)
        .await
        .map_err(|e| format!("load {} failed\n{}", indexing_yaml_path.display(), e))?;

    _load_indexing_yaml_str(&content, relative_path_base)
        .map_err(|e| format!("load {} failed\n{}", indexing_yaml_path.display(), e))
}

pub async fn reload_global_indexing_only(gcx: Arc<GlobalContext>) -> IndexingEverywhere {
    let (config_dir, indexing_yaml) =
        { (gcx.config_dir.clone(), gcx.cmdline.indexing_yaml.clone()) };
    let global_indexing_path = if indexing_yaml.is_empty() {
        config_dir.join("indexing.yaml")
    } else {
        canonical_path(indexing_yaml)
    };
    IndexingEverywhere {
        global: load_indexing_yaml(&global_indexing_path, None)
            .await
            .unwrap_or_default(),
        vcs_indexing_settings_map: HashMap::new(),
        loaded_ts: 0,
    }
}

pub async fn reload_indexing_everywhere_if_needed(
    gcx: Arc<GlobalContext>,
) -> Arc<IndexingEverywhere> {
    reload_indexing_everywhere(gcx, false).await
}

pub async fn reload_indexing_everywhere_now(gcx: Arc<GlobalContext>) -> Arc<IndexingEverywhere> {
    reload_indexing_everywhere(gcx, true).await
}

async fn reload_indexing_everywhere(
    gcx: Arc<GlobalContext>,
    force: bool,
) -> Arc<IndexingEverywhere> {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let cache_key = Arc::as_ptr(&gcx) as usize;
    let (config_dir, indexing_yaml, workspace_vcs_roots, indexing_cache, cached) = {
        let indexing_cache = gcx.indexing_everywhere.clone();
        let cached = indexing_cache.read().unwrap().clone();
        let checked_ts = INDEXING_CHECK_STATES
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap()
            .get(&cache_key)
            .map(|state| state.checked_ts)
            .unwrap_or(cached.loaded_ts);
        if !force && checked_ts + INDEXING_TOO_OLD.as_secs() > now {
            return cached;
        }
        (
            gcx.config_dir.clone(),
            gcx.cmdline.indexing_yaml.clone(),
            gcx.documents_state.workspace_vcs_roots.clone(),
            indexing_cache,
            cached,
        )
    };

    let global_indexing_path = if indexing_yaml.is_empty() {
        config_dir.join("indexing.yaml")
    } else {
        canonical_path(&indexing_yaml)
    };
    let vcs_dirs: Vec<PathBuf> = workspace_vcs_roots
        .lock()
        .unwrap()
        .iter()
        .cloned()
        .map(canonicalize_normalized_path)
        .collect();
    let mut indexing_paths = Vec::with_capacity(vcs_dirs.len() + 1);
    indexing_paths.push(global_indexing_path.clone());
    indexing_paths.extend(
        vcs_dirs
            .iter()
            .map(|root| root.join(".refact").join("indexing.yaml")),
    );
    let stamps = indexing_file_stamps(&indexing_paths).await;
    if !force {
        let mut states = INDEXING_CHECK_STATES
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap();
        if states
            .get(&cache_key)
            .map(|state| state.loaded_ts == cached.loaded_ts && state.stamps == stamps)
            .unwrap_or(false)
        {
            states.get_mut(&cache_key).unwrap().checked_ts = now;
            return cached;
        }
    }

    let indexing_everywhere = {
        let global = {
            load_indexing_yaml(&global_indexing_path, None)
                .await
                .unwrap_or_else(|e| {
                    tracing::error!("cannot load {:?}: {}, fallback to defaults", config_dir, e);
                    IndexingSettings::default()
                })
        };

        let mut vcs_indexing_settings_map: HashMap<String, IndexingSettings> = HashMap::new();
        for indexing_root in vcs_dirs {
            let indexing_path = indexing_root.join(".refact").join("indexing.yaml");
            if indexing_path.exists() {
                match load_indexing_yaml(&indexing_path, Some(&indexing_root)).await {
                    Ok(indexing_settings) => {
                        vcs_indexing_settings_map.insert(
                            indexing_root.to_str().unwrap().to_string(),
                            indexing_settings,
                        );
                    }
                    Err(e) => {
                        tracing::error!("{}, skip", e);
                    }
                }
            }
        }
        IndexingEverywhere {
            global,
            vcs_indexing_settings_map,
            loaded_ts: now,
        }
    };

    let indexing_everywhere = Arc::new(indexing_everywhere);
    *indexing_cache.write().unwrap() = indexing_everywhere.clone();
    INDEXING_CHECK_STATES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap()
        .insert(
            cache_key,
            IndexingCheckState {
                checked_ts: now,
                loaded_ts: now,
                stamps,
            },
        );
    indexing_everywhere
}

// pub fn is_this_inside_blocklisted_dir(indexing_settings: &IndexingSettings, path: &PathBuf) -> bool {
//     is_blocklisted(&indexing_settings, &path)
// }

pub fn is_blocklisted(indexing_settings: &IndexingSettings, path: &Path) -> bool {
    let block = any_glob_matches_path(&indexing_settings.blocklist, path);
    // tracing::info!("is_blocklisted {:?} {:?} block={}", indexing_settings, path, block);
    block
}

fn _load_indexing_yaml_str(
    indexing_yaml_str: &str,
    relative_path_base: Option<&PathBuf>,
) -> Result<IndexingSettings, String> {
    match serde_yaml::from_str::<IndexingSettings>(indexing_yaml_str) {
        Ok(indexing_settings) => {
            let mut additional_indexing_dirs = vec![];
            for indexing_dir in indexing_settings.additional_indexing_dirs.iter() {
                if indexing_dir.is_empty() {
                    continue;
                }
                let expanded_dir = if indexing_dir.starts_with("~") {
                    if let Some(without_tilde) = indexing_dir.strip_prefix("~") {
                        let home_dir = PathBuf::from(
                            &home::home_dir()
                                .ok_or(())
                                .expect("failed to find home dir")
                                .to_string_lossy()
                                .to_string(),
                        );
                        home_dir
                            .join(without_tilde.trim_start_matches('/'))
                            .to_string_lossy()
                            .into_owned()
                    } else {
                        indexing_dir.clone()
                    }
                } else {
                    indexing_dir.clone()
                };
                let indexing_dir_path = PathBuf::from(&expanded_dir);
                if indexing_dir_path.is_absolute() {
                    let normalized = crate::files_correction::canonical_path(&expanded_dir)
                        .to_string_lossy()
                        .into_owned();
                    additional_indexing_dirs.push(normalized);
                } else {
                    if let Some(b) = relative_path_base {
                        let joined_path = b.join(&expanded_dir).to_str().unwrap().to_string();
                        let normalized = crate::files_correction::canonical_path(&joined_path)
                            .to_string_lossy()
                            .into_owned();
                        additional_indexing_dirs.push(normalized);
                    } else {
                        tracing::error!(
                            "can't have relative path {} in the global indexing.yaml",
                            indexing_dir
                        )
                    }
                }
            }
            return Ok(IndexingSettings {
                blocklist: indexing_settings.blocklist,
                additional_indexing_dirs,
            });
        }
        Err(e) => {
            return Err(format!("{}", e));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn forced_reload_replaces_the_cached_indexing_settings() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let config_path = gcx.config_dir.join("indexing.yaml");

        tokio::fs::write(&config_path, "blocklist:\n  - first/*\n")
            .await
            .unwrap();
        let first = reload_indexing_everywhere_now(gcx.clone()).await;
        assert_eq!(first.global.blocklist, vec!["first/*"]);

        tokio::fs::write(&config_path, "blocklist:\n  - second/*\n")
            .await
            .unwrap();
        let second = reload_indexing_everywhere_now(gcx.clone()).await;
        let cached = gcx.indexing_everywhere.read().unwrap().clone();

        assert_eq!(second.global.blocklist, vec!["second/*"]);
        assert_eq!(cached.global.blocklist, vec!["second/*"]);
    }
}
