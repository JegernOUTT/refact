//! Per-tab dispatch gate. One holder at a time may change what a tab shows: a Browser-view
//! frame (pane size, screenshot and metrics as one unit), a pane resize, a Browser-view input
//! gesture, or a tool action that injects input, writes emulation or starts a navigation.
//! Lock order: a gate is always acquired before the runtime mutex, never while holding it.

use std::cell::RefCell;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateUse {
    Frame,
    Resize,
    Input,
    Navigate,
    Tool,
}

/// What the Browser view last made the tab show; only the gate holder changes it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TabView {
    pub applied_pane: Option<(u32, u32)>,
}

#[derive(Default)]
struct GateState {
    holder: Option<GateUse>,
    view: TabView,
    mouse: crate::MouseState,
}

#[derive(Default)]
pub struct TabGate {
    state: Mutex<GateState>,
    released: Condvar,
    retired: std::sync::atomic::AtomicBool,
    waiters: std::sync::atomic::AtomicUsize,
}

pub struct TabGateGuard {
    gate: Arc<TabGate>,
}

impl TabGate {
    fn state(&self) -> MutexGuard<'_, GateState> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// Waits until `deadline` for the gate; a frame never queues behind another frame. The
    /// error names the use that held the gate.
    pub fn acquire(
        self: &Arc<Self>,
        user: GateUse,
        deadline: Instant,
    ) -> Result<TabGateGuard, GateUse> {
        let mut state = self.state();
        loop {
            match state.holder {
                None => {
                    state.holder = Some(user);
                    return Ok(TabGateGuard {
                        gate: self.clone(),
                    });
                }
                Some(GateUse::Frame) if user == GateUse::Frame => return Err(GateUse::Frame),
                Some(holder) => {
                    let now = Instant::now();
                    if now >= deadline {
                        return Err(holder);
                    }
                    self.waiters
                        .fetch_add(1, std::sync::atomic::Ordering::Release);
                    state = self
                        .released
                        .wait_timeout(state, deadline - now)
                        .unwrap_or_else(|error| error.into_inner())
                        .0;
                    self.waiters
                        .fetch_sub(1, std::sync::atomic::Ordering::Release);
                }
            }
        }
    }

    pub(crate) fn retire(&self) {
        self.retired
            .store(true, std::sync::atomic::Ordering::Release);
    }

    #[doc(hidden)]
    pub fn waiting_dispatches(&self) -> usize {
        self.waiters.load(std::sync::atomic::Ordering::Acquire)
    }

    pub fn holder(&self) -> Option<GateUse> {
        self.state().holder
    }
}

impl TabGateGuard {
    pub fn view(&self) -> TabView {
        self.gate.state().view.clone()
    }

    pub fn update_view(&self, update: impl FnOnce(&mut TabView)) {
        update(&mut self.gate.state().view);
    }
}

impl Drop for TabGateGuard {
    fn drop(&mut self) {
        self.gate.state().holder = None;
        self.gate.released.notify_all();
    }
}

struct StepGate {
    target_id: String,
    gate: Arc<TabGate>,
    deadline: Instant,
    held: Option<TabGateGuard>,
    holds: usize,
}

thread_local! {
    static STEP_GATES: RefCell<Vec<StepGate>> = const { RefCell::new(Vec::new()) };
}

/// Makes the gates of the given tabs available to input and navigation dispatches on this
/// thread until the scope is dropped. Each gate is taken lazily by the first dispatch.
pub struct StepGateScope {
    target_ids: Vec<String>,
}

pub fn enter_step_gates(gates: Vec<(String, Arc<TabGate>)>, deadline: Instant) -> StepGateScope {
    let target_ids = gates.iter().map(|(id, _)| id.clone()).collect();
    STEP_GATES.with(|step| {
        step.borrow_mut()
            .extend(gates.into_iter().map(|(target_id, gate)| StepGate {
                target_id,
                gate,
                deadline,
                held: None,
                holds: 0,
            }));
    });
    StepGateScope {
        target_ids,
    }
}

impl Drop for StepGateScope {
    fn drop(&mut self) {
        let released = STEP_GATES.with(|step| {
            let mut step = step.borrow_mut();
            let (released, kept): (Vec<StepGate>, Vec<StepGate>) = std::mem::take(&mut *step)
                .into_iter()
                .partition(|gate| self.target_ids.contains(&gate.target_id));
            *step = kept;
            released
        });
        drop(released);
    }
}

/// Holds the tab's step gate while alive; a no-op when no step scope covers the tab.
pub struct TabDispatchHold {
    target_id: Option<String>,
}

impl TabDispatchHold {
    pub fn mouse_state(&self) -> Option<crate::MouseState> {
        self.with_gate(|gate| gate.state().mouse.clone())
    }

    pub fn set_mouse_state(&self, mouse: crate::MouseState) {
        self.with_gate(|gate| gate.state().mouse = mouse);
    }

    fn with_gate<T>(&self, read: impl FnOnce(&TabGate) -> T) -> Option<T> {
        let id = self.target_id.as_ref()?;
        STEP_GATES.with(|step| {
            let step = step.borrow();
            let entry = step.iter().find(|entry| &entry.target_id == id)?;
            entry.held.as_ref().map(|guard| read(&guard.gate))
        })
    }
}

pub fn hold_tab_for_dispatch(target_id: &str) -> Result<TabDispatchHold, String> {
    let pending = STEP_GATES.with(|step| {
        let mut step = step.borrow_mut();
        let entry = step.iter_mut().find(|gate| gate.target_id == target_id)?;
        if entry.held.is_some() {
            entry.holds += 1;
            return Some(None);
        }
        Some(Some((entry.gate.clone(), entry.deadline)))
    });
    let Some(pending) = pending else {
        return Ok(TabDispatchHold {
            target_id: None,
        });
    };
    if let Some((gate, deadline)) = pending {
        let guard = gate.acquire(GateUse::Tool, deadline).map_err(|holder| {
            format!(
                "the tab stayed busy ({}) until the browser request deadline; nothing was dispatched",
                gate_use_label(holder)
            )
        })?;
        if gate.retired.load(std::sync::atomic::Ordering::Acquire) {
            return Err("The selected tab closed; nothing was dispatched".into());
        }
        STEP_GATES.with(|step| {
            if let Some(entry) = step
                .borrow_mut()
                .iter_mut()
                .find(|gate| gate.target_id == target_id)
            {
                entry.held = Some(guard);
                entry.holds += 1;
            }
        });
    }
    Ok(TabDispatchHold {
        target_id: Some(target_id.to_string()),
    })
}

impl Drop for TabDispatchHold {
    fn drop(&mut self) {
        let Some(target_id) = self.target_id.take() else {
            return;
        };
        let released = STEP_GATES.with(|step| {
            let mut step = step.borrow_mut();
            let entry = step.iter_mut().find(|gate| gate.target_id == target_id)?;
            entry.holds = entry.holds.saturating_sub(1);
            if entry.holds == 0 {
                entry.held.take()
            } else {
                None
            }
        });
        drop(released);
    }
}

/// Runs one dispatch under the tab's step gate, if a step scope covers the tab.
pub fn with_tab_dispatch<T>(
    target_id: &str,
    dispatch: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let _hold = hold_tab_for_dispatch(target_id)?;
    dispatch()
}

pub fn gate_use_label(user: GateUse) -> &'static str {
    match user {
        GateUse::Frame => "a Browser-view frame",
        GateUse::Resize => "a Browser-view resize",
        GateUse::Input => "a Browser-view input",
        GateUse::Navigate => "a Browser-view navigation",
        GateUse::Tool => "another browser action",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn soon(ms: u64) -> Instant {
        Instant::now() + Duration::from_millis(ms)
    }

    #[test]
    fn one_holder_at_a_time_and_frames_never_queue_behind_frames() {
        let gate = Arc::new(TabGate::default());
        let frame = gate.acquire(GateUse::Frame, soon(10)).unwrap();
        let started = Instant::now();
        assert_eq!(
            gate.acquire(GateUse::Frame, soon(500)).err(),
            Some(GateUse::Frame)
        );
        assert!(started.elapsed() < Duration::from_millis(100));
        assert_eq!(
            gate.acquire(GateUse::Input, soon(30)).err(),
            Some(GateUse::Frame)
        );
        frame.update_view(|view| view.applied_pane = Some((800, 600)));
        drop(frame);
        let input = gate.acquire(GateUse::Input, soon(10)).unwrap();
        assert_eq!(input.view().applied_pane, Some((800, 600)));
        assert_eq!(
            gate.acquire(GateUse::Frame, soon(30)).err(),
            Some(GateUse::Input)
        );
        let waiter = std::thread::spawn({
            let gate = gate.clone();
            move || gate.acquire(GateUse::Tool, soon(2_000)).map(|_| ())
        });
        std::thread::sleep(Duration::from_millis(50));
        drop(input);
        assert_eq!(waiter.join().unwrap(), Ok(()));
        assert_eq!(gate.holder(), None);
    }

    #[test]
    fn step_dispatches_share_one_hold_until_the_last_dispatch_ends() {
        let gate = Arc::new(TabGate::default());
        assert!(hold_tab_for_dispatch("other").unwrap().target_id.is_none());
        let scope = enter_step_gates(vec![("tab".into(), gate.clone())], soon(1_000));
        let outer = hold_tab_for_dispatch("tab").unwrap();
        assert_eq!(gate.holder(), Some(GateUse::Tool));
        with_tab_dispatch("tab", || Ok(())).unwrap();
        assert_eq!(gate.holder(), Some(GateUse::Tool));
        drop(outer);
        assert_eq!(gate.holder(), None);

        let frame = gate.acquire(GateUse::Frame, soon(10)).unwrap();
        let releasing = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            drop(frame);
        });
        let mut dispatched_after_frame = false;
        with_tab_dispatch("tab", || {
            dispatched_after_frame = gate.holder() == Some(GateUse::Tool);
            Ok(())
        })
        .unwrap();
        releasing.join().unwrap();
        assert!(dispatched_after_frame);

        let _held = hold_tab_for_dispatch("tab").unwrap();
        drop(scope);
        assert_eq!(gate.holder(), None, "leaving the step releases its gates");
    }

    #[test]
    fn a_step_gives_up_at_its_deadline_without_dispatching() {
        let gate = Arc::new(TabGate::default());
        let frame = gate.acquire(GateUse::Frame, soon(10)).unwrap();
        let _scope = enter_step_gates(vec![("tab".into(), gate.clone())], soon(50));
        let mut dispatched = false;
        let error = with_tab_dispatch("tab", || {
            dispatched = true;
            Ok(())
        })
        .unwrap_err();
        assert!(error.contains("nothing was dispatched"), "{error}");
        assert!(!dispatched);
        drop(frame);
    }
}
