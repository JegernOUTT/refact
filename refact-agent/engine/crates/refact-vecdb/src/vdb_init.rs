use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::time::sleep;
use tracing::{error, info, warn};

use refact_core::vecdb_types::VecdbSearch;

use crate::vdb_highlev::VecDb;
use crate::vdb_structs::VecdbConstants;

pub struct VecDbInitConfig {
    pub max_attempts: usize,
    pub initial_delay_ms: u64,
    pub max_delay_ms: u64,
    pub backoff_factor: f64,
    pub test_search_after_init: bool,
}

impl Default for VecDbInitConfig {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            initial_delay_ms: 500,
            max_delay_ms: 10000,
            backoff_factor: 2.0,
            test_search_after_init: true,
        }
    }
}

#[derive(Debug)]
pub enum VecDbInitError {
    InitializationError(String),
    TestSearchError(String),
    ShutdownRequested,
}

impl std::fmt::Display for VecDbInitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VecDbInitError::InitializationError(msg) => write!(f, "Initialization error: {}", msg),
            VecDbInitError::TestSearchError(msg) => write!(f, "Test search error: {}", msg),
            VecDbInitError::ShutdownRequested => write!(f, "shutdown requested"),
        }
    }
}

pub async fn init_vecdb_fail_safe(
    vecdb_dir: &PathBuf,
    legacy_cache_dir: &PathBuf,
    workspace_folder: String,
    insecure: bool,
    constants: VecdbConstants,
    init_config: VecDbInitConfig,
    shutdown_flag: Arc<AtomicBool>,
) -> Result<VecDb, VecDbInitError> {
    let mut attempt: usize = 0;
    let mut delay = Duration::from_millis(init_config.initial_delay_ms);

    loop {
        if shutdown_flag.load(Ordering::Relaxed) {
            return Err(VecDbInitError::ShutdownRequested);
        }

        attempt += 1;
        info!(
            "VecDb init attempt {}/{}",
            attempt, init_config.max_attempts
        );

        match VecDb::init(
            vecdb_dir,
            legacy_cache_dir,
            workspace_folder.clone(),
            insecure,
            constants.clone(),
        )
        .await
        {
            Ok(vecdb) => {
                info!("Successfully initialized VecDb on attempt {}", attempt);
                if init_config.test_search_after_init {
                    match vecdb_test_search(&vecdb).await {
                        Ok(_) => {
                            info!("VecDb test search successful");
                            return Ok(vecdb);
                        }
                        Err(err) => {
                            warn!("VecDb test search failed: {}", err);
                            if attempt >= init_config.max_attempts {
                                return Err(VecDbInitError::TestSearchError(err));
                            }
                        }
                    }
                } else {
                    return Ok(vecdb);
                }
            }
            Err(err) => {
                if attempt >= init_config.max_attempts {
                    error!(
                        "VecDb initialization failed after {} attempts. Last error: {}",
                        attempt, err
                    );
                    return Err(VecDbInitError::InitializationError(err));
                } else {
                    warn!("VecDb initialization attempt {} failed with error: {}. Retrying in {:?}...", attempt, err, delay);
                }
            }
        }

        let flag = shutdown_flag.clone();
        tokio::select! {
            _ = sleep(delay) => {}
            _ = async move { while !flag.load(Ordering::Relaxed) { tokio::time::sleep(Duration::from_millis(50)).await; } } => {
                return Err(VecDbInitError::ShutdownRequested);
            }
        }

        let new_delay_ms = (delay.as_millis() as f64 * init_config.backoff_factor) as u64;
        delay = Duration::from_millis(new_delay_ms.min(init_config.max_delay_ms));
    }
}

async fn vecdb_test_search(vecdb: &VecDb) -> Result<(), String> {
    match VecdbSearch::vecdb_search(vecdb, "test query".to_string(), 3, None).await {
        Ok(_) => Ok(()),
        Err(e) => Err(format!("Test search failed: {}", e)),
    }
}
