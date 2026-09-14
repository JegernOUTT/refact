use crate::identity::{CheckoutKey, ProjectKey, RunKey, RuntimeKind, WorkScope};
use crate::settings::{CoordinationSettings, SettingsError};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};
use tokio::sync::oneshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TicketId(pub u64);

impl fmt::Display for TicketId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ticket-{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdmissionRequest {
    pub run: RunKey,
    pub scope: WorkScope,
    pub depth: usize,
}

impl AdmissionRequest {
    pub fn new(run: RunKey, scope: WorkScope) -> Self {
        AdmissionRequest {
            run,
            scope,
            depth: 0,
        }
    }

    pub fn with_depth(mut self, depth: usize) -> Self {
        self.depth = depth;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum AdmissionError {
    Disabled,
    Duplicate {
        run: RunKey,
    },
    QueueFull {
        pending: usize,
        max_pending: usize,
    },
    OutstandingFull {
        outstanding: usize,
        max_outstanding: usize,
    },
    DepthExceeded {
        depth: usize,
        max_subchat_depth: usize,
    },
    Cancelled,
    Gone,
}

impl fmt::Display for AdmissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AdmissionError::Disabled => write!(f, "coordination scheduler is disabled"),
            AdmissionError::Duplicate { run } => write!(f, "run {run} is already admitted"),
            AdmissionError::QueueFull {
                pending,
                max_pending,
            } => write!(f, "admission queue is full ({pending}/{max_pending})"),
            AdmissionError::OutstandingFull {
                outstanding,
                max_outstanding,
            } => write!(
                f,
                "too many outstanding runs ({outstanding}/{max_outstanding})"
            ),
            AdmissionError::DepthExceeded {
                depth,
                max_subchat_depth,
            } => write!(f, "depth {depth} exceeds limit {max_subchat_depth}"),
            AdmissionError::Cancelled => write!(f, "admission was cancelled"),
            AdmissionError::Gone => write!(f, "admission ticket no longer exists"),
        }
    }
}

impl std::error::Error for AdmissionError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TicketState {
    Reserved,
    Pending,
    Active,
    Parked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TicketSnapshot {
    pub id: TicketId,
    pub run: RunKey,
    pub scope: WorkScope,
    pub depth: usize,
    pub seq: u64,
    pub state: TicketState,
    pub park_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchedulerSnapshot {
    pub settings: CoordinationSettings,
    pub active: usize,
    pub pending: usize,
    pub reserved: usize,
    pub parked: usize,
    pub outstanding: usize,
    pub active_by_project: BTreeMap<ProjectKey, usize>,
    pub active_by_root: BTreeMap<CheckoutKey, usize>,
    pub active_by_runtime: BTreeMap<RuntimeKind, usize>,
    pub tickets: Vec<TicketSnapshot>,
}

struct Entry {
    run: RunKey,
    scope: WorkScope,
    depth: usize,
    seq: u64,
    ready_seq: u64,
    state: TicketState,
    park_count: u32,
    cancel_requested: bool,
    waiter: Option<oneshot::Sender<Result<(), AdmissionError>>>,
}

struct State {
    settings: CoordinationSettings,
    next_id: u64,
    next_seq: u64,
    entries: HashMap<TicketId, Entry>,
    by_run: HashMap<RunKey, TicketId>,
    queue: BTreeSet<(u64, TicketId)>,
    active_total: usize,
    active_by_project: HashMap<ProjectKey, usize>,
    active_by_root: HashMap<CheckoutKey, usize>,
    last_root: Option<CheckoutKey>,
    roots: VecDeque<CheckoutKey>,
    root_credit: usize,
    resume_burst: HashMap<CheckoutKey, usize>,
}

impl State {
    fn counts(&self) -> (usize, usize, usize) {
        let mut pending = 0;
        let mut reserved = 0;
        let mut parked = 0;
        for entry in self.entries.values() {
            match entry.state {
                TicketState::Pending => pending += 1,
                TicketState::Reserved => reserved += 1,
                TicketState::Parked => parked += 1,
                TicketState::Active => {}
            }
        }
        (pending, reserved, parked)
    }

    fn fits(&self, entry: &Entry) -> bool {
        if self.active_total >= self.settings.max_active_engine {
            return false;
        }
        let project = self
            .active_by_project
            .get(&entry.scope.project)
            .copied()
            .unwrap_or(0);
        if project >= self.settings.max_active_project {
            return false;
        }
        let root = self
            .active_by_root
            .get(&entry.scope.root)
            .copied()
            .unwrap_or(0);
        root < self.settings.max_active_root
    }

    fn charge(&mut self, scope: &WorkScope) {
        self.active_total += 1;
        *self
            .active_by_project
            .entry(scope.project.clone())
            .or_insert(0) += 1;
        *self.active_by_root.entry(scope.root.clone()).or_insert(0) += 1;
    }

    fn discharge(&mut self, scope: &WorkScope) {
        self.active_total = self.active_total.saturating_sub(1);
        if let Some(count) = self.active_by_project.get_mut(&scope.project) {
            *count -= 1;
            if *count == 0 {
                self.active_by_project.remove(&scope.project);
            }
        }
        if let Some(count) = self.active_by_root.get_mut(&scope.root) {
            *count -= 1;
            if *count == 0 {
                self.active_by_root.remove(&scope.root);
            }
        }
    }

    fn pump(&mut self) {
        while self.active_total < self.settings.max_active_engine {
            // One lane per root, FIFO by original age within each lane. A root
            // with many queued descendants cannot monopolize all later releases.
            let mut lanes: BTreeMap<CheckoutKey, Vec<(u64, TicketId)>> = BTreeMap::new();
            for &(seq, id) in &self.queue {
                if let Some(entry) = self.entries.get(&id) {
                    if entry.state == TicketState::Pending && self.fits(entry) {
                        lanes
                            .entry(entry.scope.root.clone())
                            .or_default()
                            .push((seq, id));
                    }
                }
            }
            // Persistent rotation, not lexicographic successor: newly arriving
            // roots join the tail and cannot postpone an older lane's turn.
            let mut chosen_root = None;
            for _ in 0..self.roots.len() {
                let root = self.roots.front().expect("nonempty rotation").clone();
                if lanes.contains_key(&root) {
                    chosen_root = Some(root);
                    break;
                }
                self.roots.rotate_left(1);
                self.root_credit = 0;
            }
            let Some(root) = chosen_root else {
                break;
            };
            if self.last_root.as_ref() != Some(&root) || self.root_credit == 0 {
                self.root_credit = self.settings.root_quantum(root.as_str());
            }
            self.last_root = Some(root.clone());
            let lane = &lanes[&root];
            let burst = self.resume_burst.get(&root).copied().unwrap_or(0);
            let fairness_turn = burst >= self.settings.max_resume_burst;
            let chosen = if fairness_turn {
                // Every bounded burst gets a strict ready-FIFO turn, not only
                // fresh work. Otherwise two older resumers can alternate forever
                // ahead of a younger accepted resumer with no fresh work present.
                lane.iter()
                    .min_by_key(|(_, id)| self.entries[id].ready_seq)
                    .unwrap()
            } else {
                &lane[0]
            };
            let (seq, id) = *chosen;
            let scope = self.entries[&id].scope.clone();
            if !fairness_turn && self.entries[&id].park_count > 0 {
                self.resume_burst.insert(root, burst.saturating_add(1));
            } else {
                self.resume_burst.remove(&root);
            }
            self.root_credit = self.root_credit.saturating_sub(1);
            if self.root_credit == 0 {
                self.roots.rotate_left(1);
            }
            self.charge(&scope);
            self.queue.remove(&(seq, id));
            if let Some(entry) = self.entries.get_mut(&id) {
                entry.state = TicketState::Active;
                if let Some(tx) = entry.waiter.take() {
                    let _ = tx.send(Ok(()));
                }
            }
        }
    }

    fn remove(&mut self, id: TicketId) -> Option<Entry> {
        let entry = self.entries.remove(&id)?;
        self.by_run.remove(&entry.run);
        self.queue.remove(&(entry.seq, id));
        if entry.state == TicketState::Active {
            let scope = entry.scope.clone();
            self.discharge(&scope);
        }
        if !self
            .entries
            .values()
            .any(|other| other.scope.root == entry.scope.root)
        {
            self.resume_burst.remove(&entry.scope.root);
            self.roots.retain(|root| *root != entry.scope.root);
        }
        Some(entry)
    }
}

struct Inner {
    state: Mutex<State>,
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn begin_wait(
        self: &Arc<Self>,
        id: TicketId,
    ) -> Result<oneshot::Receiver<Result<(), AdmissionError>>, AdmissionError> {
        let (tx, rx) = oneshot::channel();
        let mut state = self.lock();
        let ready_seq = state.next_seq;
        state.next_seq += 1;
        let (seq, needs_queue) = match state.entries.get_mut(&id) {
            None => return Err(AdmissionError::Gone),
            Some(entry) => match entry.state {
                TicketState::Active => {
                    let _ = tx.send(Ok(()));
                    return Ok(rx);
                }
                TicketState::Pending => (entry.seq, false),
                TicketState::Reserved | TicketState::Parked => {
                    entry.state = TicketState::Pending;
                    entry.ready_seq = ready_seq;
                    entry.waiter = Some(tx);
                    (entry.seq, true)
                }
            },
        };
        if !needs_queue {
            return Err(AdmissionError::Gone);
        }
        state.queue.insert((seq, id));
        state.pump();
        Ok(rx)
    }

    fn abandon(&self, id: TicketId) {
        let mut state = self.lock();
        if state.remove(id).is_some() {
            state.pump();
        }
    }

    fn release(&self, id: TicketId) {
        self.abandon(id);
    }

    fn park(&self, id: TicketId) -> Result<(), AdmissionError> {
        let mut state = self.lock();
        let scope = match state.entries.get_mut(&id) {
            None => return Err(AdmissionError::Gone),
            Some(entry) => {
                if entry.state != TicketState::Active {
                    return Err(AdmissionError::Gone);
                }
                if entry.cancel_requested {
                    return Err(AdmissionError::Cancelled);
                }
                entry.state = TicketState::Parked;
                entry.park_count += 1;
                entry.scope.clone()
            }
        };
        state.discharge(&scope);
        state.pump();
        Ok(())
    }

    fn cancel(&self, id: TicketId) -> bool {
        let mut state = self.lock();
        if let Some(entry) = state.entries.get_mut(&id) {
            if entry.state == TicketState::Active {
                if entry.cancel_requested {
                    return false;
                }
                entry.cancel_requested = true;
                return true;
            }
        }
        match state.remove(id) {
            None => false,
            Some(mut entry) => {
                if let Some(tx) = entry.waiter.take() {
                    let _ = tx.send(Err(AdmissionError::Cancelled));
                }
                state.pump();
                true
            }
        }
    }
}

pub struct AdmissionScheduler {
    inner: Arc<Inner>,
}

impl fmt::Debug for AdmissionScheduler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.inner.lock();
        f.debug_struct("AdmissionScheduler")
            .field("active", &state.active_total)
            .field("outstanding", &state.entries.len())
            .finish()
    }
}

impl Default for AdmissionScheduler {
    fn default() -> Self {
        AdmissionScheduler::new(CoordinationSettings::default())
    }
}

impl AdmissionScheduler {
    pub fn new(settings: CoordinationSettings) -> Self {
        AdmissionScheduler {
            inner: Arc::new(Inner {
                state: Mutex::new(State {
                    settings,
                    next_id: 1,
                    next_seq: 1,
                    entries: HashMap::new(),
                    by_run: HashMap::new(),
                    queue: BTreeSet::new(),
                    active_total: 0,
                    active_by_project: HashMap::new(),
                    active_by_root: HashMap::new(),
                    last_root: None,
                    roots: VecDeque::new(),
                    root_credit: 0,
                    resume_burst: HashMap::new(),
                }),
            }),
        }
    }

    pub fn handle(&self) -> AdmissionSchedulerHandle {
        AdmissionSchedulerHandle {
            inner: self.inner.clone(),
        }
    }

    pub fn settings(&self) -> CoordinationSettings {
        self.inner.lock().settings.clone()
    }

    pub fn configure(&self, settings: CoordinationSettings) -> Result<(), SettingsError> {
        settings.validate()?;
        let mut state = self.inner.lock();
        state.settings = settings;
        let quantum = state
            .last_root
            .as_ref()
            .map(|root| state.settings.root_quantum(root.as_str()))
            .unwrap_or(0);
        state.root_credit = state.root_credit.min(quantum);
        state.pump();
        Ok(())
    }

    pub fn reserve(&self, request: AdmissionRequest) -> Result<Ticket, AdmissionError> {
        let mut state = self.inner.lock();
        if !state.settings.enabled {
            return Err(AdmissionError::Disabled);
        }
        if request.depth > state.settings.max_subchat_depth {
            return Err(AdmissionError::DepthExceeded {
                depth: request.depth,
                max_subchat_depth: state.settings.max_subchat_depth,
            });
        }
        if state.by_run.contains_key(&request.run) {
            return Err(AdmissionError::Duplicate { run: request.run });
        }
        let outstanding = state.entries.len();
        if outstanding >= state.settings.max_outstanding {
            return Err(AdmissionError::OutstandingFull {
                outstanding,
                max_outstanding: state.settings.max_outstanding,
            });
        }
        let (pending, reserved, _) = state.counts();
        // Parked dependency chains are already accepted resident work, not new
        // queue admission. max_outstanding bounds residency independently.
        let queued = pending + reserved;
        if queued >= state.settings.max_pending {
            return Err(AdmissionError::QueueFull {
                pending: queued,
                max_pending: state.settings.max_pending,
            });
        }

        let id = TicketId(state.next_id);
        state.next_id += 1;
        let seq = state.next_seq;
        state.next_seq += 1;
        let run = request.run.clone();
        state.entries.insert(
            id,
            Entry {
                run: run.clone(),
                scope: request.scope.clone(),
                depth: request.depth,
                seq,
                ready_seq: seq,
                state: TicketState::Reserved,
                park_count: 0,
                cancel_requested: false,
                waiter: None,
            },
        );
        if !state.roots.contains(&request.scope.root) {
            state.roots.push_back(request.scope.root.clone());
        }
        state.by_run.insert(run.clone(), id);
        drop(state);

        Ok(Ticket {
            inner: self.inner.clone(),
            id,
            run,
            scope: request.scope,
            seq,
            armed: true,
        })
    }

    pub fn cancel(&self, run: &RunKey) -> bool {
        let id = { self.inner.lock().by_run.get(run).copied() };
        match id {
            Some(id) => self.inner.cancel(id),
            None => false,
        }
    }

    pub fn snapshot(&self) -> SchedulerSnapshot {
        let state = self.inner.lock();
        let (pending, reserved, parked) = state.counts();
        let mut tickets: Vec<TicketSnapshot> = state
            .entries
            .iter()
            .map(|(id, entry)| TicketSnapshot {
                id: *id,
                run: entry.run.clone(),
                scope: entry.scope.clone(),
                depth: entry.depth,
                seq: entry.seq,
                state: entry.state,
                park_count: entry.park_count,
            })
            .collect();
        tickets.sort_by_key(|t| t.seq);
        let mut active_by_runtime: BTreeMap<RuntimeKind, usize> = BTreeMap::new();
        for entry in state.entries.values() {
            if entry.state == TicketState::Active {
                *active_by_runtime.entry(entry.run.runtime()).or_insert(0) += 1;
            }
        }
        SchedulerSnapshot {
            settings: state.settings.clone(),
            active: state.active_total,
            pending,
            reserved,
            parked,
            outstanding: state.entries.len(),
            active_by_project: state
                .active_by_project
                .iter()
                .map(|(k, v)| (k.clone(), *v))
                .collect(),
            active_by_root: state
                .active_by_root
                .iter()
                .map(|(k, v)| (k.clone(), *v))
                .collect(),
            active_by_runtime,
            tickets,
        }
    }
}

#[derive(Clone)]
pub struct AdmissionSchedulerHandle {
    inner: Arc<Inner>,
}

impl AdmissionSchedulerHandle {
    pub fn cancel(&self, run: &RunKey) -> bool {
        let id = { self.inner.lock().by_run.get(run).copied() };
        match id {
            Some(id) => self.inner.cancel(id),
            None => false,
        }
    }

    pub fn is_tracked(&self, run: &RunKey) -> bool {
        self.inner.lock().by_run.contains_key(run)
    }

    pub fn active_count(&self) -> usize {
        self.inner.lock().active_total
    }
}

pub struct Ticket {
    inner: Arc<Inner>,
    id: TicketId,
    run: RunKey,
    scope: WorkScope,
    seq: u64,
    armed: bool,
}

impl fmt::Debug for Ticket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ticket")
            .field("id", &self.id)
            .field("run", &self.run)
            .field("seq", &self.seq)
            .finish()
    }
}

impl Ticket {
    pub fn id(&self) -> TicketId {
        self.id
    }

    pub fn run(&self) -> &RunKey {
        &self.run
    }

    pub fn scope(&self) -> &WorkScope {
        &self.scope
    }

    pub fn seq(&self) -> u64 {
        self.seq
    }

    pub fn state(&self) -> Option<TicketState> {
        self.inner.lock().entries.get(&self.id).map(|e| e.state)
    }

    pub fn cancel(mut self) {
        self.armed = false;
        self.inner.cancel(self.id);
    }

    pub async fn acquire(mut self) -> Result<Permit, AdmissionError> {
        self.armed = false;
        let inner = self.inner.clone();
        let id = self.id;
        let rx = inner.begin_wait(id)?;
        let mut guard = AcquireGuard {
            inner: inner.clone(),
            id,
            armed: true,
        };
        match rx.await {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                guard.armed = false;
                return Err(err);
            }
            Err(_) => {
                guard.armed = false;
                return Err(AdmissionError::Cancelled);
            }
        }
        let cancelled = inner
            .lock()
            .entries
            .get(&id)
            .is_none_or(|entry| entry.cancel_requested);
        if cancelled {
            return Err(AdmissionError::Cancelled);
        }
        guard.armed = false;
        Ok(Permit {
            inner,
            id,
            run: self.run.clone(),
            scope: self.scope.clone(),
            seq: self.seq,
            armed: true,
        })
    }
}

impl Drop for Ticket {
    fn drop(&mut self) {
        if self.armed {
            self.inner.abandon(self.id);
        }
    }
}

struct AcquireGuard {
    inner: Arc<Inner>,
    id: TicketId,
    armed: bool,
}

impl Drop for AcquireGuard {
    fn drop(&mut self) {
        if self.armed {
            self.inner.abandon(self.id);
        }
    }
}

pub struct Permit {
    inner: Arc<Inner>,
    id: TicketId,
    run: RunKey,
    scope: WorkScope,
    seq: u64,
    armed: bool,
}

impl fmt::Debug for Permit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Permit")
            .field("id", &self.id)
            .field("run", &self.run)
            .field("seq", &self.seq)
            .finish()
    }
}

impl Permit {
    pub fn id(&self) -> TicketId {
        self.id
    }

    pub fn run(&self) -> &RunKey {
        &self.run
    }

    pub fn scope(&self) -> &WorkScope {
        &self.scope
    }

    pub fn seq(&self) -> u64 {
        self.seq
    }

    pub fn release(mut self) {
        if self.armed {
            self.armed = false;
            self.inner.release(self.id);
        }
    }

    pub fn park(mut self) -> Result<Ticket, AdmissionError> {
        if !self.armed {
            return Err(AdmissionError::Gone);
        }
        self.inner.park(self.id)?;
        self.armed = false;
        Ok(Ticket {
            inner: self.inner.clone(),
            id: self.id,
            run: self.run.clone(),
            scope: self.scope.clone(),
            seq: self.seq,
            armed: true,
        })
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        if self.armed {
            self.armed = false;
            self.inner.release(self.id);
        }
    }
}
