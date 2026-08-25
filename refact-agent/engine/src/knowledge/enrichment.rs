use crate::global_context::GlobalContext;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Component, Path as FilePath, PathBuf};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{Duration, Instant, UNIX_EPOCH};
use regex::Regex;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::sync::watch;

use crate::call_validation::{ChatContent, ChatMessage, ContextFile};
use crate::chat::perf_diagnostics::{self, PerfComponent, PerfOutcome};
use crate::file_filter::KNOWLEDGE_FOLDER_NAME;
use crate::files_correction::get_project_dirs;
use crate::files_in_workspace::get_file_text_from_memory_or_disk;
use crate::knowledge_graph::kg_structs::KnowledgeFrontmatter;
use crate::memories::{enrichment_current_root_id, memories_search_for_enrichment};
use crate::subchat::{resolve_subchat_config, run_subchat};
use crate::yaml_configs::customization_registry::get_subagent_config;

static PATH_IN_CARD_RE: OnceLock<Regex> = OnceLock::new();
static TITLE_IN_CARD_RE: OnceLock<Regex> = OnceLock::new();
static CODE_FENCE_RE: OnceLock<Regex> = OnceLock::new();
static TOOL_PATH_LINE_RE: OnceLock<Regex> = OnceLock::new();
static TITLE_ICON_RE: OnceLock<Regex> = OnceLock::new();
static KIND_ICON_RE: OnceLock<Regex> = OnceLock::new();
static RELATED_BULLET_RE: OnceLock<Regex> = OnceLock::new();
static LINE_RANGE_SUFFIX_RE: OnceLock<Regex> = OnceLock::new();

fn path_in_card_re() -> &'static Regex {
    PATH_IN_CARD_RE.get_or_init(|| Regex::new(r"Memory file: (.+)").unwrap())
}

fn title_in_card_re() -> &'static Regex {
    TITLE_IN_CARD_RE.get_or_init(|| Regex::new(r"(?m)^Title: (.+)$").unwrap())
}

fn code_fence_re() -> &'static Regex {
    CODE_FENCE_RE.get_or_init(|| Regex::new(r"```[\s\S]*?```").unwrap())
}

fn tool_path_line_re() -> &'static Regex {
    TOOL_PATH_LINE_RE.get_or_init(|| Regex::new(r"(?m)^📄\s+(.+)$").unwrap())
}

fn title_icon_re() -> &'static Regex {
    TITLE_ICON_RE.get_or_init(|| Regex::new(r"(?m)^📌\s+(.+)$").unwrap())
}

fn kind_icon_re() -> &'static Regex {
    KIND_ICON_RE.get_or_init(|| Regex::new(r"(?m)^📦\s+(.+)$").unwrap())
}

fn related_bullet_re() -> &'static Regex {
    RELATED_BULLET_RE.get_or_init(|| Regex::new(r"(?m)^-\s+(.+?)\s+\(([^()\n]+)\)\s*$").unwrap())
}

fn line_range_suffix_re() -> &'static Regex {
    LINE_RANGE_SUFFIX_RE
        .get_or_init(|| Regex::new(r"^(?P<path>.+):(?P<line1>\d+)-(?P<line2>\d+)$").unwrap())
}

fn format_enrichment_card(m: &crate::memories::MemoRecord) -> String {
    let mut out = String::new();
    out.push_str("# Related memory (short form)\n");
    out.push_str("Note: this is a heuristic match and may be unrelated to the actual problem.\n\n");
    if let Some(title) = &m.title {
        out.push_str(&format!("Title: {}\n", bounded_card_field(title)));
    }
    if let Some(kind) = &m.kind {
        out.push_str(&format!("Kind: {}\n", bounded_card_field(kind)));
    }
    if let Some(score) = m.score {
        out.push_str(&format!("Relevance: {:.0}%\n", score * 100.0));
    }
    if !m.tags.is_empty() {
        let tags = m
            .tags
            .iter()
            .take(16)
            .map(|tag| bounded_card_field(tag))
            .collect::<Vec<_>>();
        out.push_str(&format!("Tags: {}\n", tags.join(", ")));
    }
    if let Some(path) = &m.file_path {
        out.push_str(&format!("Memory file: {}\n", path.display()));
        out.push_str(&format!(
            "To load full content: call `cat(paths=\"{}\")`\n\n",
            path.display()
        ));
    }
    out.push_str(&m.content);
    out
}

fn bounded_card_field(value: &str) -> String {
    value.chars().take(512).collect()
}

const KNOWLEDGE_TOP_N: usize = 3;
const TRAJECTORY_TOP_N: usize = 2;
const KNOWLEDGE_SCORE_THRESHOLD: f32 = 0.75;
const FORCED_KNOWLEDGE_SCORE_THRESHOLD: f32 = 0.50;
const KNOWLEDGE_ENRICHMENT_MARKER: &str = "knowledge_enrichment";
pub const MAX_QUERY_LENGTH: usize = 2000;
pub const AUTO_ENRICHMENT_TOTAL_TOKEN_CAP: usize = 1600;
pub const AUTO_ENRICHMENT_CARD_TOKEN_CAP: usize = 480;
const MAX_ENRICHMENT_PREVIEW_ITEMS: usize = 5;
const MAX_ENRICHMENT_PREVIEW_CANDIDATES: usize = 64;
const ENRICHMENT_CACHE_MAX_ENTRIES: usize = 128;
const ENRICHMENT_CACHE_MAX_BYTES: usize = 4 * 1024 * 1024;
const ENRICHMENT_CACHE_TTL: Duration = Duration::from_secs(30);
const ENRICHMENT_EMPTY_CACHE_TTL: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct EnrichmentCacheKey {
    query_fingerprint: [u8; 32],
    workspace_scope_fingerprint: [u8; 32],
    allowed_roots_fingerprint: [u8; 32],
    privacy_generation: u64,
    index_generation: u64,
    embedding_config_fingerprint: [u8; 32],
    current_root_fingerprint: [u8; 32],
    top_n_memories: usize,
    top_n_trajectories: usize,
    score_threshold_bits: u32,
}

#[derive(Clone)]
struct MemoSourceFingerprint {
    path: PathBuf,
    size: u64,
    modified_ns: u128,
    content_fingerprint: [u8; 32],
}

#[derive(Clone)]
struct CachedMemo {
    memo: crate::memories::MemoRecord,
    source: MemoSourceFingerprint,
}

#[derive(Clone)]
struct CachedEnrichmentResult {
    memories: Vec<CachedMemo>,
    result_fingerprint: [u8; 32],
    bytes: usize,
    cacheable: bool,
}

impl CachedEnrichmentResult {
    fn empty() -> Self {
        Self {
            memories: Vec::new(),
            result_fingerprint: fingerprint_bytes(&[b"empty-enrichment-result"]),
            bytes: 0,
            cacheable: true,
        }
    }
}

struct EnrichmentCacheEntry {
    result: CachedEnrichmentResult,
    expires_at: Instant,
}

#[derive(Default)]
struct EnrichmentCacheState {
    entries: HashMap<EnrichmentCacheKey, EnrichmentCacheEntry>,
    lru: VecDeque<EnrichmentCacheKey>,
    inflight: HashMap<EnrichmentCacheKey, watch::Sender<bool>>,
    bytes: usize,
}

#[derive(Default)]
pub struct EnrichmentCache {
    state: StdMutex<EnrichmentCacheState>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EnrichmentCacheDisposition {
    Hit,
    Miss,
    Coalesced,
}

impl EnrichmentCache {
    async fn get_or_fetch<F, Fut>(
        &self,
        key: EnrichmentCacheKey,
        fetch: F,
    ) -> Result<(CachedEnrichmentResult, EnrichmentCacheDisposition), String>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<CachedEnrichmentResult, String>>,
    {
        let mut fetch = Some(fetch);
        let mut coalesced = false;
        loop {
            enum Next {
                Hit(CachedEnrichmentResult),
                Wait(watch::Receiver<bool>),
                Fetch,
            }
            let next = {
                let mut state = self
                    .state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                state.purge_expired();
                if let Some(entry) = state.entries.get(&key) {
                    let result = entry.result.clone();
                    state.touch(&key);
                    Next::Hit(result)
                } else if let Some(sender) = state.inflight.get(&key) {
                    Next::Wait(sender.subscribe())
                } else {
                    let (sender, _) = watch::channel(false);
                    state.inflight.insert(key, sender);
                    Next::Fetch
                }
            };

            match next {
                Next::Hit(result) => {
                    return Ok((
                        result,
                        if coalesced {
                            EnrichmentCacheDisposition::Coalesced
                        } else {
                            EnrichmentCacheDisposition::Hit
                        },
                    ));
                }
                Next::Wait(mut receiver) => {
                    coalesced = true;
                    let _ = receiver.changed().await;
                }
                Next::Fetch => {
                    let mut inflight = EnrichmentInFlightGuard {
                        cache: self,
                        key,
                        active: true,
                    };
                    let result = fetch.take().expect("enrichment cache fetch used once")().await;
                    let mut state = self
                        .state
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    let sender = state.inflight.remove(&key);
                    if let Ok(result) = &result {
                        if result.cacheable {
                            state.insert(key, result.clone());
                        }
                    }
                    if let Some(sender) = sender {
                        sender.send_replace(true);
                    }
                    inflight.active = false;
                    return result.map(|result| (result, EnrichmentCacheDisposition::Miss));
                }
            }
        }
    }

    fn invalidate(&self, key: &EnrichmentCacheKey) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(entry) = state.entries.remove(key) {
            state.bytes = state.bytes.saturating_sub(entry.result.bytes);
        }
        state.lru.retain(|cached_key| cached_key != key);
    }
}

struct EnrichmentInFlightGuard<'a> {
    cache: &'a EnrichmentCache,
    key: EnrichmentCacheKey,
    active: bool,
}

impl Drop for EnrichmentInFlightGuard<'_> {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let mut state = self
            .cache
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(sender) = state.inflight.remove(&self.key) {
            sender.send_replace(true);
        }
    }
}

impl EnrichmentCacheState {
    fn purge_expired(&mut self) {
        let now = Instant::now();
        let expired = self
            .entries
            .iter()
            .filter_map(|(key, entry)| (entry.expires_at <= now).then_some(*key))
            .collect::<Vec<_>>();
        for key in expired {
            if let Some(entry) = self.entries.remove(&key) {
                self.bytes = self.bytes.saturating_sub(entry.result.bytes);
            }
        }
        self.lru.retain(|key| self.entries.contains_key(key));
    }

    fn touch(&mut self, key: &EnrichmentCacheKey) {
        self.lru.retain(|cached_key| cached_key != key);
        self.lru.push_back(*key);
    }

    fn insert(&mut self, key: EnrichmentCacheKey, result: CachedEnrichmentResult) {
        if let Some(previous) = self.entries.remove(&key) {
            self.bytes = self.bytes.saturating_sub(previous.result.bytes);
        }
        let ttl = if result.memories.is_empty() {
            ENRICHMENT_EMPTY_CACHE_TTL
        } else {
            ENRICHMENT_CACHE_TTL
        };
        self.bytes = self.bytes.saturating_add(result.bytes);
        self.entries.insert(
            key,
            EnrichmentCacheEntry {
                result,
                expires_at: Instant::now() + ttl,
            },
        );
        self.touch(&key);
        while self.entries.len() > ENRICHMENT_CACHE_MAX_ENTRIES
            || self.bytes > ENRICHMENT_CACHE_MAX_BYTES
        {
            let Some(oldest) = self.lru.pop_front() else {
                break;
            };
            if let Some(entry) = self.entries.remove(&oldest) {
                self.bytes = self.bytes.saturating_sub(entry.result.bytes);
            }
        }
    }
}

fn fingerprint_bytes(parts: &[&[u8]]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part);
    }
    hasher.finalize().into()
}

fn fingerprint_strings<I>(parts: I) -> [u8; 32]
where
    I: IntoIterator<Item = String>,
{
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part.as_bytes());
    }
    hasher.finalize().into()
}

fn fingerprint_hex(fingerprint: &[u8; 32]) -> String {
    hex::encode(fingerprint)
}

pub(crate) fn enrichment_query_fingerprint(query: &str) -> String {
    fingerprint_hex(&fingerprint_bytes(&[query.as_bytes()]))
}

pub(crate) fn enrichment_identity_from_context(message: &ChatMessage) -> Option<String> {
    (message.role == "context_file" && message.tool_call_id == KNOWLEDGE_ENRICHMENT_MARKER)
        .then(|| {
            message
                .extra
                .get("knowledge_enrichment")
                .and_then(|value| value.get("identity"))
                .and_then(|value| value.as_str())
                .map(str::to_string)
        })
        .flatten()
}

pub(crate) fn record_enrichment_identity_on_user_message(
    message: &mut ChatMessage,
    identity: &str,
    query_fingerprint: &str,
) {
    message.extra.insert(
        "knowledge_enrichment".to_string(),
        serde_json::json!({
            "identity": identity,
            "query_fingerprint": query_fingerprint,
        }),
    );
}

pub async fn enrich_messages_with_knowledge(
    gcx: Arc<GlobalContext>,
    messages: &mut Vec<ChatMessage>,
    current_chat_id: Option<&str>,
    force_enrichment: bool,
) {
    let last_user_idx = match messages.iter().rposition(|m| m.role == "user") {
        Some(idx) => idx,
        None => {
            record_enrichment(
                current_chat_id,
                PerfComponent::EnrichmentSkipNoUser,
                PerfOutcome::Skipped,
                0,
                None,
                Some(messages.len() as u64),
                None,
            );
            return;
        }
    };
    let query_raw = messages[last_user_idx].content.content_text_only();

    if has_knowledge_enrichment_near(messages, last_user_idx) {
        record_enrichment(
            current_chat_id,
            PerfComponent::EnrichmentSkipAlreadyPresent,
            PerfOutcome::Skipped,
            0,
            None,
            Some(messages.len() as u64),
            None,
        );
        return;
    }

    let normalize_started = perf_diagnostics::is_enabled().then(Instant::now);
    let query_normalized = normalize_query(&query_raw);
    record_enrichment(
        current_chat_id,
        PerfComponent::EnrichmentQueryNormalize,
        PerfOutcome::Success,
        elapsed_us(normalize_started),
        Some(query_normalized.len() as u64),
        Some(1),
        Some(estimate_tokens(&query_normalized)),
    );

    let decision = should_enrich(messages, &query_raw, &query_normalized, force_enrichment);
    match decision {
        EnrichmentDecision::FirstUser => record_enrichment(
            current_chat_id,
            PerfComponent::EnrichmentDecisionFirstUser,
            PerfOutcome::Success,
            0,
            Some(query_normalized.len() as u64),
            Some(1),
            Some(estimate_tokens(&query_normalized)),
        ),
        EnrichmentDecision::Forced => record_enrichment(
            current_chat_id,
            PerfComponent::EnrichmentDecisionForced,
            PerfOutcome::Success,
            0,
            Some(query_normalized.len() as u64),
            Some(1),
            Some(estimate_tokens(&query_normalized)),
        ),
        EnrichmentDecision::Signaled => record_enrichment(
            current_chat_id,
            PerfComponent::EnrichmentDecisionSignaled,
            PerfOutcome::Success,
            0,
            Some(query_normalized.len() as u64),
            Some(1),
            Some(estimate_tokens(&query_normalized)),
        ),
        EnrichmentDecision::SkipEmpty => record_enrichment(
            current_chat_id,
            PerfComponent::EnrichmentSkipEmptyQuery,
            PerfOutcome::Skipped,
            0,
            Some(query_normalized.len() as u64),
            Some(1),
            None,
        ),
        EnrichmentDecision::SkipCommand => record_enrichment(
            current_chat_id,
            PerfComponent::EnrichmentSkipCommand,
            PerfOutcome::Skipped,
            0,
            Some(query_normalized.len() as u64),
            Some(1),
            None,
        ),
        EnrichmentDecision::SkipThreshold => record_enrichment(
            current_chat_id,
            PerfComponent::EnrichmentSkipThreshold,
            PerfOutcome::Skipped,
            0,
            Some(query_normalized.len() as u64),
            Some(1),
            Some(estimate_tokens(&query_normalized)),
        ),
    }
    if !decision.should_enrich() {
        return;
    }

    record_enrichment(
        current_chat_id,
        PerfComponent::EnrichmentAttempt,
        PerfOutcome::Success,
        0,
        Some(query_normalized.len() as u64),
        Some(1),
        Some(estimate_tokens(&query_normalized)),
    );
    let context_scan_started = perf_diagnostics::is_enabled().then(Instant::now);
    let existing_paths = get_existing_context_file_paths(messages);
    record_enrichment(
        current_chat_id,
        PerfComponent::EnrichmentExistingContextScan,
        PerfOutcome::Success,
        elapsed_us(context_scan_started),
        None,
        Some(existing_paths.len() as u64),
        None,
    );

    let score_threshold = if force_enrichment {
        FORCED_KNOWLEDGE_SCORE_THRESHOLD
    } else {
        KNOWLEDGE_SCORE_THRESHOLD
    };

    if let Some(knowledge_context) = create_knowledge_context(
        gcx,
        &query_normalized,
        &existing_paths,
        current_chat_id,
        score_threshold,
    )
    .await
    {
        messages.insert(last_user_idx, knowledge_context);
        if perf_diagnostics::is_enabled() {
            let (file_count, char_count, estimated_tokens) =
                context_message_stats(&messages[last_user_idx]);
            record_enrichment(
                current_chat_id,
                PerfComponent::EnrichmentInsertion,
                PerfOutcome::Success,
                0,
                Some(char_count),
                Some(file_count),
                Some(estimated_tokens),
            );
        }
        tracing::info!(
            "Injected knowledge context before user message at position {}",
            last_user_idx
        );
    } else {
        record_enrichment(
            current_chat_id,
            PerfComponent::EnrichmentInsertion,
            PerfOutcome::Skipped,
            0,
            None,
            Some(0),
            Some(0),
        );
    }
}

async fn enrichment_cache_key(
    gcx: Arc<GlobalContext>,
    query_text: &str,
    current_chat_id: Option<&str>,
    score_threshold: f32,
) -> EnrichmentCacheKey {
    crate::privacy::load_privacy_if_needed(gcx.clone()).await;
    let project_dirs = get_project_dirs(gcx.clone()).await;
    let mut roots = project_dirs
        .iter()
        .map(|path| crate::files_correction::canonicalize_normalized_path(path.clone()))
        .collect::<Vec<_>>();
    roots.sort();
    roots.dedup();
    let allowed_roots_fingerprint = fingerprint_strings(
        roots
            .iter()
            .map(|root| {
                root.join(KNOWLEDGE_FOLDER_NAME)
                    .to_string_lossy()
                    .into_owned()
            })
            .chain(std::iter::once(
                gcx.config_dir
                    .join("knowledge")
                    .to_string_lossy()
                    .into_owned(),
            )),
    );
    let workspace_scope_fingerprint = fingerprint_strings(
        roots
            .iter()
            .map(|root| root.to_string_lossy().into_owned())
            .chain(std::iter::once(
                gcx.config_dir.to_string_lossy().into_owned(),
            )),
    );
    let current_root = enrichment_current_root_id(gcx.clone(), current_chat_id).await;
    let current_root_fingerprint =
        fingerprint_bytes(&[current_root.as_deref().unwrap_or_default().as_bytes()]);
    let (embedding_config_fingerprint, index_generation) = {
        let vecdb = gcx.vec_db.lock().await.clone();
        match vecdb {
            Some(vecdb) => {
                let (config, splitter_window_size) = vecdb.current_constants();
                let backend_identity = format!("{:p}", Arc::as_ptr(&vecdb));
                let model = fingerprint_strings([
                    backend_identity,
                    config.model_id,
                    config.endpoint,
                    config.endpoint_style,
                    config.embedding_endpoint_style,
                    config.model_name,
                    fingerprint_hex(&fingerprint_bytes(&[config.api_key.as_bytes()])),
                    config.embedding_size.to_string(),
                    config.dimensions.unwrap_or_default().to_string(),
                    config.query_prefix,
                    config.document_prefix,
                    config.rejection_threshold.to_bits().to_string(),
                    config.embedding_batch.to_string(),
                    config.n_ctx.to_string(),
                    splitter_window_size.to_string(),
                ]);
                (
                    model,
                    gcx.enrichment_generation
                        .load(std::sync::atomic::Ordering::Acquire),
                )
            }
            None => (
                fingerprint_bytes(&[b"fallback-enrichment-search"]),
                gcx.enrichment_generation
                    .load(std::sync::atomic::Ordering::Acquire),
            ),
        }
    };
    EnrichmentCacheKey {
        query_fingerprint: fingerprint_bytes(&[query_text.as_bytes()]),
        workspace_scope_fingerprint,
        allowed_roots_fingerprint,
        privacy_generation: gcx
            .tool_catalog_generations
            .privacy
            .load(std::sync::atomic::Ordering::Acquire),
        index_generation,
        embedding_config_fingerprint,
        current_root_fingerprint,
        top_n_memories: KNOWLEDGE_TOP_N,
        top_n_trajectories: TRAJECTORY_TOP_N,
        score_threshold_bits: score_threshold.to_bits(),
    }
}

async fn source_fingerprint(path: &FilePath, content: &str) -> Option<MemoSourceFingerprint> {
    let metadata = tokio::fs::metadata(path).await.ok()?;
    let modified_ns = metadata
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some(MemoSourceFingerprint {
        path: path.to_path_buf(),
        size: metadata.len(),
        modified_ns,
        content_fingerprint: fingerprint_bytes(&[content.as_bytes()]),
    })
}

async fn cache_result_from_memories(
    gcx: Arc<GlobalContext>,
    memories: Vec<crate::memories::MemoRecord>,
) -> CachedEnrichmentResult {
    if memories.is_empty() {
        return CachedEnrichmentResult::empty();
    }
    let mut cached = Vec::with_capacity(memories.len());
    let mut bytes = 0usize;
    for mut memo in memories {
        if memo.kind.as_deref() == Some("trajectory") {
            return CachedEnrichmentResult {
                memories: Vec::new(),
                result_fingerprint: fingerprint_bytes(&[b"uncacheable-enrichment-result"]),
                bytes: 0,
                cacheable: false,
            };
        }
        let Some(path) = memo.file_path.as_ref() else {
            return CachedEnrichmentResult {
                memories: Vec::new(),
                result_fingerprint: fingerprint_bytes(&[b"uncacheable-enrichment-result"]),
                bytes: 0,
                cacheable: false,
            };
        };
        let Some(text) = get_file_text_from_memory_or_disk(gcx.clone(), path)
            .await
            .ok()
        else {
            return CachedEnrichmentResult {
                memories: Vec::new(),
                result_fingerprint: fingerprint_bytes(&[b"uncacheable-enrichment-result"]),
                bytes: 0,
                cacheable: false,
            };
        };
        let Some(source) = source_fingerprint(path, &text).await else {
            return CachedEnrichmentResult {
                memories: Vec::new(),
                result_fingerprint: fingerprint_bytes(&[b"uncacheable-enrichment-result"]),
                bytes: 0,
                cacheable: false,
            };
        };
        bytes = bytes
            .saturating_add(memo.tags.iter().map(String::len).sum::<usize>())
            .saturating_add(path.as_os_str().len());
        memo.content.clear();
        cached.push(CachedMemo { memo, source });
    }
    let result_fingerprint = fingerprint_strings(cached.iter().map(|cached| {
        format!(
            "{}:{}:{}",
            cached.memo.memid,
            cached.source.path.display(),
            fingerprint_hex(&cached.source.content_fingerprint)
        )
    }));
    CachedEnrichmentResult {
        memories: cached,
        result_fingerprint,
        bytes,
        cacheable: true,
    }
}

async fn revalidate_cached_memories(
    gcx: Arc<GlobalContext>,
    cached: &CachedEnrichmentResult,
    score_threshold: f32,
    current_chat_id: Option<&str>,
) -> Option<Vec<crate::memories::MemoRecord>> {
    let current_root = enrichment_current_root_id(gcx.clone(), current_chat_id).await;
    let mut memories = Vec::with_capacity(cached.memories.len());
    for cached_memo in &cached.memories {
        let path = &cached_memo.source.path;
        let text = get_file_text_from_memory_or_disk(gcx.clone(), path)
            .await
            .ok()?;
        let source = source_fingerprint(path, &text).await?;
        if source.size != cached_memo.source.size
            || source.modified_ns != cached_memo.source.modified_ns
            || source.content_fingerprint != cached_memo.source.content_fingerprint
        {
            return None;
        }
        if cached_memo.memo.score.unwrap_or_default() < score_threshold {
            return None;
        }
        let mut memo = cached_memo.memo.clone();
        let (frontmatter, content_start) = KnowledgeFrontmatter::parse(&text);
        if frontmatter.is_archived() || frontmatter.is_deprecated() {
            return None;
        }
        if matches!(
            (frontmatter.source_chat_id.as_deref(), current_root.as_deref()),
            (Some(source_root), Some(current_root)) if source_root == current_root
        ) {
            return None;
        }
        memo.content = text[content_start..].trim().to_string();
        memo.tags = frontmatter.tags;
        memo.title = frontmatter.title;
        memo.created = frontmatter.created;
        memo.kind = frontmatter.kind;
        memories.push(memo);
    }
    Some(memories)
}

fn record_enrichment(
    current_chat_id: Option<&str>,
    component: PerfComponent,
    outcome: PerfOutcome,
    elapsed_us: u64,
    size_bytes: Option<u64>,
    item_count: Option<u64>,
    estimated_tokens: Option<u64>,
) {
    if let Some(chat_id) = current_chat_id {
        perf_diagnostics::record_enrichment(
            component,
            chat_id,
            outcome,
            elapsed_us,
            size_bytes,
            item_count,
            estimated_tokens,
        );
    }
}

fn elapsed_us(started: Option<Instant>) -> u64 {
    started
        .map(|started| started.elapsed().as_micros().try_into().unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn estimate_tokens(text: &str) -> u64 {
    u64::try_from(crate::tokens::count_text_tokens_with_fallback(None, text)).unwrap_or(u64::MAX)
}

fn context_message_stats(message: &ChatMessage) -> (u64, u64, u64) {
    let ChatContent::ContextFiles(files) = &message.content else {
        return (0, 0, 0);
    };
    let char_count = files
        .iter()
        .map(|file| file.file_content.len() as u64)
        .sum::<u64>();
    (
        files.len() as u64,
        char_count,
        files
            .iter()
            .map(|file| estimate_tokens(&file.file_content))
            .sum(),
    )
}

fn normalize_query(query: &str) -> String {
    let normalized = code_fence_re().replace_all(query, " [code] ").to_string();
    let normalized = normalized.trim();
    if normalized.len() > MAX_QUERY_LENGTH {
        normalized.chars().take(MAX_QUERY_LENGTH).collect()
    } else {
        normalized.to_string()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EnrichmentDecision {
    FirstUser,
    Forced,
    Signaled,
    SkipEmpty,
    SkipCommand,
    SkipThreshold,
}

impl EnrichmentDecision {
    fn should_enrich(self) -> bool {
        matches!(self, Self::FirstUser | Self::Forced | Self::Signaled)
    }
}

fn should_enrich(
    messages: &[ChatMessage],
    query_raw: &str,
    query_normalized: &str,
    force_enrichment: bool,
) -> EnrichmentDecision {
    let trimmed = query_raw.trim();
    if trimmed.is_empty() {
        return EnrichmentDecision::SkipEmpty;
    }
    if trimmed.starts_with('@') || trimmed.starts_with('/') {
        return EnrichmentDecision::SkipCommand;
    }
    if force_enrichment {
        tracing::info!("Knowledge enrichment: explicitly enabled for later turn");
        return EnrichmentDecision::Forced;
    }
    let user_message_count = messages.iter().filter(|m| m.role == "user").count();
    if user_message_count == 1 {
        tracing::info!("Knowledge enrichment: first user message");
        return EnrichmentDecision::FirstUser;
    }
    let strong = count_strong_signals(query_raw);
    let weak = count_weak_signals(query_raw, query_normalized);
    if strong >= 1 {
        tracing::info!("Knowledge enrichment: {} strong signal(s)", strong);
        return EnrichmentDecision::Signaled;
    }
    if weak >= 2 && query_normalized.len() >= 20 {
        tracing::info!("Knowledge enrichment: {} weak signal(s)", weak);
        return EnrichmentDecision::Signaled;
    }
    EnrichmentDecision::SkipThreshold
}

fn count_strong_signals(query: &str) -> usize {
    let query_lower = query.to_lowercase();
    let mut count = 0;
    let error_keywords = [
        "error",
        "panic",
        "exception",
        "traceback",
        "stack trace",
        "segfault",
        "failed",
        "unable to",
        "cannot",
        "doesn't work",
        "does not work",
        "broken",
        "bug",
        "crash",
    ];
    if error_keywords.iter().any(|kw| query_lower.contains(kw)) {
        count += 1;
    }
    let file_extensions = [
        ".rs", ".ts", ".tsx", ".js", ".jsx", ".py", ".go", ".java", ".cpp", ".c", ".h",
    ];
    let config_files = [
        "cargo.toml",
        "package.json",
        "tsconfig",
        "pyproject",
        ".yaml",
        ".yml",
        ".toml",
    ];
    if file_extensions.iter().any(|ext| query_lower.contains(ext))
        || config_files.iter().any(|f| query_lower.contains(f))
    {
        count += 1;
    }
    static PATH_RE: OnceLock<Regex> = OnceLock::new();
    let path_re = PATH_RE.get_or_init(|| Regex::new(r"\b[\w-]+/[\w-]+(?:/[\w.-]+)*\b").unwrap());
    if path_re.is_match(query) {
        count += 1;
    }
    if query.contains("::") || query.contains("->") || query.contains("`") {
        count += 1;
    }
    let retrieval_phrases = [
        "search",
        "find",
        "where is",
        "which file",
        "look up",
        "in this repo",
        "in the codebase",
        "in the project",
    ];
    if retrieval_phrases.iter().any(|p| query_lower.contains(p)) {
        count += 1;
    }
    count
}

fn count_weak_signals(query_raw: &str, query_normalized: &str) -> usize {
    let mut count = 0;
    if query_raw.contains('?') {
        count += 1;
    }
    let query_lower = query_raw.trim().to_lowercase();
    let question_starters = [
        "how",
        "why",
        "what",
        "where",
        "when",
        "can",
        "should",
        "could",
        "would",
        "is there",
        "are there",
    ];
    if question_starters.iter().any(|s| query_lower.starts_with(s)) {
        count += 1;
    }
    if query_normalized.len() >= 80 {
        count += 1;
    }
    count
}

async fn create_knowledge_context(
    gcx: Arc<GlobalContext>,
    query_text: &str,
    existing_paths: &HashSet<String>,
    current_chat_id: Option<&str>,
    score_threshold: f32,
) -> Option<ChatMessage> {
    let cache_key =
        enrichment_cache_key(gcx.clone(), query_text, current_chat_id, score_threshold).await;
    let gcx_for_fetch = gcx.clone();
    let query_for_fetch = query_text.to_string();
    let current_chat_for_fetch = current_chat_id.map(str::to_string);
    let (cached, disposition) = gcx
        .enrichment_cache
        .get_or_fetch(cache_key, move || async move {
            let memories = memories_search_for_enrichment(
                gcx_for_fetch.clone(),
                &query_for_fetch,
                KNOWLEDGE_TOP_N,
                TRAJECTORY_TOP_N,
                current_chat_for_fetch.as_deref(),
                current_chat_for_fetch.as_deref(),
            )
            .await?;
            Ok(cache_result_from_memories(gcx_for_fetch, memories).await)
        })
        .await
        .ok()?;
    if enrichment_cache_key(gcx.clone(), query_text, current_chat_id, score_threshold).await
        != cache_key
    {
        return None;
    }
    let memories =
        match revalidate_cached_memories(gcx.clone(), &cached, score_threshold, current_chat_id)
            .await
        {
            Some(memories) => memories,
            None => {
                gcx.enrichment_cache.invalidate(&cache_key);
                return None;
            }
        };
    let cache_component = match disposition {
        EnrichmentCacheDisposition::Miss => PerfComponent::EnrichmentCacheMiss,
        EnrichmentCacheDisposition::Hit => PerfComponent::EnrichmentCacheHit,
        EnrichmentCacheDisposition::Coalesced => PerfComponent::EnrichmentCacheCoalesced,
    };
    record_enrichment(
        current_chat_id,
        cache_component,
        PerfOutcome::Success,
        0,
        Some(cached.bytes as u64),
        Some(cached.memories.len() as u64),
        None,
    );

    let high_score_memories: Vec<_> = memories
        .into_iter()
        .filter(|m| m.score.unwrap_or(0.0) >= score_threshold)
        .filter(|m| {
            if let Some(path) = &m.file_path {
                !existing_paths.contains(&path.to_string_lossy().to_string())
            } else {
                true
            }
        })
        .collect();

    if high_score_memories.is_empty() {
        return None;
    }

    tracing::info!(
        "Knowledge enrichment: {} memories passed threshold {}",
        high_score_memories.len(),
        score_threshold
    );

    let card_started = perf_diagnostics::is_enabled().then(Instant::now);
    let context_files = build_bounded_enrichment_context_files(high_score_memories);

    if context_files.is_empty() {
        return None;
    }

    if perf_diagnostics::is_enabled() {
        let card_chars = context_files
            .iter()
            .map(|file| file.file_content.len() as u64)
            .sum::<u64>();
        record_enrichment(
            current_chat_id,
            PerfComponent::EnrichmentCardBuild,
            PerfOutcome::Success,
            elapsed_us(card_started),
            Some(card_chars),
            Some(context_files.len() as u64),
            Some((card_chars.saturating_add(3)) / 4),
        );
    }

    let query_fingerprint = fingerprint_hex(&cache_key.query_fingerprint);
    let result_fingerprint = fingerprint_hex(&cached.result_fingerprint);
    let index_fingerprint = fingerprint_hex(&fingerprint_bytes(&[
        cache_key.index_generation.to_string().as_bytes(),
        &cache_key.embedding_config_fingerprint,
    ]));
    let identity = format!("{query_fingerprint}:{result_fingerprint}:{index_fingerprint}");
    let mut context = ChatMessage {
        role: "context_file".to_string(),
        content: ChatContent::ContextFiles(context_files),
        tool_call_id: KNOWLEDGE_ENRICHMENT_MARKER.to_string(),
        ..Default::default()
    };
    context.extra.insert(
        "knowledge_enrichment".to_string(),
        serde_json::json!({
            "identity": identity,
            "query_fingerprint": query_fingerprint,
            "result_fingerprint": result_fingerprint,
            "index_fingerprint": index_fingerprint,
        }),
    );
    Some(context)
}

fn build_bounded_enrichment_context_files(
    mut memories: Vec<crate::memories::MemoRecord>,
) -> Vec<ContextFile> {
    memories.sort_by(|a, b| {
        b.score
            .unwrap_or_default()
            .total_cmp(&a.score.unwrap_or_default())
            .then_with(|| a.file_path.cmp(&b.file_path))
            .then_with(|| a.memid.cmp(&b.memid))
    });

    let mut remaining_tokens = AUTO_ENRICHMENT_TOTAL_TOKEN_CAP;
    let mut context_files = Vec::new();
    for memo in memories {
        let Some(file_path) = memo.file_path.as_ref() else {
            continue;
        };
        let card_cap = AUTO_ENRICHMENT_CARD_TOKEN_CAP.min(remaining_tokens);
        let (content, content_truncated) = bounded_enrichment_content(&memo.content, card_cap);
        let mut bounded_memo = memo.clone();
        bounded_memo.content = content;
        let mut card = format_enrichment_card(&bounded_memo);
        if content_truncated {
            card.push_str("\nEnrichment metadata: truncated=true\n");
        }
        let Some(card) = truncate_enrichment_card(&card, card_cap) else {
            continue;
        };
        let card_tokens = estimate_tokens(&card);
        if card_tokens > remaining_tokens as u64 {
            continue;
        }
        remaining_tokens = remaining_tokens.saturating_sub(card_tokens as usize);
        let line_count = card.lines().count().max(1);
        context_files.push(ContextFile {
            file_name: file_path.to_string_lossy().to_string(),
            file_content: card,
            line1: 1,
            line2: line_count,
            file_rev: None,
            symbols: vec![],
            gradient_type: -1,
            usefulness: 80.0 + (memo.score.unwrap_or(0.75) * 20.0),
            skip_pp: true,
        });
        if remaining_tokens == 0 {
            break;
        }
    }
    context_files
}

fn bounded_enrichment_content(content: &str, token_cap: usize) -> (String, bool) {
    let max_chars = token_cap.saturating_mul(4).max(1);
    let mut chars = content.chars();
    let bounded = chars.by_ref().take(max_chars).collect::<String>();
    (bounded, chars.next().is_some())
}

fn truncate_enrichment_card(card: &str, token_cap: usize) -> Option<String> {
    if estimate_tokens(card) <= token_cap as u64 {
        return Some(card.to_string());
    }

    const TRUNCATION_METADATA: &str = "\nEnrichment metadata: truncated=true\n";
    let mut truncated = String::new();
    for line in card.lines() {
        let mut candidate = truncated.clone();
        candidate.push_str(line);
        candidate.push('\n');
        candidate.push_str(TRUNCATION_METADATA);
        if estimate_tokens(&candidate) > token_cap as u64 {
            break;
        }
        truncated.push_str(line);
        truncated.push('\n');
    }
    if truncated.trim().is_empty() {
        return None;
    }
    truncated.push_str(TRUNCATION_METADATA);
    Some(truncated)
}

fn has_knowledge_enrichment_near(messages: &[ChatMessage], user_idx: usize) -> bool {
    let search_start = user_idx.saturating_sub(2);
    let search_end = (user_idx + 2).min(messages.len());
    for i in search_start..search_end {
        if messages[i].role == "context_file"
            && messages[i].tool_call_id == KNOWLEDGE_ENRICHMENT_MARKER
        {
            tracing::info!("Skipping enrichment - already enriched at position {}", i);
            return true;
        }
    }
    false
}

fn get_existing_context_file_paths(messages: &[ChatMessage]) -> HashSet<String> {
    let mut paths = HashSet::new();
    for msg in messages {
        if msg.role == "context_file" {
            let files: Vec<ContextFile> = match &msg.content {
                ChatContent::ContextFiles(files) => files.clone(),
                ChatContent::SimpleText(text) => {
                    serde_json::from_str::<Vec<ContextFile>>(text).unwrap_or_default()
                }
                _ => vec![],
            };
            for file in files {
                paths.insert(file.file_name.clone());
            }
        }
    }
    paths
}

async fn get_allowed_enrichment_dirs(gcx: Arc<GlobalContext>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let config_dir = gcx.config_dir.clone();
    let mut candidates = Vec::new();
    let project_dirs = crate::files_correction::get_project_dirs(gcx.clone()).await;
    for pd in project_dirs {
        candidates.push(pd.join(KNOWLEDGE_FOLDER_NAME));
    }
    candidates.push(config_dir.join("knowledge"));

    let mut seen = HashSet::new();
    for candidate in candidates {
        if let Some(canonical) = canonicalize_allowed_enrichment_root(&candidate).await {
            if seen.insert(canonical.clone()) {
                dirs.push(canonical);
            }
        }
    }

    dirs
}

async fn canonicalize_allowed_enrichment_root(root: &FilePath) -> Option<PathBuf> {
    let metadata = match tokio::fs::symlink_metadata(root).await {
        Ok(metadata) => metadata,
        Err(_) => return None,
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        tracing::warn!(
            "preview: skipping unsafe enrichment root: {}",
            root.display()
        );
        return None;
    }
    let canonical = match tokio::fs::canonicalize(root).await {
        Ok(canonical) => canonical,
        Err(_) => return None,
    };
    Some(dunce::simplified(&canonical).to_path_buf())
}

fn path_has_unsafe_component(path: &FilePath) -> bool {
    path.components()
        .any(|component| matches!(component, Component::ParentDir))
}

fn path_has_markdown_extension(path: &FilePath) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("mdx"))
        .unwrap_or(false)
}

fn workspace_root_for_allowed_knowledge_dir(root: &FilePath) -> Option<PathBuf> {
    let knowledge_folder = FilePath::new(KNOWLEDGE_FOLDER_NAME);
    let knowledge_name = knowledge_folder.file_name()?;
    let refact_name = knowledge_folder.parent()?.file_name()?;
    if root.file_name()? == knowledge_name && root.parent()?.file_name()? == refact_name {
        root.parent()?.parent().map(|path| path.to_path_buf())
    } else {
        None
    }
}

fn candidate_paths_for_enrichment_path(path: &FilePath, allowed_dirs: &[PathBuf]) -> Vec<PathBuf> {
    if path.is_absolute() {
        return vec![path.to_path_buf()];
    }

    let mut candidates = Vec::new();
    for root in allowed_dirs {
        if path.starts_with(KNOWLEDGE_FOLDER_NAME) {
            if let Some(workspace_root) = workspace_root_for_allowed_knowledge_dir(root) {
                candidates.push(workspace_root.join(path));
            }
        } else {
            candidates.push(root.join(path));
        }
    }
    candidates
}

fn canonicalize_enrichment_candidate(raw_path: &str, allowed_dirs: &[PathBuf]) -> Option<PathBuf> {
    let path_str = strip_line_range_suffix(raw_path);
    let path_str = path_str.trim();
    if allowed_dirs.is_empty() || path_str.is_empty() || path_str.contains('\0') {
        return None;
    }

    let path = FilePath::new(path_str);
    if path_has_unsafe_component(path) || !path_has_markdown_extension(path) {
        return None;
    }

    for candidate in candidate_paths_for_enrichment_path(path, allowed_dirs) {
        let canonical = match std::fs::canonicalize(&candidate) {
            Ok(canonical) => dunce::simplified(&canonical).to_path_buf(),
            Err(_) => continue,
        };
        if !std::fs::metadata(&canonical)
            .map(|metadata| metadata.is_file())
            .unwrap_or(false)
        {
            continue;
        }
        if !path_has_markdown_extension(&canonical) {
            continue;
        }
        if allowed_dirs.iter().any(|root| canonical.starts_with(root)) {
            return Some(canonical);
        }
    }

    None
}

fn strip_line_range_suffix(path: &str) -> String {
    let trimmed = path.trim();
    line_range_suffix_re()
        .captures(trimmed)
        .and_then(|caps| caps.name("path").map(|m| m.as_str().trim().to_string()))
        .unwrap_or_else(|| trimmed.to_string())
}

fn title_from_section(section: &str) -> Option<String> {
    title_in_card_re()
        .captures(section)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().trim().to_string())
        .or_else(|| {
            title_icon_re()
                .captures(section)
                .and_then(|c| c.get(1))
                .map(|m| m.as_str().trim().to_string())
        })
}

fn kind_from_section_or_path(section: &str, path_str: &str) -> String {
    let raw_kind = kind_icon_re()
        .captures(section)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().trim().to_lowercase())
        .filter(|kind| !kind.is_empty())
        .unwrap_or_else(|| {
            if path_str.contains("trajectories") {
                "trajectory".to_string()
            } else {
                "memory".to_string()
            }
        });

    match raw_kind.as_str() {
        "trajectory" => "trajectory".to_string(),
        "file" => "file".to_string(),
        _ => "memory".to_string(),
    }
}

fn push_enrichment_item(
    items: &mut Vec<EnrichmentItem>,
    seen_paths: &mut HashSet<String>,
    candidate_attempts: &mut usize,
    allowed_dirs: &[PathBuf],
    raw_path: &str,
    label: Option<String>,
    kind: String,
    content: String,
) {
    if items.len() >= MAX_ENRICHMENT_PREVIEW_ITEMS
        || *candidate_attempts >= MAX_ENRICHMENT_PREVIEW_CANDIDATES
    {
        return;
    }
    *candidate_attempts += 1;

    let path = match canonicalize_enrichment_candidate(raw_path, allowed_dirs) {
        Some(path) => path,
        None => {
            tracing::warn!(
                "preview: skipping enrichment path outside allowed roots: {}",
                strip_line_range_suffix(raw_path)
            );
            return;
        }
    };
    let path_str = path.to_string_lossy().to_string();

    if seen_paths.contains(&path_str) {
        return;
    }

    let label = label
        .filter(|s| !s.trim().is_empty())
        .or_else(|| path.file_stem().map(|s| s.to_string_lossy().to_string()))
        .unwrap_or_else(|| path_str.clone());
    let line_count = content.lines().count().max(1);

    seen_paths.insert(path_str.clone());

    items.push(EnrichmentItem {
        kind,
        label,
        context_file: ContextFile {
            file_name: path_str,
            file_content: content,
            line1: 1,
            line2: line_count,
            file_rev: None,
            symbols: vec![],
            gradient_type: -1,
            usefulness: 85.0,
            skip_pp: true,
        },
    });
}

/// Extract enrichment items from tool result messages produced by the knowledge tool.
/// Content comes directly from tool results (server-generated) — no re-reading from disk.
/// Paths are validated against allowed directories.
fn extract_items_from_tool_results(
    messages: &[ChatMessage],
    allowed_dirs: &[PathBuf],
) -> Vec<EnrichmentItem> {
    let path_re = path_in_card_re();
    let title_re = title_in_card_re();

    let mut items: Vec<EnrichmentItem> = Vec::new();
    let mut seen_paths: HashSet<String> = HashSet::new();
    let mut candidate_attempts = 0usize;

    for msg in messages {
        if items.len() >= MAX_ENRICHMENT_PREVIEW_ITEMS
            || candidate_attempts >= MAX_ENRICHMENT_PREVIEW_CANDIDATES
        {
            break;
        }
        if msg.role != "tool" {
            continue;
        }
        let text = match &msg.content {
            ChatContent::SimpleText(t) => t.as_str(),
            _ => continue,
        };

        for section in text.split("# Related memory").skip(1) {
            if items.len() >= MAX_ENRICHMENT_PREVIEW_ITEMS
                || candidate_attempts >= MAX_ENRICHMENT_PREVIEW_CANDIDATES
            {
                break;
            }
            let path_str = match path_re
                .captures(section)
                .and_then(|c| c.get(1))
                .map(|m| m.as_str().trim().to_string())
            {
                Some(p) if !p.is_empty() => p,
                _ => continue,
            };

            let label = title_re
                .captures(section)
                .and_then(|c| c.get(1))
                .map(|m| m.as_str().trim().to_string())
                .or_else(|| title_from_section(section));

            let card = format!("# Related memory{}", section);
            let kind = kind_from_section_or_path(section, &path_str);
            push_enrichment_item(
                &mut items,
                &mut seen_paths,
                &mut candidate_attempts,
                allowed_dirs,
                &path_str,
                label,
                kind,
                card,
            );
        }

        for section in text.split("\n---\n") {
            if items.len() >= MAX_ENRICHMENT_PREVIEW_ITEMS
                || candidate_attempts >= MAX_ENRICHMENT_PREVIEW_CANDIDATES
            {
                break;
            }
            let path_str = match tool_path_line_re()
                .captures(section)
                .and_then(|c| c.get(1))
                .map(|m| m.as_str().trim().to_string())
            {
                Some(p) if !p.is_empty() => p,
                _ => continue,
            };

            let label = title_from_section(section);
            let kind = kind_from_section_or_path(section, &path_str);
            push_enrichment_item(
                &mut items,
                &mut seen_paths,
                &mut candidate_attempts,
                allowed_dirs,
                &path_str,
                label,
                kind,
                section.trim().to_string(),
            );
        }

        for caps in related_bullet_re().captures_iter(text) {
            if items.len() >= MAX_ENRICHMENT_PREVIEW_ITEMS
                || candidate_attempts >= MAX_ENRICHMENT_PREVIEW_CANDIDATES
            {
                break;
            }
            let label = caps.get(1).map(|m| m.as_str().trim().to_string());
            let path_str = match caps.get(2).map(|m| m.as_str().trim().to_string()) {
                Some(p) if !p.is_empty() => p,
                _ => continue,
            };
            let content = label
                .as_ref()
                .map(|l| {
                    format!(
                        "# Related memory (short form)\nTitle: {}\nMemory file: {}",
                        l, path_str
                    )
                })
                .unwrap_or_else(|| {
                    format!("# Related memory (short form)\nMemory file: {}", path_str)
                });
            let kind = kind_from_section_or_path(text, &path_str);
            push_enrichment_item(
                &mut items,
                &mut seen_paths,
                &mut candidate_attempts,
                allowed_dirs,
                &path_str,
                label,
                kind,
                content,
            );
        }
    }

    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_file(path: &FilePath, content: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn create_file_symlink(target: &FilePath, link: &FilePath) -> bool {
        #[cfg(unix)]
        {
            return std::os::unix::fs::symlink(target, link).is_ok();
        }
        #[cfg(windows)]
        {
            return std::os::windows::fs::symlink_file(target, link).is_ok();
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (target, link);
            false
        }
    }

    fn create_dir_symlink(target: &FilePath, link: &FilePath) -> bool {
        #[cfg(unix)]
        {
            return std::os::unix::fs::symlink(target, link).is_ok();
        }
        #[cfg(windows)]
        {
            return std::os::windows::fs::symlink_dir(target, link).is_ok();
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (target, link);
            false
        }
    }

    fn canonical(path: &FilePath) -> PathBuf {
        dunce::simplified(&fs::canonicalize(path).unwrap()).to_path_buf()
    }

    fn extract_from_single_path(path: &FilePath, allowed_dirs: &[PathBuf]) -> Vec<EnrichmentItem> {
        extract_from_path_str(&path.display().to_string(), allowed_dirs)
    }

    fn extract_from_path_str(path: &str, allowed_dirs: &[PathBuf]) -> Vec<EnrichmentItem> {
        let message = format!(
            "📄 {}:1-3\n📌 Memory Title\n📦 decision\nbody\n\n---\n",
            path
        );
        extract_items_from_tool_results(&[tool_message(&message)], allowed_dirs)
    }

    fn tool_message(content: &str) -> ChatMessage {
        ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn enrichment_decision_matrix_preserves_existing_trigger_rules() {
        let first_turn = vec![ChatMessage::new("user".to_string(), "hello".to_string())];
        assert_eq!(
            should_enrich(&first_turn, "hello", "hello", false),
            EnrichmentDecision::FirstUser
        );
        let later_turn = vec![
            ChatMessage::new("user".to_string(), "hello".to_string()),
            ChatMessage::new("assistant".to_string(), "reply".to_string()),
            ChatMessage::new("user".to_string(), "plain followup".to_string()),
        ];
        assert_eq!(
            should_enrich(&later_turn, "plain followup", "plain followup", false),
            EnrichmentDecision::SkipThreshold
        );
        assert_eq!(
            should_enrich(&later_turn, "", "", false),
            EnrichmentDecision::SkipEmpty
        );
        assert_eq!(
            should_enrich(&later_turn, "/help", "/help", true),
            EnrichmentDecision::SkipCommand
        );
        assert_eq!(
            should_enrich(&later_turn, "plain followup", "plain followup", true),
            EnrichmentDecision::Forced
        );
        assert_eq!(
            should_enrich(
                &later_turn,
                "find src/chat/generation.rs error",
                "find src/chat/generation.rs error",
                false
            ),
            EnrichmentDecision::Signaled
        );
    }

    #[test]
    fn enrichment_context_stats_measure_cards_without_retaining_text() {
        let message = ChatMessage {
            role: "context_file".to_string(),
            content: ChatContent::ContextFiles(vec![ContextFile {
                file_name: "private-memory.md".to_string(),
                file_content: "private enrichment content".to_string(),
                line1: 1,
                line2: 1,
                file_rev: None,
                symbols: Vec::new(),
                gradient_type: -1,
                usefulness: 80.0,
                skip_pp: true,
            }]),
            ..Default::default()
        };

        assert_eq!(
            context_message_stats(&message),
            (
                1,
                "private enrichment content".len() as u64,
                estimate_tokens("private enrichment content")
            )
        );
    }

    fn cache_key(seed: u8) -> EnrichmentCacheKey {
        EnrichmentCacheKey {
            query_fingerprint: [seed; 32],
            workspace_scope_fingerprint: [1; 32],
            allowed_roots_fingerprint: [2; 32],
            privacy_generation: 1,
            index_generation: 1,
            embedding_config_fingerprint: [3; 32],
            current_root_fingerprint: [4; 32],
            top_n_memories: KNOWLEDGE_TOP_N,
            top_n_trajectories: TRAJECTORY_TOP_N,
            score_threshold_bits: KNOWLEDGE_SCORE_THRESHOLD.to_bits(),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn enrichment_cache_coalesces_identical_concurrent_fetches() {
        let cache = Arc::new(EnrichmentCache::default());
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let tasks = (0..100)
            .map(|_| {
                let cache = cache.clone();
                let calls = calls.clone();
                tokio::spawn(async move {
                    cache
                        .get_or_fetch(cache_key(1), move || async move {
                            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            tokio::time::sleep(Duration::from_millis(10)).await;
                            Ok(CachedEnrichmentResult::empty())
                        })
                        .await
                        .unwrap()
                })
            })
            .collect::<Vec<_>>();
        let results = futures::future::join_all(tasks).await;

        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(results
            .into_iter()
            .filter_map(Result::ok)
            .any(|(_, disposition)| disposition == EnrichmentCacheDisposition::Coalesced));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn enrichment_cache_separates_scope_privacy_and_model_identities() {
        let cache = EnrichmentCache::default();
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let base = cache_key(1);
        let mut distinct_scope = base;
        distinct_scope.workspace_scope_fingerprint = [2; 32];
        let mut distinct_privacy = base;
        distinct_privacy.privacy_generation = 2;
        let mut distinct_model = base;
        distinct_model.embedding_config_fingerprint = [4; 32];
        for key in [base, distinct_scope, distinct_privacy, distinct_model] {
            cache
                .get_or_fetch(key, || async {
                    calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Ok(CachedEnrichmentResult::empty())
                })
                .await
                .unwrap();
        }

        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 4);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn enrichment_cache_keeps_empty_results_and_cleans_failures() {
        let cache = EnrichmentCache::default();
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let key = cache_key(7);
        cache
            .get_or_fetch(key, || async {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(CachedEnrichmentResult::empty())
            })
            .await
            .unwrap();
        let (_, disposition) = cache
            .get_or_fetch(key, || async {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(CachedEnrichmentResult::empty())
            })
            .await
            .unwrap();
        assert_eq!(disposition, EnrichmentCacheDisposition::Hit);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);

        let failure_key = cache_key(8);
        assert!(cache
            .get_or_fetch(failure_key, || async { Err("fixture failure".to_string()) })
            .await
            .is_err());
        let (_, disposition) = cache
            .get_or_fetch(failure_key, || async {
                Ok(CachedEnrichmentResult::empty())
            })
            .await
            .unwrap();
        assert_eq!(disposition, EnrichmentCacheDisposition::Miss);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn enrichment_cache_reuses_completed_result_without_refetch() {
        let cache = EnrichmentCache::default();
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let key = cache_key(9);
        let result = CachedEnrichmentResult {
            memories: vec![CachedMemo {
                memo: crate::memories::MemoRecord::default(),
                source: MemoSourceFingerprint {
                    path: PathBuf::from("fixture.md"),
                    size: 0,
                    modified_ns: 0,
                    content_fingerprint: [9; 32],
                },
            }],
            result_fingerprint: [9; 32],
            bytes: 1,
            cacheable: true,
        };
        cache
            .get_or_fetch(key, || async {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(result)
            })
            .await
            .unwrap();
        let (_, disposition) = cache
            .get_or_fetch(key, || async {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(CachedEnrichmentResult::empty())
            })
            .await
            .unwrap();

        assert_eq!(disposition, EnrichmentCacheDisposition::Hit);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn enrichment_cache_evicts_lru_entries_with_bounded_memory() {
        let mut state = EnrichmentCacheState::default();
        for seed in 0..=ENRICHMENT_CACHE_MAX_ENTRIES {
            let mut key = cache_key(1);
            key.privacy_generation = seed as u64;
            state.insert(key, CachedEnrichmentResult::empty());
        }

        assert!(state.entries.len() <= ENRICHMENT_CACHE_MAX_ENTRIES);
        let mut oldest = cache_key(1);
        oldest.privacy_generation = 0;
        assert!(!state.entries.contains_key(&oldest));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cached_memo_revalidation_rejects_changed_deleted_and_archived_sources() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().to_path_buf()];
        let path = dir.path().join("memory.md");
        let frontmatter = crate::memories::create_frontmatter(
            Some("Cached memory"),
            &Vec::new(),
            &Vec::new(),
            &Vec::new(),
            "memory",
        );
        let body = "Initial cached content";
        tokio::fs::write(&path, format!("{}\n\n{}", frontmatter.to_yaml(), body))
            .await
            .unwrap();
        let memo = crate::memories::MemoRecord {
            memid: "cached".to_string(),
            content: body.to_string(),
            file_path: Some(path.clone()),
            score: Some(0.9),
            ..Default::default()
        };
        let cached = cache_result_from_memories(gcx.clone(), vec![memo]).await;
        assert!(cached.cacheable);
        assert_eq!(cached.memories.len(), 1);
        let initial_text = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(
            cached.memories[0].source.content_fingerprint,
            source_fingerprint(&path, &initial_text)
                .await
                .unwrap()
                .content_fingerprint
        );
        assert!(revalidate_cached_memories(gcx.clone(), &cached, 0.75, None)
            .await
            .is_some());

        tokio::fs::write(
            &path,
            format!("{}\n\nChanged content", frontmatter.to_yaml()),
        )
        .await
        .unwrap();
        assert!(revalidate_cached_memories(gcx.clone(), &cached, 0.75, None)
            .await
            .is_none());

        let mut archived = frontmatter.clone();
        archived.status = Some("archived".to_string());
        tokio::fs::write(&path, format!("{}\n\n{}", archived.to_yaml(), body))
            .await
            .unwrap();
        assert!(revalidate_cached_memories(gcx.clone(), &cached, 0.75, None)
            .await
            .is_none());

        tokio::fs::remove_file(&path).await.unwrap();
        assert!(revalidate_cached_memories(gcx, &cached, 0.75, None)
            .await
            .is_none());
    }

    #[test]
    fn enrichment_identity_helpers_only_store_fingerprints() {
        let mut user = ChatMessage::new("user".to_string(), "sensitive query".to_string());
        let query_fingerprint = enrichment_query_fingerprint(&user.content.content_text_only());
        record_enrichment_identity_on_user_message(&mut user, "query:result", &query_fingerprint);

        assert!(!serde_json::Value::Object(user.extra.clone())
            .to_string()
            .contains("sensitive query"));
        assert_eq!(
            user.extra["knowledge_enrichment"]["query_fingerprint"],
            query_fingerprint
        );
    }

    #[test]
    fn bounded_enrichment_cards_preserve_order_and_mark_truncation() {
        let memories = vec![
            crate::memories::MemoRecord {
                memid: "lower".to_string(),
                tags: vec!["knowledge".to_string()],
                content: "lower relevance\n".repeat(1_000),
                file_path: Some(PathBuf::from("/knowledge/lower.md")),
                title: Some("Lower".to_string()),
                created: None,
                kind: Some("memory".to_string()),
                score: Some(0.80),
                line_range: None,
            },
            crate::memories::MemoRecord {
                memid: "higher".to_string(),
                tags: vec!["knowledge".to_string()],
                content: "higher relevance\n".repeat(1_000),
                file_path: Some(PathBuf::from("/knowledge/higher.md")),
                title: Some("Higher".to_string()),
                created: None,
                kind: Some("memory".to_string()),
                score: Some(0.95),
                line_range: None,
            },
        ];

        let files = build_bounded_enrichment_context_files(memories);
        assert_eq!(
            files.first().map(|file| file.file_name.as_str()),
            Some("/knowledge/higher.md")
        );
        assert!(files.iter().all(
            |file| estimate_tokens(&file.file_content) <= AUTO_ENRICHMENT_CARD_TOKEN_CAP as u64
        ));
        assert!(files
            .iter()
            .any(|file| file.file_content.contains("truncated=true")));
        assert!(
            files
                .iter()
                .map(|file| estimate_tokens(&file.file_content))
                .sum::<u64>()
                <= AUTO_ENRICHMENT_TOTAL_TOKEN_CAP as u64
        );
    }

    #[test]
    fn enrichment_card_truncation_keeps_markdown_line_boundaries() {
        let card =
            "# Heading\n\n- café\n- résumé\n\n```rust\nlet value = \"🌻\";\n```\n".repeat(100);
        let truncated =
            truncate_enrichment_card(&card, 120).expect("card should retain a safe prefix");

        assert!(truncated.ends_with("Enrichment metadata: truncated=true\n"));
        assert!(estimate_tokens(&truncated) <= 120);
        assert!(truncated.is_char_boundary(truncated.len()));
    }

    #[test]
    fn extract_items_from_tool_results_parses_knowledge_tool_blocks() {
        let dir = tempfile::tempdir().unwrap();
        let knowledge_dir = dir.path().join(KNOWLEDGE_FOLDER_NAME);
        let path = knowledge_dir.join("memory.md");
        write_file(&path, "memory body");
        let items = extract_from_single_path(&path, &[canonical(&knowledge_dir)]);

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "Memory Title");
        assert_eq!(items[0].kind, "memory");
        assert_eq!(
            items[0].context_file.file_name,
            canonical(&path).display().to_string()
        );
    }

    #[test]
    fn extract_items_from_tool_results_parses_related_memory_bullets() {
        let dir = tempfile::tempdir().unwrap();
        let knowledge_dir = dir.path().join(KNOWLEDGE_FOLDER_NAME);
        let path = knowledge_dir.join("related.md");
        write_file(&path, "related body");
        let messages = vec![tool_message(&format!(
            "## Related memories (short form)\n\n- Related Title ({})\n  short desc\n",
            path.display()
        ))];
        let items = extract_items_from_tool_results(&messages, &[canonical(&knowledge_dir)]);

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "Related Title");
        assert_eq!(
            items[0].context_file.file_name,
            canonical(&path).display().to_string()
        );
    }

    #[test]
    fn extract_items_from_tool_results_rejects_parent_component_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let knowledge_dir = dir.path().join(KNOWLEDGE_FOLDER_NAME);
        write_file(&knowledge_dir.join("allowed.md"), "allowed body");
        write_file(&dir.path().join(".refact/secrets.json"), "{}");
        let traversal = knowledge_dir.join("../secrets.json");

        let items = extract_from_single_path(&traversal, &[canonical(&knowledge_dir)]);

        assert!(items.is_empty());
    }

    #[test]
    fn extract_items_from_tool_results_rejects_config_dir_non_knowledge_file() {
        let dir = tempfile::tempdir().unwrap();
        let config_dir = dir.path().join("config");
        let knowledge_dir = config_dir.join("knowledge");
        let provider_file = config_dir.join("providers.d/provider.md");
        write_file(&knowledge_dir.join("memory.md"), "memory body");
        write_file(&provider_file, "provider body");

        let items = extract_from_single_path(&provider_file, &[canonical(&knowledge_dir)]);

        assert!(items.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn get_allowed_enrichment_dirs_skips_symlinked_roots() {
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("workspace");
        let outside = dir.path().join("outside-knowledge");
        tokio::fs::create_dir_all(workspace.join(".refact"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(&outside).await.unwrap();
        if !create_dir_symlink(&outside, &workspace.join(KNOWLEDGE_FOLDER_NAME)) {
            return;
        }

        let gcx = crate::global_context::tests::make_test_gcx().await;
        {
            *gcx.documents_state.workspace_folders.lock().unwrap() = vec![workspace];
        }

        let allowed_dirs = get_allowed_enrichment_dirs(gcx).await;

        #[cfg(unix)]
        assert!(allowed_dirs.is_empty());
    }

    #[test]
    fn extract_items_from_tool_results_rejects_symlink_escape() {
        let dir = tempfile::tempdir().unwrap();
        let knowledge_dir = dir.path().join(KNOWLEDGE_FOLDER_NAME);
        let outside = dir.path().join("outside.md");
        let link = knowledge_dir.join("link.md");
        write_file(&knowledge_dir.join("memory.md"), "memory body");
        write_file(&outside, "outside body");
        if !create_file_symlink(&outside, &link) {
            return;
        }

        let items = extract_from_single_path(&link, &[canonical(&knowledge_dir)]);

        assert!(items.is_empty());
    }

    #[test]
    fn extract_items_from_tool_results_accepts_valid_canonical_knowledge_doc() {
        let dir = tempfile::tempdir().unwrap();
        let knowledge_dir = dir.path().join(KNOWLEDGE_FOLDER_NAME);
        let path = knowledge_dir.join("valid.md");
        write_file(&path, "valid body");

        let items = extract_from_single_path(&path, &[canonical(&knowledge_dir)]);

        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].context_file.file_name,
            canonical(&path).display().to_string()
        );
    }

    #[test]
    fn extract_items_from_tool_results_accepts_relative_refact_knowledge_doc() {
        let dir = tempfile::tempdir().unwrap();
        let knowledge_dir = dir.path().join(KNOWLEDGE_FOLDER_NAME);
        let path = knowledge_dir.join("relative.md");
        write_file(&path, "relative body");

        let items = extract_from_path_str(
            ".refact/knowledge/relative.md",
            &[canonical(&knowledge_dir)],
        );

        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].context_file.file_name,
            canonical(&path).display().to_string()
        );
    }

    #[test]
    fn extract_items_from_tool_results_accepts_relative_memory_doc_name() {
        let dir = tempfile::tempdir().unwrap();
        let knowledge_dir = dir.path().join(KNOWLEDGE_FOLDER_NAME);
        let path = knowledge_dir.join("nested/relative.mdx");
        write_file(&path, "relative body");

        let items = extract_from_path_str("nested/relative.mdx", &[canonical(&knowledge_dir)]);

        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0].context_file.file_name,
            canonical(&path).display().to_string()
        );
    }

    #[test]
    fn extract_items_from_tool_results_rejects_relative_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let knowledge_dir = dir.path().join(KNOWLEDGE_FOLDER_NAME);
        write_file(&knowledge_dir.join("allowed.md"), "allowed body");
        write_file(&dir.path().join("outside.md"), "outside body");

        let items = extract_from_path_str("../outside.md", &[canonical(&knowledge_dir)]);

        assert!(items.is_empty());
    }

    #[test]
    fn extract_items_from_tool_results_rejects_relative_symlink_escape() {
        let dir = tempfile::tempdir().unwrap();
        let knowledge_dir = dir.path().join(KNOWLEDGE_FOLDER_NAME);
        let outside = dir.path().join("outside.md");
        let link = knowledge_dir.join("link.md");
        write_file(&knowledge_dir.join("memory.md"), "memory body");
        write_file(&outside, "outside body");
        if !create_file_symlink(&outside, &link) {
            return;
        }

        let items = extract_from_path_str("link.md", &[canonical(&knowledge_dir)]);

        assert!(items.is_empty());
    }

    #[test]
    fn extract_items_from_tool_results_is_bounded_and_dedupes() {
        let dir = tempfile::tempdir().unwrap();
        let knowledge_dir = dir.path().join(KNOWLEDGE_FOLDER_NAME);
        for i in 0..10 {
            write_file(&knowledge_dir.join(format!("valid-{i}.md")), "valid body");
        }
        let mut message = String::new();
        for i in 0..70 {
            let path = if i < 60 {
                format!("missing-{i}.md")
            } else {
                format!("valid-{}.md", i - 60)
            };
            message.push_str(&format!("📄 {path}:1-3\n📌 Title {i}\nbody\n\n---\n"));
        }

        let items = extract_items_from_tool_results(
            &[tool_message(&message)],
            &[canonical(&knowledge_dir)],
        );

        assert_eq!(items.len(), 4);
        assert!(items
            .iter()
            .all(|item| item.context_file.file_name.contains("valid-")));

        let mut duplicate_message = String::new();
        for i in 0..10 {
            duplicate_message.push_str(&format!(
                "📄 valid-0.md:1-3\n📌 Duplicate {i}\nbody\n\n---\n"
            ));
        }
        for i in 1..10 {
            duplicate_message
                .push_str(&format!("📄 valid-{i}.md:1-3\n📌 Valid {i}\nbody\n\n---\n"));
        }

        let deduped = extract_items_from_tool_results(
            &[tool_message(&duplicate_message)],
            &[canonical(&knowledge_dir)],
        );

        assert_eq!(deduped.len(), MAX_ENRICHMENT_PREVIEW_ITEMS);
        let unique_paths = deduped
            .iter()
            .map(|item| item.context_file.file_name.clone())
            .collect::<HashSet<_>>();
        assert_eq!(unique_paths.len(), deduped.len());
    }
}

/// A single enrichment item returned to the frontend for wand-preview chip rendering.
#[derive(Serialize)]
pub struct EnrichmentItem {
    pub kind: String,
    pub label: String,
    pub context_file: ContextFile,
}

const ENRICHMENT_SUBAGENT_ID: &str = "memory_enrichment_rewrite";

pub async fn model_gather_and_rewrite(
    gcx: Arc<GlobalContext>,
    query: &str,
) -> Result<(String, Vec<EnrichmentItem>), String> {
    let system_prompt = get_subagent_config(gcx.clone(), ENRICHMENT_SUBAGENT_ID, None)
        .await
        .and_then(|c| c.messages.system_prompt)
        .unwrap_or_else(|| {
            "Search for relevant memories using the knowledge tool, then output JSON: \
            {\"rewritten_text\": \"...\"}"
                .to_string()
        });

    let messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: ChatContent::SimpleText(system_prompt),
            ..Default::default()
        },
        ChatMessage {
            role: "user".to_string(),
            content: ChatContent::SimpleText(query.to_string()),
            ..Default::default()
        },
    ];

    let config = resolve_subchat_config(
        gcx.clone(),
        ENRICHMENT_SUBAGENT_ID,
        false,
        None,
        None,
        None,
        None,
        None,
        Some(vec!["knowledge".to_string()]),
        4,
        false,
        None,
        "agent".to_string(),
    )
    .await
    .map_err(|e| format!("config: {}", e))?;

    let result = run_subchat(gcx.clone(), messages, config)
        .await
        .map_err(|e| format!("subchat: {}", e))?;

    let last_text = result
        .messages
        .iter()
        .rev()
        .find(|m| m.role == "assistant")
        .and_then(|m| match &m.content {
            ChatContent::SimpleText(t) => Some(t.clone()),
            _ => None,
        })
        .unwrap_or_default();

    let rewritten_text = parse_rewritten_text(&last_text);

    let allowed_dirs = get_allowed_enrichment_dirs(gcx.clone()).await;
    let items = extract_items_from_tool_results(&result.messages, &allowed_dirs);

    Ok((rewritten_text, items))
}

fn parse_rewritten_text(text: &str) -> String {
    let stripped = {
        let t = text.trim();
        if t.starts_with("```") {
            let inner: Vec<&str> = t.lines().skip(1).collect();
            let last = inner
                .iter()
                .rposition(|l| l.trim() == "```")
                .unwrap_or(inner.len());
            inner[..last].join("\n")
        } else {
            t.to_string()
        }
    };

    let val = serde_json::from_str::<serde_json::Value>(stripped.trim())
        .or_else(|_| crate::json_utils::extract_json_object(text));

    match val {
        Ok(v) => v
            .get("rewritten_text")
            .and_then(|x| x.as_str())
            .map(|s| s.trim().to_string())
            .unwrap_or_default(),
        Err(_) => String::new(),
    }
}
