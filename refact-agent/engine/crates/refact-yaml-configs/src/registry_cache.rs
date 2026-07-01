use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use refact_files::path_utils::canonicalize_normalized_path;

use crate::customization_types::ProjectRegistry;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RegistryCache {
    pub project_root: PathBuf,
    pub registry: ProjectRegistry,
    pub last_scan: SystemTime,
}

pub struct RegistryCacheManager {
    pub cache: HashMap<PathBuf, RegistryCache>,
}

impl RegistryCacheManager {
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }

    pub fn get(&self, project_root: &Path) -> Option<&RegistryCache> {
        self.cache.get(project_root).or_else(|| {
            let normalized = canonicalize_normalized_path(project_root.to_path_buf());
            self.cache.get(&normalized)
        })
    }

    pub fn insert(&mut self, project_root: PathBuf, registry: ProjectRegistry) {
        let project_root = canonicalize_normalized_path(project_root);
        self.cache.insert(
            project_root.clone(),
            RegistryCache {
                project_root,
                registry,
                last_scan: SystemTime::now(),
            },
        );
    }

    #[allow(dead_code)]
    pub fn remove(&mut self, project_root: &Path) {
        self.cache.remove(project_root);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_cache_manager_new_is_empty() {
        let manager = RegistryCacheManager::new();

        assert!(manager.cache.is_empty());
    }
}
