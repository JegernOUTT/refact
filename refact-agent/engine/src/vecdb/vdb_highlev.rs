use std::sync::Arc;
use tokio::sync::Mutex as AMutex;
use tracing::{error, info};

use refact_core::vecdb_types::VecdbSearch;

use crate::background_tasks::BackgroundTasksHolder;
use crate::global_context::GlobalContext;
use crate::vecdb::vdb_structs::{EmbeddingModelConfig, VecDbStatus, VecdbConstants};

pub use refact_vecdb::vdb_highlev::VecDb;

async fn do_i_need_to_reload_vecdb(gcx: Arc<GlobalContext>) -> (bool, Option<VecdbConstants>) {
    let caps =
        match crate::global_context::try_load_caps_quickly_if_not_present(gcx.clone(), 0).await {
            Ok(caps) => caps,
            Err(e) => {
                info!(
                    "vecdb: no caps, will not start or reload vecdb, the error was: {}",
                    e
                );
                return (false, None);
            }
        };

    let vecdb_max_files = gcx.cmdline.vecdb_max_files;
    let embedding_config = EmbeddingModelConfig::from(&caps.embedding_model);
    let splitter_window_size = caps.embedding_model.base.n_ctx / 2;

    let mut consts = VecdbConstants {
        embedding_model: embedding_config.clone(),
        tokenizer: None,
        splitter_window_size,
        vecdb_max_files,
    };

    let vec_db = gcx.vec_db.clone();
    match *vec_db.lock().await {
        None => {}
        Some(ref db) => {
            let (current_emb, current_splitter_window_size) = db.current_constants();
            if current_emb == consts.embedding_model
                && current_splitter_window_size == consts.splitter_window_size
            {
                return (false, None);
            }
        }
    }

    if consts.embedding_model.model_name.is_empty() || consts.embedding_model.endpoint.is_empty() {
        error!("command line says to launch vecdb, but this will not happen: embedding model name or endpoint are empty");
        return (true, None);
    }

    let tokenizer_result =
        crate::tokens::cached_tokenizer(gcx.clone(), &caps.embedding_model.base).await;

    consts.tokenizer = match tokenizer_result {
        Ok(tokenizer) => tokenizer,
        Err(err) => {
            error!(
                "vecdb launch failed, embedding model tokenizer didn't load: {}",
                err
            );
            return (false, None);
        }
    };
    return (true, Some(consts));
}

pub async fn vecdb_background_reload(gcx: Arc<GlobalContext>) {
    let cmd_line = gcx.cmdline.clone();
    if !cmd_line.vecdb {
        return;
    }

    let mut background_tasks = BackgroundTasksHolder::new(vec![]);
    loop {
        if gcx.shutdown_flag.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        let (need_reload, consts) = do_i_need_to_reload_vecdb(gcx.clone()).await;
        if need_reload {
            background_tasks.abort().await;
        }
        if need_reload && consts.is_some() {
            background_tasks = BackgroundTasksHolder::new(vec![]);

            let init_config = refact_vecdb::vdb_init::VecDbInitConfig {
                max_attempts: 5,
                initial_delay_ms: 10,
                max_delay_ms: 1000,
                backoff_factor: 2.0,
                test_search_after_init: true,
            };
            {
                let ev = crate::buddy::actor::make_runtime_event(
                    "vecdb_building",
                    "Building vector embeddings...",
                    "indexer",
                    "vecdb",
                    "started",
                    None,
                );
                crate::buddy::actor::buddy_enqueue_event(
                    crate::app_state::AppState::from_gcx(gcx.clone()).await,
                    ev,
                )
                .await;
            }
            match initialize_vecdb_with_context(gcx.clone(), consts.unwrap(), Some(init_config))
                .await
            {
                Ok(_) => {
                    *gcx.vec_db_error.lock().unwrap() = "".to_string();
                    info!("vecdb: initialization successful");
                    let ev = crate::buddy::actor::make_runtime_event(
                        "vecdb_building",
                        "VecDB ready",
                        "indexer",
                        "vecdb",
                        "completed",
                        None,
                    );
                    crate::buddy::actor::buddy_enqueue_event(
                        crate::app_state::AppState::from_gcx(gcx.clone()).await,
                        ev,
                    )
                    .await;
                }
                Err(refact_vecdb::vdb_init::VecDbInitError::ShutdownRequested) => break,
                Err(err) => {
                    let err_msg = err.to_string();
                    *gcx.vec_db_error.lock().unwrap() = err_msg.clone();
                    error!("vecdb init failed: {}", err_msg);
                }
            }
        }
        let shutdown_flag = gcx.shutdown_flag.clone();
        tokio::select! {
            _ = tokio::time::sleep(tokio::time::Duration::from_secs(60)) => {}
            _ = async move { while !shutdown_flag.load(std::sync::atomic::Ordering::Relaxed) { tokio::time::sleep(tokio::time::Duration::from_millis(50)).await; } } => {
                break;
            }
        }
    }
}

pub async fn get_status(
    vec_db: Arc<AMutex<Option<Arc<dyn VecdbSearch>>>>,
) -> Result<Option<VecDbStatus>, String> {
    let db_locked = vec_db.lock().await;
    match db_locked.as_ref() {
        None => Ok(None),
        Some(db) => Ok(Some(db.get_status().await?)),
    }
}

async fn initialize_vecdb_with_context(
    gcx: Arc<GlobalContext>,
    constants: VecdbConstants,
    init_config: Option<refact_vecdb::vdb_init::VecDbInitConfig>,
) -> Result<(), refact_vecdb::vdb_init::VecDbInitError> {
    let (legacy_cache_dir, cmdline, shutdown_flag) = {
        (
            gcx.cache_dir.clone(),
            gcx.cmdline.clone(),
            gcx.shutdown_flag.clone(),
        )
    };

    let vecdb_dir = if !cmdline.vecdb_force_path.is_empty() {
        std::path::PathBuf::from(&cmdline.vecdb_force_path)
    } else if let Some(dir) = get_default_vecdb_dir(gcx.clone()).await {
        dir
    } else {
        legacy_cache_dir.join("vecdb")
    };

    let config = init_config.unwrap_or_default();
    let vec_db = refact_vecdb::vdb_init::init_vecdb_fail_safe(
        &vecdb_dir,
        &legacy_cache_dir,
        cmdline.workspace_folder.clone(),
        cmdline.insecure,
        constants,
        config,
        shutdown_flag,
    )
    .await?;

    let shutdown_flag2 = gcx.shutdown_flag.clone();
    let gcx_clone = gcx.clone();
    let file_reader: refact_core::vecdb_types::FileReader = Arc::new(move |path| {
        let gcx = gcx_clone.clone();
        Box::pin(async move {
            let mut doc = refact_core::ast_types::Document::new(&path);
            crate::files_in_workspace::update_document_text_from_disk(&mut doc, gcx).await?;
            doc.text_as_string()
        })
    });

    let tasks = vec_db
        .vecdb_start_background_tasks(shutdown_flag2, file_reader)
        .await;
    let _background_tasks = BackgroundTasksHolder::new(tasks);

    let vec_db_arc: Arc<dyn VecdbSearch> = Arc::new(vec_db);
    {
        let (vec_db, vec_db_error) = { (gcx.vec_db.clone(), gcx.vec_db_error.clone()) };
        *vec_db.lock().await = Some(vec_db_arc);
        *vec_db_error.lock().unwrap() = "".to_string();
    }

    crate::files_in_workspace::enqueue_all_files_from_workspace_folders(gcx.clone(), true, true)
        .await;
    crate::files_in_jsonl::enqueue_all_docs_from_jsonl_but_read_first(gcx.clone(), true, true)
        .await;

    info!("VecDb initialization and setup complete");
    Ok(())
}

async fn get_default_vecdb_dir(gcx: Arc<GlobalContext>) -> Option<std::path::PathBuf> {
    let project_dirs = crate::files_correction::get_project_dirs(gcx).await;
    project_dirs
        .first()
        .map(|root| root.join(".refact").join("vecdb"))
}
