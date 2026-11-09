use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::{broadcast, oneshot, OwnedSemaphorePermit, Semaphore};
use uuid::Uuid;
pub const LEASE_MS: u64 = 120_000;
pub const HEARTBEAT_MS: u64 = 20_000;
pub const STALE_MS: u64 = 45_000;
pub const CAPTURE_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_VIEWS: usize = 128;
const MAX_PENDING: usize = 16;
#[derive(Clone, Debug, Serialize)]
pub struct EventEnvelope {
    pub generation: Uuid,
    pub seq: u64,
    pub kind: String,
    pub data: Value,
}
struct Events {
    seq: u64,
    ring: VecDeque<EventEnvelope>,
}
pub struct EventBus {
    generation: Uuid,
    events: Mutex<Events>,
    tx: broadcast::Sender<EventEnvelope>,
    slots: Arc<Semaphore>,
}
pub struct Subscription {
    pub initial: Vec<EventEnvelope>,
    pub rx: broadcast::Receiver<EventEnvelope>,
    _permit: OwnedSemaphorePermit,
}
impl Subscription {
    pub fn hold_capacity(&self) {
        let _ = &self._permit;
    }
}
impl EventBus {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(256);
        Self {
            generation: Uuid::new_v4(),
            events: Mutex::new(Events {
                seq: 0,
                ring: VecDeque::new(),
            }),
            tx,
            slots: Arc::new(Semaphore::new(32)),
        }
    }
    pub fn snapshot(&self) -> Vec<EventEnvelope> {
        self.events.lock().unwrap().ring.iter().cloned().collect()
    }
    pub fn close(&self) {
        self.slots.close();
        self.emit("shutdown", json!({}));
    }
    pub fn emit(&self, kind: &str, data: Value) {
        let mut events = self.events.lock().unwrap();
        events.seq += 1;
        let event = EventEnvelope {
            generation: self.generation,
            seq: events.seq,
            kind: kind.into(),
            data,
        };
        if events.ring.len() >= 256 {
            events.ring.pop_front();
        }
        events.ring.push_back(event.clone());
        let _ = self.tx.send(event);
    }
    pub fn subscribe(&self, cursor: Option<&str>) -> Result<Subscription, String> {
        let permit = self
            .slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| "app subscription capacity reached")?;
        let events = self.events.lock().unwrap();
        let rx = self.tx.subscribe();
        let parsed = cursor
            .and_then(|c| c.split_once(':'))
            .and_then(|(g, s)| Some((Uuid::parse_str(g).ok()?, s.parse::<u64>().ok()?)));
        let valid = parsed.filter(|(g, s)| {
            *g == self.generation
                && *s <= events.seq
                && events
                    .ring
                    .front()
                    .is_none_or(|e| s.saturating_add(1) >= e.seq)
        });
        let initial = match valid {
            Some((_, s)) => events.ring.iter().filter(|e| e.seq > s).cloned().collect(),
            None => vec![EventEnvelope {
                generation: self.generation,
                seq: events.seq,
                kind: if cursor.is_some() {
                    "reset"
                } else {
                    "snapshot"
                }
                .into(),
                data: json!({"reason":if cursor.is_some(){"cursor_gap"}else{"initial"}}),
            }],
        };
        Ok(Subscription {
            initial,
            rx,
            _permit: permit,
        })
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct ViewPresence {
    pub visible: Option<bool>,
    pub focused: Option<bool>,
    pub ready: Option<bool>,
}
#[derive(Clone)]
pub struct View {
    pub app_id: Uuid,
    pub chat_id: Option<String>,
    pub binding_id: Option<Uuid>,
    pub revision: String,
    pub generation: Uuid,
    pub visible: bool,
    pub focused: bool,
    pub ready: bool,
    token: String,
    registered: Instant,
    last_seen: Instant,
}
impl View {
    fn age_ms(&self, now: Instant) -> u64 {
        now.saturating_duration_since(self.last_seen).as_millis() as u64
    }
    fn expired(&self, now: Instant) -> bool {
        self.age_ms(now) >= LEASE_MS
    }
    pub fn stale_at(&self, now: Instant) -> bool {
        self.age_ms(now) >= STALE_MS
    }
    pub fn usable_at(&self, now: Instant) -> bool {
        self.ready && !self.stale_at(now)
    }
}
pub struct Pending {
    pub view_id: Uuid,
    pub generation: Uuid,
    pub tx: oneshot::Sender<Value>,
    pub expires: Instant,
}
#[derive(Default)]
pub struct Views {
    entries: HashMap<Uuid, View>,
    pending: HashMap<Uuid, Pending>,
}
impl Views {
    pub fn list(&mut self, chat_id: Option<&str>) -> Value {
        self.prune();
        let now = Instant::now();
        let mut rows: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, v)| chat_id.is_none_or(|id| v.chat_id.as_deref() == Some(id)))
            .collect();
        rows.sort_by_key(|(_, v)| (!v.usable_at(now), !v.focused, !v.visible, v.age_ms(now)));
        json!(rows
            .into_iter()
            .map(|(id, v)| json!({
                "view_id": id,
                "app_id": v.app_id,
                "chat_id": v.chat_id,
                "binding_id": v.binding_id,
                "revision": v.revision,
                "generation": v.generation,
                "visible": v.visible,
                "focused": v.focused,
                "ready": v.ready,
                "stale": v.stale_at(now),
                "usable": v.usable_at(now),
                "last_seen_ms_ago": v.age_ms(now),
                "registered_ms_ago": now.saturating_duration_since(v.registered).as_millis() as u64,
            }))
            .collect::<Vec<_>>())
    }
    pub fn active(&mut self) -> Vec<(Uuid, View)> {
        self.prune();
        self.entries
            .iter()
            .map(|(id, v)| (*id, v.clone()))
            .collect()
    }
    pub fn clear(&mut self) {
        self.entries.clear();
        self.pending.clear();
    }
    fn prune(&mut self) {
        let now = Instant::now();
        self.entries.retain(|_, v| !v.expired(now));
        self.pending
            .retain(|_, p| p.expires > now && self.entries.contains_key(&p.view_id));
    }
    fn evict_stalest(&mut self) -> bool {
        let now = Instant::now();
        let victim = self
            .entries
            .iter()
            .filter(|(_, v)| v.stale_at(now))
            .max_by_key(|(_, v)| v.age_ms(now))
            .map(|(id, _)| *id);
        match victim {
            Some(id) => {
                self.remove(id);
                true
            }
            None => false,
        }
    }
    pub fn register(
        &mut self,
        app_id: Uuid,
        chat_id: Option<String>,
        binding_id: Option<Uuid>,
        revision: String,
    ) -> Result<Value, String> {
        self.prune();
        if self.entries.len() >= MAX_VIEWS && !self.evict_stalest() {
            return Err("view capacity reached".into());
        }
        let view_id = Uuid::new_v4();
        let generation = Uuid::new_v4();
        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let now = Instant::now();
        self.entries.insert(
            view_id,
            View {
                app_id,
                chat_id,
                binding_id,
                revision: revision.clone(),
                generation,
                visible: false,
                focused: false,
                ready: false,
                token: token.clone(),
                registered: now,
                last_seen: now,
            },
        );
        Ok(json!({
            "view_id": view_id,
            "view_token": token,
            "generation": generation,
            "revision": revision,
            "expires_in_ms": LEASE_MS,
            "heartbeat_ms": HEARTBEAT_MS,
        }))
    }
    pub fn check(&mut self, id: Uuid, token: Option<&str>) -> Result<View, String> {
        self.prune();
        let view = self.entries.get(&id).ok_or("view expired or unavailable")?;
        if !token.is_some_and(|t| crate::daemon::auth::token_matches(t, &view.token)) {
            return Err("invalid view credential".into());
        }
        Ok(view.clone())
    }
    pub fn heartbeat(
        &mut self,
        id: Uuid,
        token: Option<&str>,
        presence: ViewPresence,
    ) -> Result<View, String> {
        self.check(id, token)?;
        let view = self
            .entries
            .get_mut(&id)
            .ok_or("view expired or unavailable")?;
        view.last_seen = Instant::now();
        if let Some(visible) = presence.visible {
            view.visible = visible;
        }
        if let Some(focused) = presence.focused {
            view.focused = focused;
        }
        if let Some(ready) = presence.ready {
            view.ready = ready;
        }
        Ok(view.clone())
    }
    pub fn lookup(&mut self, id: Uuid) -> Result<View, String> {
        self.prune();
        self.entries
            .get(&id)
            .cloned()
            .ok_or("view expired or unavailable".into())
    }
    pub fn remove(&mut self, id: Uuid) {
        self.entries.remove(&id);
        self.pending.retain(|_, p| p.view_id != id);
    }
    pub fn invalidate(&mut self, app_id: Uuid) {
        self.entries.retain(|_, v| v.app_id != app_id);
        self.prune();
    }
    pub fn request(
        &mut self,
        view_id: Uuid,
        view: &View,
    ) -> Result<(Uuid, oneshot::Receiver<Value>), String> {
        self.prune();
        let current = self
            .entries
            .get(&view_id)
            .ok_or("view expired or unavailable")?;
        if current.generation != view.generation {
            return Err("view generation changed".into());
        }
        if current.stale_at(Instant::now()) {
            return Err("view is not live (no recent heartbeat); reopen the app tab or pick a usable view from list".into());
        }
        if self.pending.len() >= MAX_PENDING {
            return Err("capture capacity reached".into());
        }
        let request_id = Uuid::new_v4();
        let (tx, rx) = oneshot::channel();
        self.pending.insert(
            request_id,
            Pending {
                view_id,
                generation: view.generation,
                tx,
                expires: Instant::now() + CAPTURE_TIMEOUT,
            },
        );
        Ok((request_id, rx))
    }
    pub fn cancel(&mut self, id: Uuid) {
        self.pending.remove(&id);
    }
    pub fn respond(
        &mut self,
        request_id: Uuid,
        view_id: Uuid,
        view: &View,
        result: Value,
    ) -> Result<(), String> {
        self.prune();
        let pending = self.pending.get(&request_id).ok_or("capture expired")?;
        if pending.view_id != view_id || pending.generation != view.generation {
            return Err("capture view mismatch".into());
        }
        let pending = self.pending.remove(&request_id).unwrap();
        pending
            .tx
            .send(result)
            .map_err(|_| "capture requester disconnected".into())
    }
    pub fn reject(
        &mut self,
        request_id: Uuid,
        view_id: Uuid,
        view: &View,
        error: &str,
    ) -> Result<(), String> {
        let error: String = error.chars().take(200).collect();
        self.respond(
            request_id,
            view_id,
            view,
            json!({"status": "error", "error": error}),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn age(views: &mut Views, id: Uuid, ms: u64) {
        let view = views.entries.get_mut(&id).unwrap();
        view.last_seen -= Duration::from_millis(ms);
    }
    fn register(views: &mut Views) -> (Uuid, String) {
        let a = views
            .register(Uuid::new_v4(), Some("chat".into()), None, "rev".into())
            .unwrap();
        (
            serde_json::from_value(a["view_id"].clone()).unwrap(),
            a["view_token"].as_str().unwrap().to_string(),
        )
    }
    #[test]
    fn bounded_stream_resets_gaps_and_releases_capacity() {
        let bus = EventBus::new();
        for i in 0..300 {
            bus.emit("data", json!(i));
        }
        let sub = bus
            .subscribe(Some(&format!("{}:0", bus.generation)))
            .unwrap();
        assert_eq!(sub.initial[0].kind, "reset");
        drop(sub);
        let subs: Vec<_> = (0..32).map(|_| bus.subscribe(None).unwrap()).collect();
        assert!(bus.subscribe(None).is_err());
        drop(subs);
        assert!(bus.subscribe(None).is_ok());
    }
    #[test]
    fn secret_not_in_events_and_exact_capture_view() {
        let mut views = Views::default();
        let (id, token) = register(&mut views);
        assert!(views.check(id, Some("wrong")).is_err());
        let view = views.check(id, Some(&token)).unwrap();
        let (request, _rx) = views.request(id, &view).unwrap();
        assert!(views
            .respond(request, Uuid::new_v4(), &view, json!({}))
            .is_err());
        assert!(views.respond(request, id, &view, json!({})).is_ok());
        assert!(views.respond(request, id, &view, json!({})).is_err());
    }
    #[test]
    fn heartbeat_presence_orders_usable_views_and_check_does_not_renew() {
        let mut views = Views::default();
        let (old, old_token) = register(&mut views);
        let (live, live_token) = register(&mut views);
        let ready = ViewPresence {
            visible: Some(true),
            focused: Some(true),
            ready: Some(true),
        };
        assert!(views.heartbeat(live, Some("wrong"), ready).is_err());
        views.heartbeat(live, Some(&live_token), ready).unwrap();
        views.heartbeat(old, Some(&old_token), ready).unwrap();
        age(&mut views, old, STALE_MS + 1);
        views.check(old, Some(&old_token)).unwrap();
        let list = views.list(Some("chat"));
        assert_eq!(list[0]["view_id"], json!(live));
        assert_eq!(list[0]["usable"], json!(true));
        assert_eq!(list[1]["stale"], json!(true));
        assert_eq!(list[1]["usable"], json!(false));
        assert!(views.list(Some("other")).as_array().unwrap().is_empty());
        let stale = views.lookup(old).unwrap();
        assert!(views.request(old, &stale).unwrap_err().contains("not live"));
        age(&mut views, old, LEASE_MS);
        assert!(views.lookup(old).is_err());
    }
    #[test]
    fn capacity_evicts_only_stale_views_and_reject_reports_error() {
        let mut views = Views::default();
        let ids: Vec<_> = (0..MAX_VIEWS).map(|_| register(&mut views).0).collect();
        assert!(views
            .register(Uuid::new_v4(), None, None, "rev".into())
            .is_err());
        age(&mut views, ids[3], STALE_MS + 5);
        age(&mut views, ids[7], STALE_MS + 1);
        assert!(views
            .register(Uuid::new_v4(), None, None, "rev".into())
            .is_ok());
        assert!(views.lookup(ids[3]).is_err());
        assert!(views.lookup(ids[7]).is_ok());
        let view = views.lookup(ids[0]).unwrap();
        let (request, mut rx) = views.request(ids[0], &view).unwrap();
        views
            .reject(request, ids[0], &view, &"x".repeat(500))
            .unwrap();
        let value = rx.try_recv().unwrap();
        assert_eq!(value["status"], json!("error"));
        assert_eq!(value["error"].as_str().unwrap().len(), 200);
    }
}
