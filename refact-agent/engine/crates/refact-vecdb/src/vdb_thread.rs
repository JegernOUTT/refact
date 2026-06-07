use indexmap::IndexMap;
use std::collections::HashSet;
use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::ops::Div;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::SystemTime;
use tokio::sync::{Mutex as AMutex, Notify as ANotify};
use tokio::task::JoinHandle;
use tracing::{info, warn};

use refact_ast::Document;
use refact_core::vecdb_types::FileReader;

use crate::ast_file_splitter::AstBasedFileSplitter;
use crate::fetch_embedding::get_embedding_with_retries;
use crate::vdb_markdown_splitter::MarkdownFileSplitter;
use crate::vdb_sqlite::VecDBSqlite;
use crate::vdb_structs::{SimpleTextHashVector, SplitResult, VecDbStatus, VecdbConstants, VecdbRecord};
use crate::vdb_trajectory_splitter::{TrajectoryFileSplitter, is_trajectory_file};

const DEBUG_WRITE_VECDB_FILES: bool = false;
const COOLDOWN_SECONDS: u64 = 10;

const SOURCE_FILE_EXTENSIONS: &[&str] = &[
    "c", "cpp", "cc", "h", "hpp", "cs", "java", "py", "rb", "go", "rs", "ts", "tsx", "js", "jsx",
    "php", "swift", "kt", "kts", "scala", "r", "m", "mm", "pl", "lua", "sh", "bash", "sql", "html",
    "css", "md", "mdx", "json", "yaml", "yml", "toml", "xml",
];

fn is_path_to_enqueue_valid(path: &PathBuf) -> Result<(), String> {
    let extension = path.extension().unwrap_or_default();
    if !SOURCE_FILE_EXTENSIONS.contains(&extension.to_str().unwrap_or_default()) {
        return Err(format!("Unsupported file extension {:?}", extension));
    }
    Ok(())
}

fn document_cache_hash(constants: &VecdbConstants, text: &str) -> String {
    let embedding_model = &constants.embedding_model;
    let cache_input = format!(
        "model={}\nsize={}\ndimensions={:?}\nstyle={}\nemb_style={}\ndoc_prefix={}\n---\n{}",
        embedding_model.model_name,
        embedding_model.embedding_size,
        embedding_model.dimensions,
        embedding_model.endpoint_style,
        embedding_model.embedding_endpoint_style,
        embedding_model.document_prefix,
        text
    );
    refact_ast::ast::chunk_utils::official_text_hashing_function(&cache_input)
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
    let B = constants.embedding_model.embedding_batch.max(1);
    let batch = run_actual_model_on_these
        .drain(..B.min(run_actual_model_on_these.len()))
        .collect::<Vec<_>>();
    assert!(batch.len() > 0);

    let embedding_inputs = batch
        .iter()
        .map(|x| constants.embedding_model.prefixed_document(&x.window_text))
        .collect();
    let batch_result = match get_embedding_with_retries(
        client.clone(),
        &constants.embedding_model,
        embedding_inputs,
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
            window_text_hash: document_cache_hash(constants, &data_res.window_text),
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
    constants: &VecdbConstants,
) {
    while !splits.is_empty() {
        let batch: Vec<SplitResult> = splits
            .drain(..group_size.min(splits.len()))
            .collect::<Vec<_>>();
        let cache_lookup = batch
            .iter()
            .map(|split| {
                let mut split = split.clone();
                split.window_text_hash = document_cache_hash(constants, &split.window_text);
                split
            })
            .collect();
        let vectors_maybe = vecdb_handler_arc
            .lock()
            .await
            .fetch_vectors_from_cache(&cache_lookup)
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

    let (vecdb_todo, constants, vecdb_handler_arc, vstatus, vstatus_notify) = {
        let vservice_locked = vservice.lock().await;
        (
            vservice_locked.vecdb_todo.clone(),
            vservice_locked.constants.clone(),
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
            let embedding_batch = constants.embedding_model.embedding_batch.max(1);
            if run_actual_model_on_these.len() > 0 && flush
                || run_actual_model_on_these.len() >= embedding_batch
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

        let mut doc = Document::new(&doc_path);
        doc.update_text(&text);

        let is_trajectory = is_trajectory_file(&doc.doc_path);
        if !is_trajectory {
            if let Err(err) = doc.does_text_look_good() {
                info!("embeddings {} doesn't look good: {}", last_30_chars, err);
                continue;
            }
        }

        let is_markdown = doc
            .doc_path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .map(|e| e == "md" || e == "mdx")
            .unwrap_or(false);

        let mut splits = if is_trajectory {
            let traj_splitter = TrajectoryFileSplitter::new(constants.splitter_window_size);
            traj_splitter
                .split(&text, &doc.doc_path)
                .await
                .unwrap_or_else(|err| {
                    info!("{}", err);
                    vec![]
                })
        } else if is_markdown {
            let md_splitter = MarkdownFileSplitter::new(constants.embedding_model.n_ctx);
            md_splitter
                .split(&text, &doc.doc_path)
                .await
                .unwrap_or_else(|err| {
                    info!("{}", err);
                    vec![]
                })
        } else {
            let file_splitter = AstBasedFileSplitter::new(constants.splitter_window_size);
            file_splitter
                .vectorization_split(&text, &doc.doc_path, None, constants.embedding_model.n_ctx)
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
                    window_text_hash: refact_ast::ast::chunk_utils::official_text_hashing_function(
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
            &constants,
        )
        .await;
    }
}

impl FileVectorizerService {
    pub async fn new(vecdb_handler: Arc<AMutex<VecDBSqlite>>, constants: VecdbConstants) -> Self {
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

fn _filter_docs_to_enqueue(docs: &[String]) -> Vec<String> {
    let mut rejected_reasons = HashMap::new();
    let mut filtered_docs = vec![];
    for d in docs {
        let path: PathBuf = d.clone().into();
        match is_path_to_enqueue_valid(&path) {
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
) {
    enqueue_files_impl(vservice, documents, process_immediately).await;
}

async fn enqueue_files_impl(
    vservice: Arc<AMutex<FileVectorizerService>>,
    documents: &[String],
    process_immediately: bool,
) -> usize {
    if documents.is_empty() {
        return 0;
    }
    info!("adding {} files", documents.len());
    let documents = _filter_docs_to_enqueue(documents);
    let (vecdb_todo, vstatus, vstatus_notify, vecdb_max_files) = {
        let service = vservice.lock().await;
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
    documents_my_copy.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vdb_structs::EmbeddingModelConfig;

    fn test_constants(vecdb_max_files: usize) -> VecdbConstants {
        VecdbConstants {
            embedding_model: EmbeddingModelConfig {
                endpoint: "test://record-only".to_string(),
                endpoint_style: "openai".to_string(),
                embedding_endpoint_style: "openai".to_string(),
                api_key: String::new(),
                auth_token: String::new(),
                extra_headers: Default::default(),
                model_name: "test-embedding".to_string(),
                embedding_size: 2,
                dimensions: None,
                query_prefix: String::new(),
                document_prefix: "doc: ".to_string(),
                rejection_threshold: 0.63,
                embedding_batch: 0,
                n_ctx: 128,
            },
            tokenizer: None,
            splitter_window_size: 128,
            vecdb_max_files,
        }
    }

    #[test]
    fn document_cache_hash_includes_prefix_identity() {
        let mut constants = test_constants(10);
        let without_prefix = document_cache_hash(&constants, "same text");
        constants.embedding_model.document_prefix = "passage: ".to_string();
        let with_prefix = document_cache_hash(&constants, "same text");

        assert_ne!(without_prefix, with_prefix);
    }

    #[tokio::test]
    async fn vectorize_batch_uses_document_prefix_and_handles_zero_batch() {
        let constants = test_constants(10);
        let tmp =
            std::env::temp_dir().join(format!("refact-vecdb-thread-{}", uuid::Uuid::new_v4()));
        let legacy = tmp.join("legacy");
        let handler = VecDBSqlite::init(&tmp, &legacy, "test-embedding", 2, "vecdb_thread_test")
            .await
            .unwrap();
        let handler = Arc::new(AMutex::new(handler));
        let vstatus = Arc::new(AMutex::new(VecDbStatus {
            files_unprocessed: 0,
            files_total: 0,
            requests_made_since_start: 0,
            vectors_made_since_start: 0,
            db_size: 0,
            db_cache_size: 0,
            state: "testing".to_string(),
            queue_additions: false,
            vecdb_max_files_hit: false,
            vecdb_errors: IndexMap::new(),
        }));
        let mut run = vec![SplitResult {
            file_path: PathBuf::from("/tmp/a.rs"),
            window_text: "fn main() {}".to_string(),
            window_text_hash: "legacy".to_string(),
            start_line: 1,
            end_line: 1,
            symbol_path: String::new(),
        }];
        let mut ready = vec![];

        vectorize_batch_from_q(
            &mut run,
            &mut ready,
            vstatus,
            Arc::new(AMutex::new(reqwest::Client::new())),
            &constants,
            handler,
        )
        .await
        .unwrap();

        assert!(run.is_empty());
        assert_eq!(ready.len(), 1);
        assert_eq!(
            crate::fetch_embedding::take_last_embedding_inputs(),
            vec!["doc: fn main() {}".to_string()]
        );
        let _ = std::fs::remove_dir_all(tmp);
    }

    #[tokio::test]
    async fn enqueue_files_applies_vecdb_max_files_truncation() {
        let tmp =
            std::env::temp_dir().join(format!("refact-vecdb-enqueue-{}", uuid::Uuid::new_v4()));
        let legacy = tmp.join("legacy");
        let handler = VecDBSqlite::init(&tmp, &legacy, "test-embedding", 2, "vecdb_enqueue_test")
            .await
            .unwrap();
        let service = Arc::new(AMutex::new(
            FileVectorizerService::new(Arc::new(AMutex::new(handler)), test_constants(2)).await,
        ));
        let docs = vec![
            "/tmp/a.rs".to_string(),
            "/tmp/b.rs".to_string(),
            "/tmp/c.rs".to_string(),
        ];

        let enqueued = enqueue_files_impl(service.clone(), &docs, false).await;

        assert_eq!(enqueued, 2);
        let service_locked = service.lock().await;
        assert!(service_locked.vstatus.lock().await.vecdb_max_files_hit);
        assert_eq!(service_locked.vecdb_todo.lock().await.len(), 2);
        let _ = std::fs::remove_dir_all(tmp);
    }
}
