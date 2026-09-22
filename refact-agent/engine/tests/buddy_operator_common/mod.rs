//! Scratch-root helpers for Buddy operator-effect tests.

pub mod fixtures;
pub mod worker;

use std::path::Path;

use refact_lsp::buddy::{
    actor::BuddyService, runtime_queue::RuntimeQueue, settings::BuddySettings,
    state::default_buddy_state, storage::load_drafts,
};
use tempfile::TempDir;
use tokio::sync::broadcast;

/// A scratch project and a separately owned temporary user-config root.
pub struct BuddyOperatorHarness {
    project: TempDir,
    user_config: TempDir,
    shutdown: bool,
}

impl BuddyOperatorHarness {
    pub fn new() -> Self {
        let project = tempfile::tempdir().expect("create scratch project");
        std::fs::create_dir_all(project.path().join(".refact/buddy"))
            .expect("create scratch Buddy storage directory");
        std::fs::write(
            project.path().join("README.md"),
            "# Buddy operator scratch\n",
        )
        .expect("seed scratch project");
        Self {
            project,
            user_config: tempfile::tempdir().expect("create scratch user config"),
            shutdown: false,
        }
    }

    pub fn project_root(&self) -> &Path {
        self.project.path()
    }

    pub fn user_config_root(&self) -> &Path {
        self.user_config.path()
    }

    pub fn service(&self) -> BuddyService {
        let (events_tx, _) = broadcast::channel(16);
        BuddyService::new(
            self.project_root().to_path_buf(),
            default_buddy_state(),
            BuddySettings::default(),
            Vec::new(),
            RuntimeQueue::new(),
            events_tx,
            None,
        )
    }

    /// Restore persisted drafts into a newly constructed real Buddy service.
    pub async fn restored_service(&self) -> BuddyService {
        let mut service = self.service();
        service
            .draft_store
            .restore(load_drafts(self.project_root()).await);
        service
    }

    /// Explicit and idempotent lifecycle hook for future process-owning tests.
    pub fn shutdown(&mut self) {
        self.shutdown = true;
    }

    pub fn is_shutdown(&self) -> bool {
        self.shutdown
    }
}

impl Drop for BuddyOperatorHarness {
    fn drop(&mut self) {
        self.shutdown();
    }
}
