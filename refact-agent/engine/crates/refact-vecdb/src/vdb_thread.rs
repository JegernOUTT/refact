use indexmap::IndexMap;
use std::collections::HashSet;
use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::ops::Div;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};
use std::time::SystemTime;
use tokio::sync::{Mutex as AMutex, Notify as ANotify};
use tokio::task::JoinHandle;
use tracing::{info, warn};

use refact_core::ast_types::Document;
use refact_core::memory_plane::{MemoryPlaneFileKind, MemoryPlaneRoots};
use refact_core::vecdb_types::FileReader;

use crate::fetch_embedding::get_embedding_with_retries;
use crate::vdb_markdown_splitter::MarkdownFileSplitter;
use crate::vdb_sqlite::VecDBSqlite;
use crate::vdb_structs::{SimpleTextHashVector, SplitResult, VecDbStatus, VecdbConstants, VecdbRecord};
use crate::vdb_trajectory_splitter::TrajectoryFileSplitter;

const DEBUG_WRITE_VECDB_FILES: bool = false;
const COOLDOWN_SECONDS: u64 = 10;

fn memory_plane_file_kind(
    path: &PathBuf,
    roots: &MemoryPlaneRoots,
) -> Result<MemoryPlaneFileKind, String> {
    roots
        .classify_file(path)
        .ok_or_else(|| format!("Unsupported memory-plane path {}", path.display()))
}

fn is_path_to_enqueue_valid(path: &PathBuf, roots: &MemoryPlaneRoots) -> Result<(), String> {
    memory_plane_file_kind(path, roots).map(|_| ())
}

enum MessageToVecdbThread {
    RegularDocument(String),
    ImmediatelyRegularDocument(String),
}

pub struct FileVectorizerService {
    pub vecdb_handler: Arc<AMutex<VecDBSqlite>>,
    pub vstatus: Arc<AMutex<VecDbStatus>>,
    pub vstatus_notify: Arc<ANotify>,
    constants: VecdbConstants,
    memory_plane_roots: Arc<RwLock<MemoryPlaneRoots>>,
    vecdb_todo: Arc<AMutex<VecDeque<MessageToVecdbThread>>>,
}

async fn vectorize_batch_from_q(
    run_actual_model_on_these: &mut Vec<SplitResult>,
    ready_to_vecdb: &mut Vec<VecdbRecord>,
    vstatus: Arc<AMutex<VecDbStatus>>,
    client: Arc<AMutex<reqwest::Client>>,
    constants: &VecdbConstants,
    vecdb_handler_arc: Arc<AMutex<VecDBSqlite>>,
) -> Result<(), String> {
    #[allow(non_snake_case)]
    let B = constants.embedding_model.embedding_batch;
    let batch = run_actual_model_on_these
        .drain(..B.min(run_actual_model_on_these.len()))
        .collect::<Vec<_>>();
    assert!(batch.len() > 0);

    let batch_result = match get_embedding_with_retries(
        client.clone(),
        &constants.embedding_model,
        batch.iter().map(|x| x.window_text.clone()).collect(),
        10,
    )
    .await
    {
        Ok(res) => res,
        Err(e) => {
            let mut vstatus_locked = vstatus.lock().await;
            vstatus_locked
                .vecdb_errors
                .entry(e.clone())
                .and_modify(|counter| *counter += 1)
                .or_insert(1);
            return Err(e);
        }
    };

    if batch_result.len() != batch.len() {
        return Err(format!(
            "vectorize: batch_result.len() != batch.len(): {} vs {}",
            batch_result.len(),
            batch.len()
        ));
    }

    {
        let mut vstatus_locked = vstatus.lock().await;
        vstatus_locked.requests_made_since_start += 1;
        vstatus_locked.vectors_made_since_start += batch_result.len();
    }

    let mut send_to_cache = vec![];
    for (i, data_res) in batch.iter().enumerate() {
        if batch_result[i].is_empty() {
            info!("skipping an empty embedding split");
            continue;
        }
        ready_to_vecdb.push(VecdbRecord {
            vector: Some(batch_result[i].clone()),
            file_path: data_res.file_path.clone(),
            start_line: data_res.start_line,
            end_line: data_res.end_line,
            distance: -1.0,
            usefulness: 0.0,
        });
        send_to_cache.push(SimpleTextHashVector {
            vector: Some(batch_result[i].clone()),
            window_text: data_res.window_text.clone(),
            window_text_hash: data_res.window_text_hash.clone(),
        });
    }

    if send_to_cache.len() > 0 {
        match vecdb_handler_arc
            .lock()
            .await
            .cache_add_new_records(send_to_cache)
            .await
        {
            Err(e) => warn!("Error adding records to the cacheDB: {}", e),
            _ => {}
        }
    }

    tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;

    Ok(())
}

async fn from_splits_to_vecdb_records_applying_cache(
    splits: &mut Vec<SplitResult>,
    ready_to_vecdb: &mut Vec<VecdbRecord>,
    run_actual_model_on_these: &mut Vec<SplitResult>,
    vecdb_handler_arc: Arc<AMutex<VecDBSqlite>>,
    group_size: usize,
) {
    while !splits.is_empty() {
        let batch: Vec<SplitResult> = splits
            .drain(..group_size.min(splits.len()))
            .collect::<Vec<_>>();
        let vectors_maybe = vecdb_handler_arc
            .lock()
            .await
            .fetch_vectors_from_cache(&batch)
            .await;
        if let Ok(vectors) = vectors_maybe {
            for (split, maybe_vector) in batch.iter().zip(vectors.iter()) {
                if maybe_vector.is_none() {
                    run_actual_model_on_these.push(split.clone());
                    continue;
                }
                ready_to_vecdb.push(VecdbRecord {
                    vector: maybe_vector.clone(),
                    file_path: split.file_path.clone(),
                    start_line: split.start_line,
                    end_line: split.end_line,
                    distance: -1.0,
                    usefulness: 0.0,
                });
            }
        } else if let Err(err) = vectors_maybe {
            tracing::error!("{}", err);
        }
    }
}

async fn _send_to_vecdb(
    vecdb_handler_arc: Arc<AMutex<VecDBSqlite>>,
    ready_to_vecdb: &mut Vec<VecdbRecord>,
) {
    let file_paths: HashSet<PathBuf> = ready_to_vecdb.iter().map(|r| r.file_path.clone()).collect();
    for file_path in &file_paths {
        match vecdb_handler_arc
            .lock()
            .await
            .vecdb_records_remove(vec![file_path.to_string_lossy().to_string()])
            .await
        {
            Ok(_) => {}
            Err(err) => info!("VECDB Error removing: {}", err),
        }
    }
    match vecdb_handler_arc
        .lock()
        .await
        .vecdb_records_add(ready_to_vecdb)
        .await
    {
        Ok(_) => {}
        Err(err) => info!("VECDB Error adding: {}", err),
    }
    ready_to_vecdb.clear();
}

async fn vectorize_thread(
    client: Arc<AMutex<reqwest::Client>>,
    vservice: Arc<AMutex<FileVectorizerService>>,
    shutdown_flag: Arc<AtomicBool>,
    file_reader: FileReader,
) {
    let mut files_total: usize = 0;
    let mut files_unprocessed: usize;
    let mut reported_unprocessed: usize = 0;
    let mut run_actual_model_on_these: Vec<SplitResult> = vec![];
    let mut ready_to_vecdb: Vec<VecdbRecord> = vec![];

    let (vecdb_todo, constants, memory_plane_roots, vecdb_handler_arc, vstatus, vstatus_notify) = {
        let vservice_locked = vservice.lock().await;
        (
            vservice_locked.vecdb_todo.clone(),
            vservice_locked.constants.clone(),
            vservice_locked.memory_plane_roots.clone(),
            vservice_locked.vecdb_handler.clone(),
            vservice_locked.vstatus.clone(),
            vservice_locked.vstatus_notify.clone(),
        )
    };

    let mut last_updated: HashMap<String, SystemTime> = HashMap::new();
    loop {
        if shutdown_flag.load(std::sync::atomic::Ordering::SeqCst) {
            tracing::info!("VecDB thread: shutdown detected, stopping");
            return;
        }
        let mut work_on_one: Option<MessageToVecdbThread> = None;
        let current_time = SystemTime::now();
        let mut vstatus_changed = false;
        {
            let mut vecdb_todo_locked = vecdb_todo.lock().await;
            while let Some(msg) = vecdb_todo_locked.pop_front() {
                match msg {
                    MessageToVecdbThread::RegularDocument(cpath) => {
                        last_updated.insert(cpath, current_time);
                    }
                    MessageToVecdbThread::ImmediatelyRegularDocument(_) => {
                        work_on_one = Some(msg);
                        break;
                    }
                }
            }
            if work_on_one.is_none() {
                let doc_to_remove = last_updated
                    .iter()
                    .find(|(_, time)| {
                        time.elapsed().unwrap_or_default().as_secs() > COOLDOWN_SECONDS
                    })
                    .map(|(doc, _)| doc.clone());
                if let Some(doc) = doc_to_remove {
                    work_on_one = Some(MessageToVecdbThread::RegularDocument(doc.clone()));
                    last_updated.remove(&doc);
                }
            }
            files_unprocessed = vecdb_todo_locked.len()
                + last_updated.len()
                + if work_on_one.is_some() { 1 } else { 0 };
            files_total = files_total.max(files_unprocessed);
            {
                let mut vstatus_locked = vstatus.lock().await;
                vstatus_locked.files_unprocessed = files_unprocessed;
                vstatus_locked.files_total = files_total;
                vstatus_locked.queue_additions = false;
                if work_on_one.is_some() && vstatus_locked.state != "parsing" {
                    vstatus_locked.state = "parsing".to_string();
                    vstatus_changed = true;
                }
                if work_on_one.is_none()
                    && files_unprocessed > 0
                    && vstatus_locked.state != "cooldown"
                {
                    vstatus_locked.state = "cooldown".to_string();
                    vstatus_changed = true;
                }
            }
        }
        if vstatus_changed {
            vstatus_notify.notify_waiters();
        }

        let flush = ready_to_vecdb.len() > 100 || files_unprocessed == 0 || work_on_one.is_none();
        loop {
            if run_actual_model_on_these.len() > 0 && flush
                || run_actual_model_on_these.len() >= constants.embedding_model.embedding_batch
            {
                if let Err(err) = vectorize_batch_from_q(
                    &mut run_actual_model_on_these,
                    &mut ready_to_vecdb,
                    vstatus.clone(),
                    client.clone(),
                    &constants,
                    vecdb_handler_arc.clone(),
                )
                .await
                {
                    tracing::error!("{}", err);
                    continue;
                }
            } else {
                break;
            }
        }

        if flush {
            assert!(run_actual_model_on_these.len() == 0);
            _send_to_vecdb(vecdb_handler_arc.clone(), &mut ready_to_vecdb).await;
        }

        if (files_unprocessed + 99).div(100) != (reported_unprocessed + 99).div(100) {
            info!("have {} unprocessed files", files_unprocessed);
            reported_unprocessed = files_unprocessed;
        }
        let cpath = {
            match work_on_one {
                Some(MessageToVecdbThread::RegularDocument(cpath))
                | Some(MessageToVecdbThread::ImmediatelyRegularDocument(cpath)) => cpath.clone(),
                None if last_updated.is_empty() => {
                    assert!(run_actual_model_on_these.is_empty());
                    assert!(ready_to_vecdb.is_empty());
                    let reported_vecdb_complete = {
                        let mut vstatus_locked = vstatus.lock().await;
                        let done = vstatus_locked.state == "done";
                        if !done {
                            files_total = 0;
                            vstatus_locked.files_unprocessed = 0;
                            vstatus_locked.files_total = 0;
                            vstatus_locked.state = "done".to_string();
                            info!(
                                "vectorizer since start {} API calls, {} vectors",
                                vstatus_locked.requests_made_since_start,
                                vstatus_locked.vectors_made_since_start
                            );
                        }
                        done
                    };
                    if !reported_vecdb_complete {
                        let _ = write!(std::io::stderr(), "VECDB COMPLETE\n");
                        info!("VECDB COMPLETE");
                        let vectors_count = {
                            let vstatus_locked = vstatus.lock().await;
                            vstatus_locked.vectors_made_since_start
                        };
                        let _vecdb_msg = if vectors_count > 0 {
                            format!("VecDB complete: {} vectors indexed", vectors_count)
                        } else {
                            "VecDB ready".to_string()
                        };
                        vstatus_notify.notify_waiters();
                        {
                            let vstatus_locked = vstatus.lock().await;
                            if !vstatus_locked.vecdb_errors.is_empty() {
                                info!("VECDB ERRORS: {:#?}", vstatus_locked.vecdb_errors);
                            }
                        }
                    }
                    tokio::select! {
                        _ = tokio::time::sleep(tokio::time::Duration::from_millis(1_000)) => {},
                        _ = vstatus_notify.notified() => {},
                    }
                    continue;
                }
                _ => continue,
            }
        };

        let last_30_chars = refact_core::custom_error::last_n_chars(&cpath, 30);
        let doc_path: PathBuf = cpath.clone().into();

        let text_result = file_reader(doc_path.clone()).await;
        let text = match text_result {
            Ok(t) => t,
            Err(_) => {
                info!("{} cannot read, deleting from index", last_30_chars);
                match vecdb_handler_arc
                    .lock()
                    .await
                    .vecdb_records_remove(vec![cpath.clone()])
                    .await
                {
                    Ok(_) => {}
                    Err(err) => info!("VECDB Error removing: {}", err),
                }
                continue;
            }
        };

        let file_kind = {
            let roots = memory_plane_roots.read().unwrap();
            match memory_plane_file_kind(&doc_path, &roots) {
                Ok(kind) => kind,
                Err(err) => {
                    info!("embeddings {} rejected: {}", last_30_chars, err);
                    continue;
                }
            }
        };

        let mut doc = Document::new(&doc_path);
        doc.update_text(&text);

        if file_kind == MemoryPlaneFileKind::KnowledgeMarkdown {
            if let Err(err) = doc.does_text_look_good() {
                info!("embeddings {} doesn't look good: {}", last_30_chars, err);
                continue;
            }
        }

        let mut splits = if file_kind == MemoryPlaneFileKind::TrajectoryJson {
            let traj_splitter = TrajectoryFileSplitter::new(constants.splitter_window_size);
            traj_splitter
                .split(&text, &doc.doc_path)
                .await
                .unwrap_or_else(|err| {
                    info!("{}", err);
                    vec![]
                })
        } else {
            let md_splitter = MarkdownFileSplitter::new(constants.embedding_model.n_ctx);
            md_splitter
                .split(&text, &doc.doc_path)
                .await
                .unwrap_or_else(|err| {
                    info!("{}", err);
                    vec![]
                })
        };

        if let Some(filename) = doc.doc_path.file_name() {
            let filename_str = filename.to_string_lossy().to_string();
            if !filename_str.is_empty() {
                splits.push(SplitResult {
                    file_path: doc.doc_path.clone(),
                    window_text: filename_str.clone(),
                    window_text_hash: refact_core::chunk_utils::official_text_hashing_function(
                        &filename_str,
                    ),
                    start_line: 0,
                    end_line: 0,
                    symbol_path: "filename".to_string(),
                });
            }
        }

        if DEBUG_WRITE_VECDB_FILES {
            let _ = std::fs::write(
                format!("/tmp/vecdb_{}.txt", last_30_chars.replace("/", "_")),
                splits
                    .iter()
                    .map(|s| s.window_text.clone())
                    .collect::<Vec<_>>()
                    .join("\n---\n"),
            );
        }

        from_splits_to_vecdb_records_applying_cache(
            &mut splits,
            &mut ready_to_vecdb,
            &mut run_actual_model_on_these,
            vecdb_handler_arc.clone(),
            10,
        )
        .await;
    }
}

impl FileVectorizerService {
    pub async fn new(
        vecdb_handler: Arc<AMutex<VecDBSqlite>>,
        constants: VecdbConstants,
        memory_plane_roots: MemoryPlaneRoots,
    ) -> Self {
        let vstatus = Arc::new(AMutex::new(VecDbStatus {
            files_unprocessed: 0,
            files_total: 0,
            requests_made_since_start: 0,
            vectors_made_since_start: 0,
            db_size: 0,
            db_cache_size: 0,
            state: "starting".to_string(),
            queue_additions: true,
            vecdb_max_files_hit: false,
            vecdb_errors: IndexMap::new(),
        }));
        FileVectorizerService {
            vecdb_handler: vecdb_handler.clone(),
            vstatus: vstatus.clone(),
            vstatus_notify: Arc::new(ANotify::new()),
            constants,
            memory_plane_roots: Arc::new(RwLock::new(memory_plane_roots)),
            vecdb_todo: Default::default(),
        }
    }
}

pub async fn vecdb_start_background_tasks(
    vecdb_client: Arc<AMutex<reqwest::Client>>,
    vservice: Arc<AMutex<FileVectorizerService>>,
    shutdown_flag: Arc<AtomicBool>,
    file_reader: FileReader,
) -> Vec<JoinHandle<()>> {
    let retrieve_thread_handle = tokio::spawn(vectorize_thread(
        vecdb_client.clone(),
        vservice.clone(),
        shutdown_flag,
        file_reader,
    ));
    vec![retrieve_thread_handle]
}

fn _filter_docs_to_enqueue(docs: &[String], roots: &MemoryPlaneRoots) -> Vec<String> {
    let mut rejected_reasons = HashMap::new();
    let mut filtered_docs = vec![];
    for d in docs {
        let path: PathBuf = d.clone().into();
        match is_path_to_enqueue_valid(&path, roots) {
            Ok(_) => filtered_docs.push(d.clone()),
            Err(e) => {
                rejected_reasons
                    .entry(e.to_string())
                    .and_modify(|x| *x += 1)
                    .or_insert(1);
            }
        }
    }
    if !rejected_reasons.is_empty() {
        info!("VecDB rejected docs to enqueue reasons:");
        for (reason, count) in &rejected_reasons {
            info!("    {:>6} {}", count, reason);
        }
    }
    filtered_docs
}

pub async fn vectorizer_enqueue_files(
    vservice: Arc<AMutex<FileVectorizerService>>,
    documents: &[String],
    process_immediately: bool,
    roots: &MemoryPlaneRoots,
) {
    info!("adding {} files", documents.len());
    let documents = _filter_docs_to_enqueue(documents, roots);
    let (vecdb_todo, vstatus, vstatus_notify, vecdb_max_files) = {
        let service = vservice.lock().await;
        *service.memory_plane_roots.write().unwrap() = roots.clone();
        (
            service.vecdb_todo.clone(),
            service.vstatus.clone(),
            service.vstatus_notify.clone(),
            service.constants.vecdb_max_files,
        )
    };
    let mut documents_my_copy = documents.clone();
    if documents_my_copy.len() > vecdb_max_files {
        info!(
            "that's more than {} allowed in the command line, reduce the number",
            vecdb_max_files
        );
        documents_my_copy.truncate(vecdb_max_files);
        vstatus.lock().await.vecdb_max_files_hit = true;
    }
    {
        {
            let mut vecdb_todo_locked = vecdb_todo.lock().await;
            for doc in documents_my_copy.iter() {
                if process_immediately {
                    vecdb_todo_locked.push_back(MessageToVecdbThread::ImmediatelyRegularDocument(
                        doc.clone(),
                    ));
                } else {
                    vecdb_todo_locked.push_back(MessageToVecdbThread::RegularDocument(doc.clone()));
                }
            }
            vstatus.lock().await.queue_additions = true;
        }
        if process_immediately {
            vstatus_notify.notify_waiters();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots() -> MemoryPlaneRoots {
        MemoryPlaneRoots::new(
            vec![PathBuf::from("/workspace/project")],
            Some(PathBuf::from("/home/user/.config/refact/knowledge")),
            Some(PathBuf::from("/home/user/.config/refact/trajectories")),
        )
    }

    #[test]
    fn filter_docs_accepts_only_memory_plane_files() {
        let roots = roots();
        let knowledge = "/workspace/project/.refact/knowledge/note.md".to_string();
        let trajectory =
            "/workspace/project/.refact/tasks/task-1/trajectories/agents/chat.json".to_string();
        let source_file = "/workspace/project/src/main.rs".to_string();
        let task_memory = "/workspace/project/.refact/tasks/task-1/memories/note.md".to_string();
        let broad_trajectory =
            "/workspace/project/src/tasks/task-1/trajectories/chat.json".to_string();

        let filtered = _filter_docs_to_enqueue(
            &[
                knowledge.clone(),
                trajectory.clone(),
                source_file,
                task_memory,
                broad_trajectory,
            ],
            &roots,
        );

        assert_eq!(filtered, vec![knowledge, trajectory]);
    }

    #[test]
    fn enqueue_validation_rejects_source_file_at_sink() {
        let roots = roots();

        assert!(
            is_path_to_enqueue_valid(&PathBuf::from("/workspace/project/src/main.rs"), &roots)
                .is_err()
        );
        assert!(is_path_to_enqueue_valid(
            &PathBuf::from("/workspace/project/.refact/knowledge/source.rs"),
            &roots
        )
        .is_err());
        assert!(is_path_to_enqueue_valid(
            &PathBuf::from("/workspace/project/.refact/knowledge/note.md"),
            &roots
        )
        .is_ok());
    }
}
