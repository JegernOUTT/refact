use std::future::Future;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokenizers::Tokenizer;

use crate::call_validation::{ChatContent, ChatMessage, ChatUsage};
use crate::llm::LlmRequest;
use crate::tokens::count_text_tokens_with_fallback;

pub(crate) const DEFAULT_DAILY_TOKENS: u64 = 5_000_000;
pub(crate) const DEFAULT_DAILY_REQUESTS: u64 = 500;
pub(crate) const FRAMING_MARGIN_TOKENS: usize = 1024;
const PER_MESSAGE_OVERHEAD_TOKENS: usize = 8;
const PER_TOOL_OVERHEAD_TOKENS: usize = 16;
const IMAGE_FALLBACK_TOKENS: usize = 1600;
const LEDGER_FILE: &str = "background-llm-budget.json";
const LEDGER_VERSION: u32 = 1;
const LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(10);
pub(crate) const BUDGET_ERROR_PREFIX: &str = "background housekeeping budget refused";

pub(crate) const PRODUCER_MEMO_EXTRACTION: &str = "memo_extraction";
pub(crate) const PRODUCER_KG_ENRICH: &str = "kg_enrich";
pub(crate) const PRODUCER_KG_DEPRECATE: &str = "kg_deprecate";
pub(crate) const PRODUCER_BUDDY_SYSTEM: &str = "buddy_system";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BackgroundScope {
    pub producer: &'static str,
    pub workspace_root: PathBuf,
}

tokio::task_local! {
    static BACKGROUND_SCOPE: BackgroundScope;
}

pub(crate) fn current_scope() -> Option<BackgroundScope> {
    BACKGROUND_SCOPE.try_with(|scope| scope.clone()).ok()
}

pub(crate) fn apply_recovery_policy(
    policy: &mut refact_core::recovery::RecoveryPolicy,
    allow_provider_retries: bool,
) {
    let scoped = current_scope().is_some();
    if !allow_provider_retries || policy.mode == refact_core::recovery::RecoveryMode::Off || scoped
    {
        policy.max_retries = 0;
    }
    if scoped {
        policy.mode = refact_core::recovery::RecoveryMode::Off;
    }
}

pub(crate) async fn with_scope<F: Future>(scope: Option<BackgroundScope>, fut: F) -> F::Output {
    match scope {
        Some(scope) if current_scope().is_none() => BACKGROUND_SCOPE.scope(scope, fut).await,
        _ => fut.await,
    }
}

pub(crate) fn producer_for(
    tool_name: &str,
    buddy_meta: Option<&crate::buddy::types::BuddyThreadMeta>,
) -> Option<&'static str> {
    match tool_name {
        PRODUCER_MEMO_EXTRACTION => return Some(PRODUCER_MEMO_EXTRACTION),
        PRODUCER_KG_ENRICH => return Some(PRODUCER_KG_ENRICH),
        PRODUCER_KG_DEPRECATE => return Some(PRODUCER_KG_DEPRECATE),
        _ => {}
    }
    buddy_meta
        .filter(|meta| meta.is_buddy_chat && meta.buddy_chat_kind == "system")
        .map(|_| PRODUCER_BUDDY_SYSTEM)
}

pub(crate) async fn canonical_root(root: PathBuf) -> Result<PathBuf, String> {
    tokio::task::spawn_blocking(move || dunce::canonicalize(&root))
        .await
        .map_err(|e| budget_error(format!("workspace root task: {e}")))?
        .map_err(|e| budget_error(format!("workspace root is not canonical: {e}")))
}

fn budget_error(detail: impl std::fmt::Display) -> String {
    format!("{BUDGET_ERROR_PREFIX}: {detail} (invalid request, not retried)")
}

fn text_tokens(tokenizer: &Option<Arc<Tokenizer>>, text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    count_text_tokens_with_fallback(tokenizer.clone(), text)
}

fn json_tokens(tokenizer: &Option<Arc<Tokenizer>>, value: &impl Serialize) -> usize {
    match serde_json::to_string(value) {
        Ok(text) => text_tokens(tokenizer, &text),
        Err(_) => usize::MAX,
    }
}

fn content_tokens(tokenizer: &Option<Arc<Tokenizer>>, content: &ChatContent) -> usize {
    match content {
        ChatContent::SimpleText(text) => text_tokens(tokenizer, text),
        ChatContent::Multimodal(elements) => elements
            .iter()
            .map(|element| {
                if element.is_image() {
                    refact_core::chat_types::calculate_image_tokens_openai(
                        &element.m_content,
                        "high",
                    )
                    .ok()
                    .and_then(|tokens| usize::try_from(tokens).ok())
                    .unwrap_or(0)
                    .max(IMAGE_FALLBACK_TOKENS)
                } else {
                    text_tokens(tokenizer, &element.m_content)
                }
            })
            .fold(0usize, usize::saturating_add),
        ChatContent::ContextFiles(files) => files
            .iter()
            .map(|file| {
                text_tokens(tokenizer, &file.file_name)
                    .saturating_add(text_tokens(tokenizer, &file.file_content))
            })
            .fold(0usize, usize::saturating_add),
    }
}

fn message_tokens(tokenizer: &Option<Arc<Tokenizer>>, message: &ChatMessage) -> usize {
    let mut parts = vec![
        PER_MESSAGE_OVERHEAD_TOKENS,
        text_tokens(tokenizer, &message.role),
        content_tokens(tokenizer, &message.content),
        text_tokens(tokenizer, &message.tool_call_id),
    ];
    if let Some(reasoning) = &message.reasoning_content {
        parts.push(text_tokens(tokenizer, reasoning));
    }
    for call in message.tool_calls.iter().flatten() {
        parts.push(PER_MESSAGE_OVERHEAD_TOKENS);
        parts.push(text_tokens(tokenizer, &call.id));
        parts.push(text_tokens(tokenizer, &call.function.name));
        parts.push(text_tokens(tokenizer, &call.function.arguments));
        if let Some(extra) = &call.extra_content {
            parts.push(json_tokens(tokenizer, extra));
        }
    }
    if let Some(blocks) = &message.thinking_blocks {
        parts.push(json_tokens(tokenizer, blocks));
    }
    if !message.server_content_blocks.is_empty() {
        parts.push(json_tokens(tokenizer, &message.server_content_blocks));
    }
    if !message.citations.is_empty() {
        parts.push(json_tokens(tokenizer, &message.citations));
    }
    parts.into_iter().fold(0usize, usize::saturating_add)
}

pub(crate) fn estimate_request_input_tokens(
    tokenizer: &Option<Arc<Tokenizer>>,
    request: &LlmRequest,
) -> usize {
    let messages: usize = request
        .messages
        .iter()
        .map(|message| message_tokens(tokenizer, message))
        .fold(0usize, usize::saturating_add);
    let tools: usize = request
        .tools
        .iter()
        .flatten()
        .map(|tool| PER_TOOL_OVERHEAD_TOKENS.saturating_add(json_tokens(tokenizer, tool)))
        .fold(0usize, usize::saturating_add);
    messages
        .saturating_add(tools)
        .saturating_add(FRAMING_MARGIN_TOKENS)
}

pub(crate) fn check_envelope(
    input_tokens: usize,
    max_output_tokens: usize,
    window: usize,
) -> Result<u64, String> {
    let envelope = input_tokens.saturating_add(max_output_tokens);
    if envelope > window {
        return Err(budget_error(format!(
            "estimated envelope {input_tokens} input + {max_output_tokens} output = {envelope} is above the {window} window for automatic housekeeping"
        )));
    }
    Ok(envelope as u64)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DailyLimits {
    pub tokens: u64,
    pub requests: u64,
}

pub(crate) fn effective_limits(user_daily_tokens: Option<u64>) -> DailyLimits {
    let tokens = match user_daily_tokens {
        Some(limit) if limit > 0 => limit.min(DEFAULT_DAILY_TOKENS),
        _ => DEFAULT_DAILY_TOKENS,
    };
    DailyLimits {
        tokens,
        requests: DEFAULT_DAILY_REQUESTS,
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct Ledger {
    pub version: u32,
    pub day: String,
    pub tokens: u64,
    pub requests: u64,
}

#[derive(Debug)]
pub(crate) struct Reservation {
    ledger_dir: PathBuf,
    day: String,
    reserved: u64,
}

pub(crate) fn ledger_dir(workspace_root: &Path) -> PathBuf {
    workspace_root.join(".refact").join("buddy")
}

pub(crate) fn utc_day() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

fn with_locked_ledger<T>(
    dir: &Path,
    update: impl FnOnce(&mut Ledger) -> Result<(T, bool), String>,
) -> Result<T, String> {
    std::fs::create_dir_all(dir).map_err(|e| budget_error(format!("ledger directory: {e}")))?;
    let path = dir.join(LEDGER_FILE);
    let lock_path = dir.join(format!("{LEDGER_FILE}.lock"));
    let lock_file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|e| budget_error(format!("ledger lock: {e}")))?;
    let mut lock = fd_lock::RwLock::new(lock_file);
    let deadline = std::time::Instant::now() + LOCK_WAIT;
    loop {
        match lock.try_write() {
            Ok(guard) => {
                let result = update_ledger(dir, &path, update);
                drop(guard);
                return result;
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if std::time::Instant::now() >= deadline {
                    return Err(budget_error("ledger lock is busy"));
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(e) => return Err(budget_error(format!("ledger lock: {e}"))),
        }
    }
}

fn update_ledger<T>(
    dir: &Path,
    path: &Path,
    update: impl FnOnce(&mut Ledger) -> Result<(T, bool), String>,
) -> Result<T, String> {
    let mut ledger = match std::fs::File::open(path) {
        Ok(mut file) => {
            let mut raw = String::new();
            file.seek(SeekFrom::Start(0))
                .and_then(|_| file.read_to_string(&mut raw))
                .map_err(|e| budget_error(format!("ledger read: {e}")))?;
            let ledger: Ledger = serde_json::from_str(&raw)
                .map_err(|_| budget_error("ledger is corrupt; repair or remove it explicitly"))?;
            if ledger.version != LEDGER_VERSION {
                return Err(budget_error("ledger version is not recognised"));
            }
            let day_valid = if ledger.day.is_empty() {
                ledger.tokens == 0 && ledger.requests == 0
            } else {
                chrono::NaiveDate::parse_from_str(&ledger.day, "%Y-%m-%d").is_ok()
            };
            if !day_valid {
                return Err(budget_error(
                    "ledger is corrupt; repair or remove it explicitly",
                ));
            }
            ledger
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ledger {
            version: LEDGER_VERSION,
            ..Default::default()
        },
        Err(e) => return Err(budget_error(format!("ledger read: {e}"))),
    };
    let (result, dirty) = update(&mut ledger)?;
    if dirty {
        let tmp = dir.join(format!(
            ".{LEDGER_FILE}.{}.tmp",
            uuid::Uuid::new_v4().simple()
        ));
        let write = || -> std::io::Result<()> {
            let mut file = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&tmp)?;
            file.write_all(&serde_json::to_vec(&ledger)?)?;
            file.sync_all()?;
            std::fs::rename(&tmp, path)
        };
        if let Err(e) = write() {
            let _ = std::fs::remove_file(&tmp);
            return Err(budget_error(format!("ledger write: {e}")));
        }
        #[cfg(unix)]
        std::fs::File::open(dir)
            .and_then(|handle| handle.sync_all())
            .map_err(|e| budget_error(format!("ledger directory sync: {e}")))?;
    }
    Ok(result)
}

pub(crate) fn reserve_blocking(
    dir: &Path,
    day: &str,
    limits: DailyLimits,
    tokens: u64,
) -> Result<Reservation, String> {
    with_locked_ledger(dir, |ledger| {
        if ledger.day.as_str() > day {
            return Err(budget_error(
                "ledger day is ahead of the current UTC day; refusing until the clock catches up",
            ));
        }
        if ledger.day != day {
            ledger.day = day.to_string();
            ledger.tokens = 0;
            ledger.requests = 0;
        }
        if ledger.requests.saturating_add(1) > limits.requests {
            return Err(budget_error(format!(
                "daily request allowance of {} is spent",
                limits.requests
            )));
        }
        if ledger.tokens.saturating_add(tokens) > limits.tokens {
            return Err(budget_error(format!(
                "daily token allowance of {} cannot cover a {tokens} token reservation ({} used)",
                limits.tokens, ledger.tokens
            )));
        }
        ledger.requests += 1;
        ledger.tokens += tokens;
        Ok((
            Reservation {
                ledger_dir: dir.to_path_buf(),
                day: day.to_string(),
                reserved: tokens,
            },
            true,
        ))
    })
}

pub(crate) fn usage_charge(usage: &ChatUsage) -> u64 {
    let components = usage
        .prompt_tokens
        .saturating_add(usage.completion_tokens)
        .saturating_add(usage.cache_read_tokens.unwrap_or(0))
        .saturating_add(usage.cache_creation_tokens.unwrap_or(0));
    usage.total_tokens.max(components) as u64
}

pub(crate) fn settle_blocking(
    reservation: &Reservation,
    actual: Option<&ChatUsage>,
    extra_sends: u64,
) -> Result<(), String> {
    let charge = actual.map(usage_charge).filter(|charge| *charge > 0);
    if charge.is_none() && extra_sends == 0 {
        return Ok(());
    }
    with_locked_ledger(&reservation.ledger_dir, |ledger| {
        if ledger.day != reservation.day {
            return Ok(((), false));
        }
        let extra_tokens = reservation.reserved.saturating_mul(extra_sends);
        let base = match charge {
            Some(charge) => ledger
                .tokens
                .saturating_sub(reservation.reserved)
                .saturating_add(charge),
            None => ledger.tokens,
        };
        ledger.tokens = base.saturating_add(extra_tokens);
        ledger.requests = ledger.requests.saturating_add(extra_sends);
        Ok(((), true))
    })
}

pub(crate) fn user_daily_tokens(buddy_dir: &Path) -> Result<Option<u64>, String> {
    match std::fs::read(buddy_dir.join("settings.json")) {
        Ok(raw) => serde_json::from_slice::<refact_buddy_core::settings::BuddySettings>(&raw)
            .map(|settings| settings.daily_llm_token_budget)
            .map_err(|_| budget_error("buddy settings are unreadable; refusing to widen the cap")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(budget_error(format!("buddy settings read: {e}"))),
    }
}

pub(crate) async fn reserve(scope: &BackgroundScope, tokens: u64) -> Result<Reservation, String> {
    let dir = ledger_dir(&scope.workspace_root);
    let day = utc_day();
    tokio::task::spawn_blocking(move || {
        let limits = effective_limits(user_daily_tokens(&dir)?);
        reserve_blocking(&dir, &day, limits, tokens)
    })
    .await
    .map_err(|e| budget_error(format!("ledger task: {e}")))?
}

pub(crate) async fn settle(reservation: Reservation, actual: Option<ChatUsage>, extra_sends: u64) {
    let result = tokio::task::spawn_blocking(move || {
        settle_blocking(&reservation, actual.as_ref(), extra_sends)
    })
    .await;
    if let Ok(Err(error)) | Err(error) = result.map_err(|e| e.to_string()) {
        tracing::warn!("background budget settlement kept the reservation: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits(tokens: u64, requests: u64) -> DailyLimits {
        DailyLimits {
            tokens,
            requests,
        }
    }

    fn read(dir: &Path) -> Ledger {
        serde_json::from_slice(&std::fs::read(dir.join(LEDGER_FILE)).unwrap()).unwrap()
    }

    #[test]
    fn user_settings_lower_cap_and_corrupt_settings_refuse() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(user_daily_tokens(dir.path()).unwrap(), None);
        let settings = refact_buddy_core::settings::BuddySettings {
            daily_llm_token_budget: Some(1234),
            ..Default::default()
        };
        std::fs::write(
            dir.path().join("settings.json"),
            serde_json::to_vec(&settings).unwrap(),
        )
        .unwrap();
        assert_eq!(user_daily_tokens(dir.path()).unwrap(), Some(1234));
        std::fs::write(dir.path().join("settings.json"), b"{broken").unwrap();
        assert!(user_daily_tokens(dir.path()).is_err());
    }

    #[tokio::test]
    async fn scoped_recovery_cannot_be_widened_and_foreground_keeps_retries() {
        use refact_core::recovery::{RecoveryBudget, RecoveryMode, RecoveryPolicy};
        let widened = RecoveryPolicy {
            mode: RecoveryMode::Transient,
            max_retries: 100,
            ..Default::default()
        };
        let mut foreground = widened.clone();
        apply_recovery_policy(&mut foreground, true);
        assert_eq!(foreground.max_retries, 100);
        assert_eq!(foreground.mode, RecoveryMode::Transient);

        let root = tempfile::tempdir().unwrap();
        let scope = BackgroundScope {
            producer: PRODUCER_MEMO_EXTRACTION,
            workspace_root: root.path().to_path_buf(),
        };
        with_scope(Some(scope), async {
            let mut background = widened;
            apply_recovery_policy(&mut background, true);
            assert_eq!(background.max_retries, 0);
            assert_eq!(background.mode, RecoveryMode::Off);
            let budget = RecoveryBudget::new(background);
            assert!(budget.reserve_send().is_ok());
            assert!(budget.reserve_send().is_err());
            assert_eq!(budget.sends_used(), 1);
        })
        .await;
        assert!(current_scope().is_none());
    }

    #[test]
    fn default_and_user_caps_only_lower() {
        assert_eq!(effective_limits(None).tokens, DEFAULT_DAILY_TOKENS);
        assert_eq!(effective_limits(Some(0)).tokens, DEFAULT_DAILY_TOKENS);
        assert_eq!(effective_limits(Some(1_000)).tokens, 1_000);
        assert_eq!(
            effective_limits(Some(u64::MAX)).tokens,
            DEFAULT_DAILY_TOKENS
        );
        assert_eq!(effective_limits(None).requests, DEFAULT_DAILY_REQUESTS);
    }

    #[test]
    fn envelope_is_fail_closed_and_not_retryable() {
        assert_eq!(check_envelope(1_000, 500, 1_500).unwrap(), 1_500);
        let error = check_envelope(1_001, 500, 1_500).unwrap_err();
        assert!(error.starts_with(BUDGET_ERROR_PREFIX));
        let decision = crate::chat::retry_policy::classify_llm_error_for_retry(&error);
        assert!(!decision.is_retryable_transient() && !decision.is_context_limit());
    }

    #[test]
    fn estimate_counts_system_tools_thinking_and_arguments() {
        let base = LlmRequest::new(
            "m".into(),
            vec![ChatMessage::new("user".into(), "hi".into())],
        );
        let small = estimate_request_input_tokens(&None, &base);
        assert!(small >= FRAMING_MARGIN_TOKENS);
        let mut big = base.clone();
        big.messages
            .insert(0, ChatMessage::new("system".into(), "x".repeat(7_000)));
        let mut assistant = ChatMessage::new("assistant".into(), String::new());
        assistant.thinking_blocks = Some(vec![
            serde_json::json!({"thinking": "y".repeat(700), "signature": "s"}),
        ]);
        assistant.tool_calls = Some(vec![crate::call_validation::ChatToolCall {
            id: "c".into(),
            index: None,
            function: refact_core::chat_types::ChatToolFunction {
                arguments: "z".repeat(700),
                name: "cat".into(),
            },
            tool_type: "function".into(),
            extra_content: None,
            started_at_ms: None,
            completed_at_ms: None,
        }]);
        big.messages.push(assistant);
        big.tools = Some(vec![
            serde_json::json!({"name": "t", "description": "d".repeat(700)}),
        ]);
        assert!(estimate_request_input_tokens(&None, &big) > small + 2_000 + 600);
    }

    #[test]
    fn reservation_is_persisted_before_settlement_and_unknown_usage_keeps_it() {
        let dir = tempfile::tempdir().unwrap();
        let reservation =
            reserve_blocking(dir.path(), "2030-01-01", limits(10_000, 10), 3_000).unwrap();
        assert_eq!(
            read(dir.path()),
            Ledger {
                version: 1,
                day: "2030-01-01".into(),
                tokens: 3_000,
                requests: 1
            }
        );
        settle_blocking(&reservation, None, 0).unwrap();
        settle_blocking(&reservation, Some(&ChatUsage::default()), 0).unwrap();
        assert_eq!(read(dir.path()).tokens, 3_000);
    }

    #[test]
    fn actual_usage_settles_cache_aware() {
        let dir = tempfile::tempdir().unwrap();
        let reservation =
            reserve_blocking(dir.path(), "2030-01-01", limits(10_000, 10), 3_000).unwrap();
        let usage = ChatUsage {
            prompt_tokens: 100,
            completion_tokens: 50,
            total_tokens: 150,
            cache_read_tokens: Some(400),
            cache_creation_tokens: Some(25),
            ..Default::default()
        };
        assert_eq!(usage_charge(&usage), 575);
        settle_blocking(&reservation, Some(&usage), 0).unwrap();
        assert_eq!(read(dir.path()).tokens, 575);
        assert_eq!(read(dir.path()).requests, 1);
    }

    #[test]
    fn token_and_request_caps_refuse() {
        let dir = tempfile::tempdir().unwrap();
        reserve_blocking(dir.path(), "2030-01-01", limits(1_000, 2), 600).unwrap();
        assert!(reserve_blocking(dir.path(), "2030-01-01", limits(1_000, 2), 600).is_err());
        reserve_blocking(dir.path(), "2030-01-01", limits(1_000, 2), 400).unwrap();
        assert!(reserve_blocking(dir.path(), "2030-01-01", limits(1_000_000, 2), 1).is_err());
        assert_eq!(read(dir.path()).requests, 2);
    }

    #[test]
    fn rollover_never_refunds_new_day_and_clock_rollback_refuses() {
        let dir = tempfile::tempdir().unwrap();
        let old = reserve_blocking(dir.path(), "2030-01-01", limits(1_000, 5), 900).unwrap();
        reserve_blocking(dir.path(), "2030-01-02", limits(1_000, 5), 800).unwrap();
        settle_blocking(
            &old,
            Some(&ChatUsage {
                total_tokens: 10,
                ..Default::default()
            }),
            0,
        )
        .unwrap();
        assert_eq!(read(dir.path()).tokens, 800);
        assert!(reserve_blocking(dir.path(), "2030-01-01", limits(1_000, 5), 1).is_err());
        assert_eq!(read(dir.path()).day, "2030-01-02");
    }

    #[test]
    fn corrupt_ledger_refuses_without_reset() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(LEDGER_FILE), b"{not json").unwrap();
        let error = reserve_blocking(dir.path(), "2030-01-01", limits(1_000, 5), 1).unwrap_err();
        assert!(error.contains("corrupt"));
        assert_eq!(
            std::fs::read(dir.path().join(LEDGER_FILE)).unwrap(),
            b"{not json"
        );
    }

    #[test]
    fn concurrent_reservations_share_only_capacity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        let handles: Vec<_> = (0..16)
            .map(|_| {
                let path = path.clone();
                std::thread::spawn(move || {
                    reserve_blocking(&path, "2030-01-01", limits(1_000, 100), 100).is_ok()
                })
            })
            .collect();
        let ok = handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .filter(|ok| *ok)
            .count();
        assert_eq!(ok, 10);
        assert_eq!(
            read(dir.path()),
            Ledger {
                version: 1,
                day: "2030-01-01".into(),
                tokens: 1_000,
                requests: 10
            }
        );
    }

    #[test]
    fn only_trusted_housekeepers_are_scoped() {
        assert_eq!(
            producer_for("memo_extraction", None),
            Some(PRODUCER_MEMO_EXTRACTION)
        );
        assert_eq!(producer_for("kg_enrich", None), Some(PRODUCER_KG_ENRICH));
        assert_eq!(
            producer_for("kg_deprecate", None),
            Some(PRODUCER_KG_DEPRECATE)
        );
        assert_eq!(producer_for("subagent", None), None);
        assert_eq!(producer_for("title_generation", None), None);
        let mut meta = crate::buddy::types::BuddyThreadMeta {
            is_buddy_chat: true,
            buddy_chat_kind: "system".into(),
            workflow_id: None,
            source_chat_id: None,
            source_project: None,
            source_created_at: None,
        };
        assert_eq!(
            producer_for("subagent", Some(&meta)),
            Some(PRODUCER_BUDDY_SYSTEM)
        );
        meta.buddy_chat_kind = "user".into();
        assert_eq!(producer_for("subagent", Some(&meta)), None);
    }

    async fn gated_stream(
        chat_id: &str,
        scope: Option<BackgroundScope>,
        max_new_tokens: usize,
    ) -> (Result<Vec<Vec<ChatMessage>>, String>, usize) {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.privacy_policy_load.write().unwrap() = refact_privacy::PolicyLoad::default();
        let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
        let mut caps = crate::caps::CodeAssistantCaps::default();
        caps.chat_models.insert(
            "model".into(),
            Arc::new(crate::caps::ChatModelRecord {
                base: crate::caps::BaseModelRecord {
                    id: "model".into(),
                    tokenizer: "fake".into(),
                    n_ctx: 4096,
                    ..Default::default()
                },
                ..Default::default()
            }),
        );
        {
            let mut state = app.model.caps.write().await;
            state.caps = Some(Arc::new(caps));
            state.last_attempted_ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
        }
        let ccx = Arc::new(tokio::sync::Mutex::new(
            crate::at_commands::at_commands::AtCommandsContext::new(
                gcx,
                4096,
                1,
                false,
                vec![],
                chat_id.to_string(),
                None,
                "model".to_string(),
                None,
                None,
            )
            .await,
        ));
        let mut fixture = super::super::SubchatModelFixture::default();
        fixture.responses.push_back(super::super::ChoiceFinal {
            content: "done".into(),
            finish_reason: Some("stop".into()),
            usage: Some(ChatUsage {
                prompt_tokens: 40,
                completion_tokens: 2,
                total_tokens: 42,
                ..Default::default()
            }),
            ..Default::default()
        });
        super::super::SUBCHAT_MODEL_FIXTURES
            .get_or_init(Default::default)
            .lock()
            .unwrap()
            .insert(chat_id.to_string(), fixture);
        let result = with_scope(
            scope,
            super::super::subchat_stream(
                ccx,
                "model",
                "agent",
                vec![ChatMessage::new("user".into(), "hello".into())],
                vec![],
                false,
                None,
                max_new_tokens,
                None,
                crate::llm::params::CacheControl::Ephemeral,
                false,
                None,
                false,
            ),
        )
        .await;
        let calls = super::super::SUBCHAT_MODEL_FIXTURES
            .get()
            .unwrap()
            .lock()
            .unwrap()
            .remove(chat_id)
            .unwrap()
            .requests
            .len();
        (result, calls)
    }

    #[tokio::test]
    async fn oversized_background_envelope_blocks_fixture_and_small_request_settles() {
        let root = tempfile::tempdir().unwrap();
        let scope = BackgroundScope {
            producer: PRODUCER_MEMO_EXTRACTION,
            workspace_root: canonical_root(root.path().to_path_buf()).await.unwrap(),
        };
        let (result, calls) = gated_stream("bg-budget-oversize", Some(scope.clone()), 3_500).await;
        assert!(result.unwrap_err().starts_with(BUDGET_ERROR_PREFIX));
        assert_eq!(calls, 0);
        assert!(!ledger_dir(&scope.workspace_root).join(LEDGER_FILE).exists());

        let (result, calls) = gated_stream("bg-budget-small", Some(scope.clone()), 256).await;
        assert!(result.is_ok());
        assert_eq!(calls, 1);
        let ledger = read(&ledger_dir(&scope.workspace_root));
        assert_eq!((ledger.tokens, ledger.requests), (42, 1));

        let (result, calls) = gated_stream("bg-budget-foreground", None, 3_500).await;
        assert!(result.is_ok(), "foreground keeps existing behaviour");
        assert_eq!(calls, 1);
        assert_eq!(read(&ledger_dir(&scope.workspace_root)).requests, 1);
    }
}
