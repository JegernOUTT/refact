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
use refact_core::vecdb_types::{FileReader, FileVectorizationGate};

use crate::fetch_embedding::get_embedding_with_retries;
use crate::vdb_markdown_splitter::MarkdownFileSplitter;
use crate::vdb_sqlite::VecDBSqlite;
use crate::vdb_structs::{SimpleTextHashVector, SplitResult, VecDbStatus, VecdbConstants, VecdbRecord};
use crate::vdb_trajectory_splitter::TrajectoryFileSplitter;

const DEBUG_WRITE_VECDB_FILES: bool = false;
const COOLDOWN_SECONDS: u64 = 10;
pub const VECDB_PATH_COALESCING_ENV: &str = "REFACT_VECDB_PATH_COALESCING";

pub fn vecdb_path_coalescing_rollout_enabled() -> bool {
    std::env::var(VECDB_PATH_COALESCING_ENV)
        .ok()
        .is_some_and(|value| {
            let value = value.trim();
            value == "1"
                || value.eq_ignore_ascii_case("true")
                || value.eq_ignore_ascii_case("yes")
                || value.eq_ignore_ascii_case("on")
        })
}

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

fn should_log_skip(skipped_paths: &mut HashSet<PathBuf>, path: &PathBuf) -> bool {
    skipped_paths.insert(path.clone())
}

enum VecdbWork {
    RegularDocument { path: String, generation: u64 },
    ImmediatelyRegularDocument(String),
}

#[derive(Clone, Copy)]
struct PendingRegular {
    updated_at: SystemTime,
    generation: u64,
}

struct QueuedRegular {
    path: String,
    pending: PendingRegular,
}

struct LatestPathQueue {
    coalescing_enabled: bool,
    immediate: VecDeque<String>,
    pending_regular: HashMap<String, PendingRegular>,
    legacy_regular: VecDeque<QueuedRegular>,
    in_flight_regular: HashMap<String, u64>,
    cancelled_regular: HashSet<String>,
}

impl LatestPathQueue {
    fn new(coalescing_enabled: bool) -> Self {
        Self {
            coalescing_enabled,
            immediate: VecDeque::new(),
            pending_regular: HashMap::new(),
            legacy_regular: VecDeque::new(),
            in_flight_regular: HashMap::new(),
            cancelled_regular: HashSet::new(),
        }
    }

    fn enqueue_regular(&mut self, path: String, updated_at: SystemTime) {
        self.cancelled_regular.remove(&path);
        let generation = self
            .pending_regular
            .get(&path)
            .map(|pending| pending.generation)
            .or_else(|| {
                self.legacy_regular
                    .iter()
                    .rev()
                    .find(|pending| pending.path == path)
                    .map(|pending| pending.pending.generation)
            })
            .or_else(|| self.in_flight_regular.get(&path).copied())
            .unwrap_or(0)
            .saturating_add(1);
        let pending = PendingRegular {
            updated_at,
            generation,
        };
        if self.coalescing_enabled {
            self.pending_regular.insert(path, pending);
        } else {
            self.legacy_regular
                .push_back(QueuedRegular { path, pending });
        }
    }

    fn enqueue_immediately(&mut self, path: String) {
        self.immediate.push_back(path);
    }

    fn take_next(&mut self, now: SystemTime) -> Option<VecdbWork> {
        if let Some(path) = self.immediate.pop_front() {
            return Some(VecdbWork::ImmediatelyRegularDocument(path));
        }
        let (path, pending) = if self.coalescing_enabled {
            let path = self
                .pending_regular
                .iter()
                .find(|(_, pending)| {
                    now.duration_since(pending.updated_at)
                        .unwrap_or_default()
                        .as_secs()
                        > COOLDOWN_SECONDS
                })
                .map(|(path, _)| path.clone())?;
            let pending = self.pending_regular.remove(&path)?;
            (path, pending)
        } else {
            let pending = self.legacy_regular.front()?;
            if now
                .duration_since(pending.pending.updated_at)
                .unwrap_or_default()
                .as_secs()
                <= COOLDOWN_SECONDS
            {
                return None;
            }
            let pending = self.legacy_regular.pop_front()?;
            (pending.path, pending.pending)
        };
        self.in_flight_regular
            .insert(path.clone(), pending.generation);
        Some(VecdbWork::RegularDocument {
            path,
            generation: pending.generation,
        })
    }

    fn complete_regular(&mut self, path: &str, generation: u64) {
        if self.in_flight_regular.get(path) == Some(&generation) {
            self.in_flight_regular.remove(path);
            self.cancelled_regular.remove(path);
        }
    }

    fn cancel_regular(&mut self, path: &str) {
        self.pending_regular.remove(path);
        self.legacy_regular.retain(|pending| pending.path != path);
        self.cancelled_regular.insert(path.to_string());
    }

    fn has_newer_regular(&self, path: &str, generation: u64) -> bool {
        self.pending_regular
            .get(path)
            .is_some_and(|pending| pending.generation > generation)
            || self
                .legacy_regular
                .iter()
                .any(|pending| pending.path == path && pending.pending.generation > generation)
    }

    fn is_cancelled_regular(&self, path: &str, generation: u64) -> bool {
        self.cancelled_regular.contains(path)
            && self.in_flight_regular.get(path) == Some(&generation)
    }

    fn should_process_regular(&self, path: &str, generation: u64) -> bool {
        !self.is_cancelled_regular(path, generation)
    }

    fn cancel_all_regular(&mut self) {
        self.pending_regular.clear();
        self.legacy_regular.clear();
        self.in_flight_regular.clear();
        self.cancelled_regular.clear();
    }

    fn unprocessed_len(&self) -> usize {
        self.immediate.len()
            + self.pending_regular.len()
            + self.legacy_regular.len()
            + self.in_flight_regular.len()
    }

    fn is_idle(&self) -> bool {
        self.unprocessed_len() == 0
    }

    #[cfg(test)]
    fn pending_regular_len(&self) -> usize {
        self.pending_regular.len() + self.legacy_regular.len()
    }

    #[cfg(test)]
    fn in_flight_regular_len(&self) -> usize {
        self.in_flight_regular.len()
    }
}

impl Default for LatestPathQueue {
    fn default() -> Self {
        Self::new(vecdb_path_coalescing_rollout_enabled())
    }
}

pub struct FileVectorizerService {
    pub vecdb_handler: Arc<AMutex<VecDBSqlite>>,
    pub vstatus: Arc<AMutex<VecDbStatus>>,
    pub vstatus_notify: Arc<ANotify>,
    constants: VecdbConstants,
    memory_plane_roots: Arc<RwLock<MemoryPlaneRoots>>,
    vecdb_queue: Arc<AMutex<LatestPathQueue>>,
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
        constants.embedding_credential_resolver.as_ref(),
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
    file_vectorization_gate: FileVectorizationGate,
) {
    let mut files_total: usize = 0;
    let mut files_unprocessed: usize;
    let mut reported_unprocessed: usize = 0;
    let mut run_actual_model_on_these: Vec<SplitResult> = vec![];
    let mut ready_to_vecdb: Vec<VecdbRecord> = vec![];

    let (vecdb_queue, constants, memory_plane_roots, vecdb_handler_arc, vstatus, vstatus_notify) = {
        let vservice_locked = vservice.lock().await;
        (
            vservice_locked.vecdb_queue.clone(),
            vservice_locked.constants.clone(),
            vservice_locked.memory_plane_roots.clone(),
            vservice_locked.vecdb_handler.clone(),
            vservice_locked.vstatus.clone(),
            vservice_locked.vstatus_notify.clone(),
        )
    };

    let mut skipped_paths = HashSet::new();
    loop {
        if shutdown_flag.load(std::sync::atomic::Ordering::SeqCst) {
            vecdb_queue.lock().await.cancel_all_regular();
            tracing::info!("VecDB thread: shutdown detected, stopping");
            return;
        }
        let work_on_one: Option<VecdbWork>;
        let current_time = SystemTime::now();
        let mut vstatus_changed = false;
        {
            let mut vecdb_queue_locked = vecdb_queue.lock().await;
            work_on_one = vecdb_queue_locked.take_next(current_time);
            files_unprocessed = vecdb_queue_locked.unprocessed_len()
                + usize::from(matches!(
                    work_on_one.as_ref(),
                    Some(VecdbWork::ImmediatelyRegularDocument(_))
                ));
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
        let (cpath, regular_generation) = {
            match work_on_one.as_ref() {
                Some(VecdbWork::RegularDocument { path, generation }) => {
                    (path.clone(), Some(*generation))
                }
                Some(VecdbWork::ImmediatelyRegularDocument(cpath)) => (cpath.clone(), None),
                None if vecdb_queue.lock().await.is_idle() => {
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
                    skipped_paths.clear();
                    tokio::select! {
                        _ = tokio::time::sleep(tokio::time::Duration::from_millis(1_000)) => {},
                        _ = vstatus_notify.notified() => {},
                    }
                    continue;
                }
                None => {
                    tokio::select! {
                        _ = tokio::time::sleep(tokio::time::Duration::from_secs(1)) => {},
                        _ = vstatus_notify.notified() => {},
                    }
                    continue;
                }
            }
        };

        let last_30_chars = refact_core::custom_error::last_n_chars(&cpath, 30);
        let doc_path: PathBuf = cpath.clone().into();
        if let Some(generation) = regular_generation {
            if !vecdb_queue
                .lock()
                .await
                .should_process_regular(&cpath, generation)
            {
                vecdb_queue
                    .lock()
                    .await
                    .complete_regular(&cpath, generation);
                continue;
            }
        }

        if let Err(reason) = file_vectorization_gate(&doc_path) {
            if should_log_skip(&mut skipped_paths, &doc_path) {
                info!("VecDB skipped {}: {}", doc_path.display(), reason);
            }
            if let Err(err) = vecdb_handler_arc
                .lock()
                .await
                .vecdb_records_remove(vec![cpath.clone()])
                .await
            {
                info!("VECDB Error removing guarded file: {}", err);
            }
            if let Some(generation) = regular_generation {
                let mut queue = vecdb_queue.lock().await;
                if !queue.has_newer_regular(&cpath, generation) {
                    queue.cancel_regular(&cpath);
                }
                queue.complete_regular(&cpath, generation);
            }
            continue;
        }

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
                if let Some(generation) = regular_generation {
                    let mut queue = vecdb_queue.lock().await;
                    if !queue.has_newer_regular(&cpath, generation) {
                        queue.cancel_regular(&cpath);
                    }
                    queue.complete_regular(&cpath, generation);
                }
                continue;
            }
        };

        if let Some(generation) = regular_generation {
            if !vecdb_queue
                .lock()
                .await
                .should_process_regular(&cpath, generation)
            {
                vecdb_queue
                    .lock()
                    .await
                    .complete_regular(&cpath, generation);
                continue;
            }
        }

        let file_kind_result = {
            let roots = memory_plane_roots.read().unwrap();
            memory_plane_file_kind(&doc_path, &roots)
        };
        let file_kind = match file_kind_result {
            Ok(kind) => kind,
            Err(err) => {
                info!("embeddings {} rejected: {}", last_30_chars, err);
                if let Some(generation) = regular_generation {
                    vecdb_queue
                        .lock()
                        .await
                        .complete_regular(&cpath, generation);
                }
                continue;
            }
        };

        let mut doc = Document::new(&doc_path);
        doc.update_text(&text);

        if file_kind == MemoryPlaneFileKind::KnowledgeMarkdown {
            if let Err(err) = doc.does_text_look_good() {
                info!("embeddings {} doesn't look good: {}", last_30_chars, err);
                if let Some(generation) = regular_generation {
                    vecdb_queue
                        .lock()
                        .await
                        .complete_regular(&cpath, generation);
                }
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
        if let Some(generation) = regular_generation {
            vecdb_queue
                .lock()
                .await
                .complete_regular(&cpath, generation);
        }
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
            vecdb_queue: Default::default(),
        }
    }

    pub(crate) async fn cancel_pending_path(&self, path: &PathBuf) {
        self.vecdb_queue
            .lock()
            .await
            .cancel_regular(&path.to_string_lossy());
    }
}

pub async fn vecdb_start_background_tasks(
    vecdb_client: Arc<AMutex<reqwest::Client>>,
    vservice: Arc<AMutex<FileVectorizerService>>,
    shutdown_flag: Arc<AtomicBool>,
    file_reader: FileReader,
    file_vectorization_gate: FileVectorizationGate,
) -> Vec<JoinHandle<()>> {
    let retrieve_thread_handle = tokio::spawn(vectorize_thread(
        vecdb_client.clone(),
        vservice.clone(),
        shutdown_flag,
        file_reader,
        file_vectorization_gate,
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
    let (vecdb_queue, vstatus, vstatus_notify, vecdb_max_files) = {
        let service = vservice.lock().await;
        *service.memory_plane_roots.write().unwrap() = roots.clone();
        (
            service.vecdb_queue.clone(),
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
            let mut vecdb_queue_locked = vecdb_queue.lock().await;
            for doc in documents_my_copy.iter() {
                if process_immediately {
                    vecdb_queue_locked.enqueue_immediately(doc.clone());
                } else {
                    vecdb_queue_locked.enqueue_regular(doc.clone(), SystemTime::now());
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

    #[test]
    fn vecdb_path_coalescing_rollout_switch_defaults_off() {
        assert!(!vecdb_path_coalescing_rollout_enabled());
        for enabled in ["1", "true", "YES", "on"] {
            std::env::set_var(VECDB_PATH_COALESCING_ENV, enabled);
            assert!(vecdb_path_coalescing_rollout_enabled());
        }
        std::env::remove_var(VECDB_PATH_COALESCING_ENV);
    }

    #[test]
    fn legacy_regular_queue_preserves_fifo_duplicates() {
        let mut queue = LatestPathQueue::new(false);
        let path = "/workspace/project/.refact/knowledge/note.md".to_string();
        let expired = SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(COOLDOWN_SECONDS + 1))
            .unwrap();
        queue.enqueue_regular(path.clone(), expired);
        queue.enqueue_regular(path.clone(), expired);
        assert_eq!(queue.pending_regular_len(), 2);
        assert!(
            matches!(queue.take_next(SystemTime::now()), Some(VecdbWork::RegularDocument { path: first, .. }) if first == path)
        );
        assert!(
            matches!(queue.take_next(SystemTime::now()), Some(VecdbWork::RegularDocument { path: second, .. }) if second == path)
        );
    }
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::vdb_emb_aux;

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

    #[test]
    fn regular_queue_coalesces_repeated_pending_paths() {
        let mut queue = LatestPathQueue::new(true);
        let path = "/workspace/project/.refact/knowledge/note.md".to_string();
        let now = SystemTime::now();

        for _ in 0..100 {
            queue.enqueue_regular(path.clone(), now);
        }

        assert_eq!(queue.pending_regular_len(), 1);
        assert_eq!(queue.unprocessed_len(), 1);
    }

    #[test]
    fn regular_queue_schedules_one_follow_up_after_in_flight_update() {
        let mut queue = LatestPathQueue::new(true);
        let path = "/workspace/project/.refact/knowledge/note.md".to_string();
        let expired = SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(COOLDOWN_SECONDS + 1))
            .unwrap();

        queue.enqueue_regular(path.clone(), expired);
        let VecdbWork::RegularDocument {
            path: in_flight_path,
            generation,
        } = queue.take_next(SystemTime::now()).unwrap()
        else {
            panic!("regular document should be selected");
        };
        queue.enqueue_regular(path.clone(), SystemTime::now());

        assert_eq!(queue.pending_regular_len(), 1);
        assert_eq!(queue.in_flight_regular_len(), 1);
        queue.complete_regular(&in_flight_path, generation);
        assert_eq!(queue.pending_regular_len(), 1);
        assert_eq!(queue.in_flight_regular_len(), 0);
    }

    #[test]
    fn regular_queue_cancel_discards_pending_but_not_immediate_requests() {
        let mut queue = LatestPathQueue::new(true);
        let path = "/workspace/project/.refact/knowledge/note.md".to_string();
        let expired = SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(COOLDOWN_SECONDS + 1))
            .unwrap();

        queue.enqueue_regular(path.clone(), expired);
        queue.enqueue_immediately(path.clone());
        queue.cancel_regular(&path);

        assert_eq!(queue.pending_regular_len(), 0);
        assert!(matches!(
            queue.take_next(SystemTime::now()),
            Some(VecdbWork::ImmediatelyRegularDocument(immediate)) if immediate == path
        ));
    }

    #[test]
    fn regular_queue_cancel_marks_in_flight_work_stale() {
        let mut queue = LatestPathQueue::new(true);
        let path = "/workspace/project/.refact/knowledge/note.md".to_string();
        let expired = SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(COOLDOWN_SECONDS + 1))
            .unwrap();

        queue.enqueue_regular(path.clone(), expired);
        let VecdbWork::RegularDocument { generation, .. } =
            queue.take_next(SystemTime::now()).unwrap()
        else {
            panic!("regular document should be selected");
        };
        queue.cancel_regular(&path);

        assert!(!queue.should_process_regular(&path, generation));
        queue.complete_regular(&path, generation);
        assert!(queue.is_idle());
    }

    #[test]
    fn regular_queue_shutdown_clears_pending_and_in_flight_state() {
        let mut queue = LatestPathQueue::new(true);
        let path = "/workspace/project/.refact/knowledge/note.md".to_string();
        let expired = SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(COOLDOWN_SECONDS + 1))
            .unwrap();

        queue.enqueue_regular(path, expired);
        let _ = queue.take_next(SystemTime::now());
        queue.enqueue_regular(
            "/workspace/project/.refact/knowledge/other.md".to_string(),
            SystemTime::now(),
        );
        queue.cancel_all_regular();

        assert_eq!(queue.pending_regular_len(), 0);
        assert_eq!(queue.in_flight_regular_len(), 0);
    }

    #[tokio::test]
    async fn guarded_file_is_skipped_once_while_normal_file_vectorizes() {
        unsafe {
            rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
                sqlite_vec::sqlite3_vec_init as *const (),
            )));
        }
        let root = std::env::temp_dir().join(format!("refact-vecdb-gate-{}", uuid::Uuid::new_v4()));
        let vecdb_dir = root.join("vecdb");
        let guarded = root.join(".refact/knowledge/guarded.md");
        let normal = root.join(".refact/knowledge/normal.md");
        let roots = MemoryPlaneRoots::new(vec![root.clone()], None, None);
        let embedding_requests = Arc::new(AtomicUsize::new(0));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/embeddings", listener.local_addr().unwrap());
        let server_requests = embedding_requests.clone();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            server_requests.fetch_add(1, Ordering::SeqCst);
            let mut request = vec![0u8; 4096];
            let size = tokio::io::AsyncReadExt::read(&mut socket, &mut request)
                .await
                .unwrap();
            let body = String::from_utf8_lossy(&request[..size]);
            assert!(!body.contains("guarded body"));
            assert!(!body.contains("guarded.md"));
            let input_count =
                body.matches("normal body").count() + body.matches("normal.md").count();
            let embeddings = std::iter::repeat("{\"embedding\":[0.25]}")
                .take(input_count)
                .collect::<Vec<_>>()
                .join(",");
            let response_body = format!("{{\"data\":[{embeddings}]}}");
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            tokio::io::AsyncWriteExt::write_all(&mut socket, response.as_bytes())
                .await
                .unwrap();
        });
        let constants = VecdbConstants {
            embedding_model: refact_core::vecdb_types::EmbeddingModelConfig {
                model_id: "embedding/test".to_string(),
                endpoint,
                endpoint_style: "openai".to_string(),
                embedding_endpoint_style: "openai".to_string(),
                api_key: String::new(),
                model_name: "test".to_string(),
                embedding_size: 1,
                dimensions: None,
                query_prefix: String::new(),
                document_prefix: String::new(),
                rejection_threshold: 1.0,
                embedding_batch: 16,
                n_ctx: 64,
            },
            embedding_credential_resolver: None,
            tokenizer: None,
            splitter_window_size: 32,
            vecdb_max_files: 10,
        };
        let handler = Arc::new(AMutex::new(
            VecDBSqlite::init(
                &vecdb_dir,
                &root,
                "test",
                1,
                &vdb_emb_aux::create_emb_table_name(&vec!["workspace".to_string()]),
            )
            .await
            .unwrap(),
        ));
        let service = Arc::new(AMutex::new(
            FileVectorizerService::new(handler.clone(), constants, roots.clone()).await,
        ));
        let reads = Arc::new(AtomicUsize::new(0));
        let file_reads = reads.clone();
        let file_reader: FileReader = Arc::new(move |path| {
            let file_reads = file_reads.clone();
            Box::pin(async move {
                file_reads.fetch_add(1, Ordering::SeqCst);
                if path.file_name().and_then(|name| name.to_str()) == Some("normal.md") {
                    Ok("normal body".to_string())
                } else {
                    Ok("guarded body".to_string())
                }
            })
        });
        let gate_calls = Arc::new(AtomicUsize::new(0));
        let guarded_path = guarded.clone();
        let calls = gate_calls.clone();
        let gate: FileVectorizationGate = Arc::new(move |path| {
            calls.fetch_add(1, Ordering::SeqCst);
            if path == &guarded_path {
                Err("guarded by test policy".to_string())
            } else {
                Ok(())
            }
        });
        let shutdown = Arc::new(AtomicBool::new(false));
        let handles = vecdb_start_background_tasks(
            Arc::new(AMutex::new(reqwest::Client::new())),
            service.clone(),
            shutdown.clone(),
            file_reader,
            gate,
        )
        .await;

        vectorizer_enqueue_files(
            service.clone(),
            &[
                guarded.to_string_lossy().into_owned(),
                normal.to_string_lossy().into_owned(),
            ],
            true,
            &roots,
        )
        .await;
        tokio::time::timeout(tokio::time::Duration::from_secs(5), async {
            loop {
                if service.lock().await.vstatus.lock().await.state == "done" {
                    break;
                }
                tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        shutdown.store(true, Ordering::SeqCst);
        service.lock().await.vstatus_notify.notify_waiters();
        for handle in handles {
            handle.await.unwrap();
        }
        server.await.unwrap();

        assert_eq!(gate_calls.load(Ordering::SeqCst), 2);
        assert_eq!(reads.load(Ordering::SeqCst), 1);
        assert_eq!(embedding_requests.load(Ordering::SeqCst), 1);
        assert!(handler.lock().await.size().await.unwrap() > 0);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn skip_log_is_deduplicated_until_pass_finishes() {
        let path = PathBuf::from("/workspace/project/.refact/knowledge/guarded.md");
        let mut skipped_paths = HashSet::new();

        assert!(should_log_skip(&mut skipped_paths, &path));
        assert!(!should_log_skip(&mut skipped_paths, &path));
        skipped_paths.clear();
        assert!(should_log_skip(&mut skipped_paths, &path));
    }
}
