use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::path_utils::canonicalize_normalized_path;

#[derive(Debug, Clone, Deserialize)]
pub struct IndexingSettings {
    #[serde(default)]
    pub blocklist: Vec<String>,
    #[serde(default)]
    pub additional_indexing_dirs: Vec<String>,
}

impl Default for IndexingSettings {
    fn default() -> Self {
        serde_yaml::from_str(include_str!(
            "../../../src/yaml_configs/default_indexing.yaml"
        ))
        .expect("src/yaml_configs/default_indexing.yaml to be valid IndexingSettings")
    }
}

pub struct IndexingEverywhere {
    pub global: IndexingSettings,
    pub vcs_indexing_settings_map: HashMap<String, IndexingSettings>,
    pub loaded_ts: u64,
}

impl Default for IndexingEverywhere {
    fn default() -> Self {
        IndexingEverywhere {
            global: IndexingSettings::default(),
            vcs_indexing_settings_map: HashMap::new(),
            loaded_ts: 0,
        }
    }
}

impl IndexingEverywhere {
    pub fn indexing_for_path(&self, path: &Path) -> IndexingSettings {
        assert!(path.is_absolute());
        let path = canonicalize_normalized_path(path.to_path_buf());
        let mut result: IndexingSettings = self.global.clone();

        let mut best_vcs: Option<IndexingSettings> = None;
        let mut best_pathbuf: Option<PathBuf> = None;
        for (vcs, vcs_settings) in &self.vcs_indexing_settings_map {
            let vcs_pathbuf = canonicalize_normalized_path(PathBuf::from(vcs));
            if path.starts_with(&vcs_pathbuf) {
                if best_vcs.is_none()
                    || vcs_pathbuf.components().count()
                        > best_pathbuf.clone().unwrap().components().count()
                {
                    best_vcs = Some(vcs_settings.clone());
                    best_pathbuf = Some(vcs_pathbuf);
                }
            }
        }

        if let Some(t) = best_vcs {
            result.blocklist.extend(t.blocklist);
            result
                .additional_indexing_dirs
                .extend(t.additional_indexing_dirs);
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexing_everywhere_default_starts_unloaded() {
        let indexing = IndexingEverywhere::default();

        assert!(indexing.vcs_indexing_settings_map.is_empty());
        assert_eq!(indexing.loaded_ts, 0);
    }
}
