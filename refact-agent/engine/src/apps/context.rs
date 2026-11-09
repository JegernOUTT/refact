use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
use refact_privacy::FileRecord;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::app_state::AppState;

pub const MAX_TEXT_CHARS: usize = 8192;
pub const MAX_LABEL_CHARS: usize = 80;
pub const MAX_IDS_PER_MESSAGE: usize = 8;
const PREVIEW_CHARS: usize = 280;
const TTL: Duration = Duration::from_secs(600);
/// A claim pins the ticket while its message waits in the chat queue.
const CLAIMED_TTL: Duration = Duration::from_secs(3600);
const MAX_LIVE_PER_CHAT: usize = 16;
const MAX_TOTAL: usize = 512;

#[derive(Clone, Debug)]
pub struct Ticket {
    pub app_id: Uuid,
    pub view_id: Uuid,
    pub chat_id: String,
    pub binding_id: Uuid,
    pub revision: String,
    pub label: String,
    pub text: String,
    pub captured_at: String,
    pub records: Vec<FileRecord>,
}

struct Entry {
    ticket: Ticket,
    expires: Instant,
    claim: Option<String>,
    consumed: bool,
}

#[derive(Default)]
pub struct TicketStore {
    entries: HashMap<String, Entry>,
}

impl TicketStore {
    fn prune(&mut self, now: Instant) {
        self.entries.retain(|_, e| e.expires > now);
    }

    fn extend_claimed(entry: &mut Entry, now: Instant) {
        entry.expires = now + CLAIMED_TTL;
    }

    pub fn insert(&mut self, ticket: Ticket, now: Instant) -> Result<String, String> {
        self.prune(now);
        if self.entries.len() >= MAX_TOTAL {
            return Err("app context capacity reached".into());
        }
        let live = self
            .entries
            .values()
            .filter(|e| !e.consumed && e.ticket.chat_id == ticket.chat_id)
            .count();
        if live >= MAX_LIVE_PER_CHAT {
            return Err("too many staged app contexts for this chat".into());
        }
        let id = Uuid::new_v4().to_string();
        self.entries.insert(
            id.clone(),
            Entry {
                ticket,
                expires: now + TTL,
                claim: None,
                consumed: false,
            },
        );
        Ok(id)
    }

    fn usable<'a>(
        entries: &'a mut HashMap<String, Entry>,
        chat_id: &str,
        request_id: &str,
        id: &str,
    ) -> Result<&'a mut Entry, String> {
        let entry = entries
            .get_mut(id)
            .ok_or("app context expired or unavailable")?;
        if entry.ticket.chat_id != chat_id {
            return Err("app context expired or unavailable".into());
        }
        if entry.consumed {
            return Err("app context already consumed".into());
        }
        if entry.claim.as_deref().is_some_and(|c| c != request_id) {
            return Err("app context is claimed by another message".into());
        }
        Ok(entry)
    }

    /// All-or-nothing claim at enqueue time; re-claiming by the same request is idempotent.
    pub fn claim(
        &mut self,
        chat_id: &str,
        request_id: &str,
        ids: &[String],
        now: Instant,
    ) -> Result<(), String> {
        self.prune(now);
        check_ids(ids)?;
        for id in ids {
            Self::usable(&mut self.entries, chat_id, request_id, id)?;
        }
        for id in ids {
            let entry = self.entries.get_mut(id).unwrap();
            entry.claim = Some(request_id.to_string());
            Self::extend_claimed(entry, now);
        }
        Ok(())
    }

    /// Tickets claimed by `request_id`, in request order; nothing is consumed.
    pub fn resolve(
        &mut self,
        chat_id: &str,
        request_id: &str,
        ids: &[String],
        now: Instant,
    ) -> Result<Vec<(String, Ticket)>, String> {
        self.prune(now);
        check_ids(ids)?;
        ids.iter()
            .map(|id| {
                let entry = Self::usable(&mut self.entries, chat_id, request_id, id)?;
                if entry.claim.is_none() {
                    return Err("app context was not claimed by this message".into());
                }
                Ok((id.clone(), entry.ticket.clone()))
            })
            .collect()
    }

    /// Marks claimed tickets consumed once the message is appended; text is dropped, tombstone stays until TTL.
    pub fn commit(&mut self, chat_id: &str, request_id: &str) {
        for entry in self.entries.values_mut() {
            if entry.ticket.chat_id == chat_id
                && entry.claim.as_deref() == Some(request_id)
                && !entry.consumed
            {
                entry.consumed = true;
                entry.ticket.text.clear();
                entry.ticket.records.clear();
            }
        }
    }

    /// Returns claimed but unconsumed tickets to the staged pool after a rejected send.
    pub fn release(&mut self, chat_id: &str, request_id: &str) {
        for entry in self.entries.values_mut() {
            if entry.ticket.chat_id == chat_id
                && entry.claim.as_deref() == Some(request_id)
                && !entry.consumed
            {
                entry.claim = None;
                entry.expires = entry.expires.min(Instant::now() + TTL);
            }
        }
    }
}

pub fn check_ids(ids: &[String]) -> Result<(), String> {
    if ids.len() > MAX_IDS_PER_MESSAGE {
        return Err("too many app contexts in one message".into());
    }
    let mut seen = std::collections::HashSet::new();
    for id in ids {
        if Uuid::parse_str(id).is_err() || !seen.insert(id.as_str()) {
            return Err("invalid app context id".into());
        }
    }
    Ok(())
}

pub fn tickets() -> &'static Mutex<TicketStore> {
    static STORE: OnceLock<Mutex<TicketStore>> = OnceLock::new();
    STORE.get_or_init(Default::default)
}

pub fn store() -> std::sync::MutexGuard<'static, TicketStore> {
    tickets().lock().unwrap_or_else(|p| p.into_inner())
}

pub fn sanitize_label(label: Option<String>) -> String {
    let label: String = label
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_LABEL_CHARS)
        .collect();
    let label = label.trim();
    if label.is_empty() {
        "App context".into()
    } else {
        label.into()
    }
}

/// Declared app bundle + state files and the bound chat's carried privacy records.
/// Process sources contribute observed records; unknown provenance refuses.
pub async fn known_privacy_records(
    app: &AppState,
    runtime: &super::AppRuntime,
    app_id: Uuid,
    chat_id: Option<&str>,
    binding_id: Option<Uuid>,
) -> Result<Vec<FileRecord>, String> {
    let record = runtime.store.get_by_id(app_id).await?;
    let bundle = runtime.store.published(&record.slug).await?;
    let process_records = runtime
        .process_sources
        .known_records(&app_id.to_string())
        .await
        .map_err(|e| format!("process source provenance unavailable: {e}"))?;
    let dir = runtime.root.join(".refact/apps").join(&record.slug);
    let mut paths: Vec<PathBuf> = record
        .files
        .iter()
        .chain(bundle.files.keys())
        .map(|f| dir.join(f))
        .collect();
    paths.push(dir.join(".local/state/shared.json"));
    paths.push(dir.join(".local/state/standalone.json"));
    if let Some(binding_id) = binding_id {
        paths.push(dir.join(format!(".local/state/binding-{binding_id}.json")));
    }
    let mut records = crate::privacy::records::declared_file_records(&app.gcx, paths)?;
    for r in process_records {
        if !records.contains(&r) {
            records.push(r);
        }
    }
    if let Some(chat_id) = chat_id {
        let session = app
            .chat
            .sessions
            .read()
            .await
            .get(chat_id)
            .cloned()
            .ok_or("bound chat unavailable")?;
        let messages = session.lock().await.messages.clone();
        let carried = crate::privacy::records::records_to_carry(&messages)
            .map_err(|e| format!("bound chat provenance unavailable: {}", e.message))?;
        for r in carried {
            if !records.contains(&r) {
                records.push(r);
            }
        }
    }
    Ok(records)
}

/// Diagnostics/events can include every binding of an app, not merely a state
/// operation's explicit scope. Resolve all provenance before reading runtime data.
pub async fn bound_privacy_records(
    app: &AppState,
    runtime: &super::AppRuntime,
    chat_id: &str,
    only_app: Option<Uuid>,
) -> Result<Vec<FileRecord>, String> {
    let session = app
        .chat
        .sessions
        .read()
        .await
        .get(chat_id)
        .cloned()
        .ok_or("caller unavailable")?;
    let bindings = {
        let session = session.lock().await;
        let workspace = session
            .thread
            .app_workspace
            .as_ref()
            .and_then(|w| w.supported())
            .ok_or("caller has no supported app workspace")?;
        workspace.validate()?;
        workspace
            .bindings
            .iter()
            .filter(|b| only_app.is_none_or(|id| id == b.app_id))
            .map(|b| (b.app_id, b.binding_id))
            .collect::<Vec<_>>()
    };
    let mut records = Vec::new();
    for (app_id, binding_id) in bindings {
        for record in
            known_privacy_records(app, runtime, app_id, Some(chat_id), Some(binding_id)).await?
        {
            if !records.contains(&record) {
                records.push(record);
            }
        }
    }
    Ok(records)
}

pub async fn stage(
    app: &AppState,
    runtime: &super::AppRuntime,
    view_id: Uuid,
    view: &super::views::View,
    text: String,
    label: Option<String>,
) -> Result<Value, String> {
    let (Some(chat_id), Some(binding_id)) = (view.chat_id.clone(), view.binding_id) else {
        return Err("app context requires a chat-bound view".into());
    };
    if text.trim().is_empty() {
        return Err("app context text is empty".into());
    }
    if text.chars().count() > MAX_TEXT_CHARS {
        return Err("app context text exceeds limit".into());
    }
    super::validate_binding(app, runtime, view.app_id, Some(&chat_id), Some(binding_id)).await?;
    let record = runtime.store.get_by_id(view.app_id).await?;
    if runtime.store.published(&record.slug).await?.revision != view.revision {
        return Err("app revision changed; reload the view".into());
    }
    let records =
        known_privacy_records(app, runtime, view.app_id, Some(&chat_id), Some(binding_id)).await?;
    let label = sanitize_label(label);
    let captured_at = chrono::Utc::now().to_rfc3339();
    let preview: String = text.chars().take(PREVIEW_CHARS).collect();
    let source_count = records.len();
    let context_id = store().insert(
        Ticket {
            app_id: view.app_id,
            view_id,
            chat_id,
            binding_id,
            revision: view.revision.clone(),
            label: label.clone(),
            text: text.clone(),
            captured_at: captured_at.clone(),
            records,
        },
        Instant::now(),
    )?;
    Ok(json!({
        "context_id": context_id,
        "label": label,
        "preview": preview,
        "text": text,
        "captured_at": captured_at,
        "expires_in_ms": TTL.as_millis() as u64,
        "app_id": view.app_id,
        "binding_id": binding_id,
        "revision": view.revision,
        "source_count": source_count,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_privacy::Attribution;

    #[tokio::test]
    async fn diagnostics_provenance_includes_exact_binding_state_file() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().into()];
        let app = AppState::from_gcx(gcx).await;
        let runtime = super::super::runtime(&app, &super::super::Caller::Host)
            .await
            .unwrap();
        let record = runtime
            .store
            .create("provenance", "Provenance")
            .await
            .unwrap();
        runtime
            .store
            .publish("provenance", &record.revision)
            .await
            .unwrap();
        let binding = Uuid::new_v4();
        let records =
            known_privacy_records(&app, &runtime, record.manifest.app_id, None, Some(binding))
                .await
                .unwrap();
        let suffix = format!(".local/state/binding-{binding}.json");
        assert!(
            records.iter().any(|record| record.path.ends_with(&suffix)),
            "binding state omitted from diagnostic/event provenance"
        );
        app.gcx.apps.shutdown().await;
    }

    fn ticket(chat: &str) -> Ticket {
        Ticket {
            app_id: Uuid::new_v4(),
            view_id: Uuid::new_v4(),
            chat_id: chat.into(),
            binding_id: Uuid::new_v4(),
            revision: "r".into(),
            label: "l".into(),
            text: "secret".into(),
            captured_at: "t".into(),
            records: vec![FileRecord {
                path: "/p/.env".into(),
                zone: "sensitive".into(),
                attribution: Attribution::Declared,
            }],
        }
    }

    #[test]
    fn claim_is_owned_idempotent_and_single_use() {
        let mut s = TicketStore::default();
        let now = Instant::now();
        let id = s.insert(ticket("c1"), now).unwrap();
        let ids = vec![id.clone()];
        assert!(s.claim("other-chat", "r1", &ids, now).is_err());
        assert!(
            s.resolve("c1", "r1", &ids, now).is_err(),
            "unclaimed must not resolve"
        );
        s.claim("c1", "r1", &ids, now).unwrap();
        s.claim("c1", "r1", &ids, now).unwrap();
        assert!(s.claim("c1", "r2", &ids, now).is_err());
        let got = s.resolve("c1", "r1", &ids, now).unwrap();
        assert_eq!(got[0].1.text, "secret");
        assert_eq!(got[0].1.records.len(), 1);
        s.commit("c1", "r1");
        assert_eq!(
            s.claim("c1", "r1", &ids, now).unwrap_err(),
            "app context already consumed"
        );
        assert!(s.resolve("c1", "r1", &ids, now).is_err());
    }

    #[test]
    fn release_restores_and_claim_is_all_or_nothing() {
        let mut s = TicketStore::default();
        let now = Instant::now();
        let a = s.insert(ticket("c"), now).unwrap();
        let b = s.insert(ticket("c"), now).unwrap();
        s.claim("c", "r1", &[a.clone()], now).unwrap();
        assert!(s.claim("c", "r2", &[b.clone(), a.clone()], now).is_err());
        s.claim("c", "r3", &[b.clone()], now).unwrap();
        s.release("c", "r1");
        s.claim("c", "r2", &[a.clone()], now).unwrap();
        assert!(s.claim("c", "r2", &[a.clone(), a.clone()], now).is_err());
    }

    #[test]
    fn same_request_id_in_other_chat_cannot_consume_or_release_ticket() {
        let mut s = TicketStore::default();
        let now = Instant::now();
        let left = s.insert(ticket("left"), now).unwrap();
        let right = s.insert(ticket("right"), now).unwrap();
        s.claim("left", "shared-request", &[left], now).unwrap();
        s.claim("right", "shared-request", &[right.clone()], now)
            .unwrap();

        s.commit("left", "shared-request");
        assert_eq!(
            s.resolve("right", "shared-request", &[right.clone()], now)
                .unwrap()[0]
                .1
                .text,
            "secret",
            "another chat consumed the ticket"
        );
        s.release("left", "shared-request");
        assert!(
            s.claim("right", "different-request", &[right], now)
                .is_err(),
            "another chat released the ticket"
        );
    }

    #[test]
    fn expiry_and_caps() {
        let mut s = TicketStore::default();
        let now = Instant::now();
        let id = s.insert(ticket("c"), now).unwrap();
        assert!(s
            .claim("c", "r", &[id], now + TTL + Duration::from_secs(1))
            .is_err());
        for _ in 0..MAX_LIVE_PER_CHAT {
            s.insert(ticket("full"), now).unwrap();
        }
        assert!(s.insert(ticket("full"), now).is_err());
        assert!(s.insert(ticket("free"), now).is_ok());
        assert!(check_ids(&["not-a-uuid".into()]).is_err());
        assert!(check_ids(&vec![Uuid::new_v4().to_string(); 1]).is_ok());
    }

    #[test]
    fn label_is_sanitized() {
        assert_eq!(sanitize_label(Some("a\nb\u{0}c".into())), "abc");
        assert_eq!(sanitize_label(Some("  ".into())), "App context");
        assert_eq!(
            sanitize_label(Some("x".repeat(200))).chars().count(),
            MAX_LABEL_CHARS
        );
    }
}
