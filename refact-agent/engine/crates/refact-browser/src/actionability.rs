use std::error::Error;
use std::fmt::{Display, Formatter};
use std::time::{Duration, Instant};

use crate::ElementState;

pub const LOCATOR_RETRY_BACKOFF_MS: &[u64] = &[0, 20, 50, 100, 100, 500, 500];
pub const ACTION_RETRY_BACKOFF_MS: &[u64] = &[0, 20, 100, 100, 500, 500];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionKind {
    Click,
    DblClick,
    Hover,
    Tap,
    Fill,
    Type,
    Press,
    Check,
    Uncheck,
    SelectOption,
    SetInputFiles,
    DragSource,
    DragTarget,
    Focus,
    ScrollIntoViewIfNeeded,
}

impl ActionKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Click => "click",
            Self::DblClick => "dblclick",
            Self::Hover => "hover",
            Self::Tap => "tap",
            Self::Fill => "fill",
            Self::Type => "type",
            Self::Press => "press",
            Self::Check => "check",
            Self::Uncheck => "uncheck",
            Self::SelectOption => "select option",
            Self::SetInputFiles => "set input files",
            Self::DragSource => "drag source",
            Self::DragTarget => "drag target",
            Self::Focus => "focus",
            Self::ScrollIntoViewIfNeeded => "scroll into view if needed",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RequiredStates {
    pub visible: bool,
    pub stable: bool,
    pub receives_events: bool,
    pub enabled: bool,
    pub editable: bool,
}

pub fn required_states(action: ActionKind) -> RequiredStates {
    match action {
        ActionKind::Click
        | ActionKind::DblClick
        | ActionKind::Tap
        | ActionKind::Check
        | ActionKind::Uncheck => RequiredStates {
            visible: true,
            stable: true,
            receives_events: true,
            enabled: true,
            editable: false,
        },
        ActionKind::Hover | ActionKind::DragSource | ActionKind::DragTarget => RequiredStates {
            visible: true,
            stable: true,
            receives_events: true,
            enabled: false,
            editable: false,
        },
        ActionKind::Fill => RequiredStates {
            visible: true,
            stable: false,
            receives_events: false,
            enabled: true,
            editable: true,
        },
        ActionKind::SelectOption => RequiredStates {
            visible: true,
            stable: false,
            receives_events: false,
            enabled: true,
            editable: false,
        },
        ActionKind::ScrollIntoViewIfNeeded => RequiredStates {
            visible: false,
            stable: true,
            receives_events: false,
            enabled: false,
            editable: false,
        },
        ActionKind::Type | ActionKind::Press | ActionKind::SetInputFiles | ActionKind::Focus => {
            RequiredStates::default()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeoutKind {
    Action,
    Navigation,
    Expect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActionabilityTimeouts {
    pub action: Duration,
    pub navigation: Duration,
    pub expect: Duration,
}

impl Default for ActionabilityTimeouts {
    fn default() -> Self {
        Self {
            action: Duration::from_millis(5_000),
            navigation: Duration::from_millis(60_000),
            expect: Duration::from_millis(5_000),
        }
    }
}

impl ActionabilityTimeouts {
    pub fn get(self, kind: TimeoutKind) -> Duration {
        match kind {
            TimeoutKind::Action => self.action,
            TimeoutKind::Navigation => self.navigation,
            TimeoutKind::Expect => self.expect,
        }
    }
}

pub trait Clock {
    fn now(&self) -> Duration;
    fn sleep(&self, duration: Duration);
}

#[derive(Debug)]
pub struct SystemClock {
    origin: Instant,
}

impl Default for SystemClock {
    fn default() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Deadline {
    at: Duration,
}

impl Deadline {
    pub fn new<C: Clock>(clock: &C, timeout: Duration) -> Self {
        Self {
            at: clock.now().saturating_add(timeout),
        }
    }

    pub fn remaining<C: Clock>(self, clock: &C) -> Duration {
        self.at.saturating_sub(clock.now())
    }

    pub fn expired<C: Clock>(self, clock: &C) -> bool {
        clock.now() >= self.at
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CallLog {
    pub entries: Vec<String>,
}

impl CallLog {
    pub fn push(&mut self, message: impl Into<String>) {
        self.entries.push(message.into());
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionabilityDiagnostic {
    NotFound,
    MultipleMatches { count: usize },
    NotVisible,
    NotStable,
    NotEnabled,
    NotEditable,
    OutsideViewport,
    Detached,
    InterceptsPointerEvents { description: String },
}

impl ActionabilityDiagnostic {
    fn log_line(&self) -> String {
        match self {
            Self::NotFound => "locator did not resolve to any element".to_string(),
            Self::MultipleMatches { count } => {
                format!("strict mode violation: locator resolved to {count} elements")
            }
            Self::NotVisible => "element is not visible".to_string(),
            Self::NotStable => "element is not stable".to_string(),
            Self::NotEnabled => "element is not enabled".to_string(),
            Self::NotEditable => "element is not editable".to_string(),
            Self::OutsideViewport => "element is outside of the viewport".to_string(),
            Self::Detached => "element was detached from the DOM".to_string(),
            Self::InterceptsPointerEvents { description } => {
                format!("{description} intercepts pointer events")
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocatorOutcome {
    Found { preview: String },
    NotFound,
    MultipleMatches { count: usize },
}

pub trait ActionabilityDriver {
    type Output;

    fn resolve(&mut self) -> LocatorOutcome;
    fn element_state(&mut self) -> Result<ElementState, ActionabilityDiagnostic>;
    fn perform(&mut self) -> Result<Self::Output, ActionabilityDiagnostic>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimeoutError {
    pub timeout: Duration,
    pub diagnostic: ActionabilityDiagnostic,
    pub call_log: CallLog,
}

impl Display for TimeoutError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        writeln!(
            formatter,
            "Timeout {}ms exceeded.",
            self.timeout.as_millis()
        )?;
        if !self.call_log.entries.is_empty() {
            writeln!(formatter, "Call log:")?;
            for entry in &self.call_log.entries {
                writeln!(formatter, "- {entry}")?;
            }
        }
        Ok(())
    }
}

impl Error for TimeoutError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionabilityError {
    Timeout(TimeoutError),
    Failed {
        diagnostic: ActionabilityDiagnostic,
        call_log: CallLog,
    },
}

impl Display for ActionabilityError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout(error) => Display::fmt(error, formatter),
            Self::Failed {
                diagnostic,
                call_log,
            } => {
                writeln!(formatter, "{}", diagnostic.log_line())?;
                if !call_log.entries.is_empty() {
                    writeln!(formatter, "Call log:")?;
                    for entry in &call_log.entries {
                        writeln!(formatter, "- {entry}")?;
                    }
                }
                Ok(())
            }
        }
    }
}

impl Error for ActionabilityError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionabilitySuccess<T> {
    pub output: T,
    pub call_log: CallLog,
}

pub struct ActionabilityEngine<C> {
    clock: C,
    timeouts: ActionabilityTimeouts,
}

impl<C: Clock> ActionabilityEngine<C> {
    pub fn new(clock: C, timeouts: ActionabilityTimeouts) -> Self {
        Self { clock, timeouts }
    }

    pub fn clock(&self) -> &C {
        &self.clock
    }

    pub fn execute<D: ActionabilityDriver>(
        &self,
        locator: &str,
        action: ActionKind,
        driver: &mut D,
    ) -> Result<D::Output, ActionabilityError> {
        self.execute_logged(locator, action, driver)
            .map(|success| success.output)
    }

    pub fn execute_logged<D: ActionabilityDriver>(
        &self,
        locator: &str,
        action: ActionKind,
        driver: &mut D,
    ) -> Result<ActionabilitySuccess<D::Output>, ActionabilityError> {
        self.execute_for(locator, action, TimeoutKind::Action, driver)
    }

    pub fn execute_for<D: ActionabilityDriver>(
        &self,
        locator: &str,
        action: ActionKind,
        timeout_kind: TimeoutKind,
        driver: &mut D,
    ) -> Result<ActionabilitySuccess<D::Output>, ActionabilityError> {
        self.execute_with_timeout(locator, action, self.timeouts.get(timeout_kind), driver)
    }

    pub fn execute_with_timeout<D: ActionabilityDriver>(
        &self,
        locator: &str,
        action: ActionKind,
        timeout: Duration,
        driver: &mut D,
    ) -> Result<ActionabilitySuccess<D::Output>, ActionabilityError> {
        let deadline = Deadline::new(&self.clock, timeout);
        let mut call_log = CallLog::default();
        let mut last_diagnostic = ActionabilityDiagnostic::NotFound;
        let mut locator_attempt = 0;
        call_log.push(format!("waiting for {locator}"));

        loop {
            let delay = backoff_delay(LOCATOR_RETRY_BACKOFF_MS, locator_attempt);
            if !self.wait(delay, deadline, &mut call_log) {
                return Err(timeout_error(timeout, last_diagnostic, call_log));
            }
            locator_attempt += 1;

            match driver.resolve() {
                LocatorOutcome::NotFound => {
                    last_diagnostic = ActionabilityDiagnostic::NotFound;
                    call_log.push(last_diagnostic.log_line());
                }
                LocatorOutcome::MultipleMatches { count } => {
                    let diagnostic = ActionabilityDiagnostic::MultipleMatches { count };
                    call_log.push(diagnostic.log_line());
                    return Err(ActionabilityError::Failed {
                        diagnostic,
                        call_log,
                    });
                }
                LocatorOutcome::Found { preview } => {
                    call_log.push(format!("locator resolved to {preview}"));
                    match self.run_action(
                        action,
                        deadline,
                        &mut call_log,
                        &mut last_diagnostic,
                        driver,
                    ) {
                        ActionLoopResult::Done(output) => {
                            return Ok(ActionabilitySuccess { output, call_log });
                        }
                        ActionLoopResult::RetryLocator => {}
                        ActionLoopResult::TimedOut => {
                            return Err(timeout_error(timeout, last_diagnostic, call_log));
                        }
                    }
                }
            }
        }
    }

    fn run_action<D: ActionabilityDriver>(
        &self,
        action: ActionKind,
        deadline: Deadline,
        call_log: &mut CallLog,
        last_diagnostic: &mut ActionabilityDiagnostic,
        driver: &mut D,
    ) -> ActionLoopResult<D::Output> {
        let required = required_states(action);
        let state_description = state_description(required);
        let mut retry = 0;

        loop {
            if deadline.expired(&self.clock) {
                return ActionLoopResult::TimedOut;
            }
            if retry == 0 {
                call_log.push(format!("attempting {} action", action.name()));
            } else {
                call_log.push(format!("retrying {} action", action.name()));
                let delay = backoff_delay(ACTION_RETRY_BACKOFF_MS, retry - 1);
                if !self.wait(delay, deadline, call_log) {
                    return ActionLoopResult::TimedOut;
                }
            }

            if let Some(description) = &state_description {
                call_log.push(format!("waiting for element to be {description}"));
                match driver.element_state() {
                    Ok(state) => {
                        if let Some(diagnostic) = missing_state(required, &state) {
                            call_log.push(diagnostic.log_line());
                            *last_diagnostic = diagnostic;
                            retry += 1;
                            continue;
                        }
                        call_log.push(format!("element is {description}"));
                    }
                    Err(diagnostic) => {
                        call_log.push(diagnostic.log_line());
                        *last_diagnostic = diagnostic.clone();
                        if diagnostic == ActionabilityDiagnostic::Detached {
                            return ActionLoopResult::RetryLocator;
                        }
                        retry += 1;
                        continue;
                    }
                }
            }

            match driver.perform() {
                Ok(output) => return ActionLoopResult::Done(output),
                Err(diagnostic) => {
                    call_log.push(diagnostic.log_line());
                    *last_diagnostic = diagnostic.clone();
                    if diagnostic == ActionabilityDiagnostic::Detached {
                        return ActionLoopResult::RetryLocator;
                    }
                    retry += 1;
                }
            }
        }
    }

    fn wait(&self, requested: Duration, deadline: Deadline, call_log: &mut CallLog) -> bool {
        if deadline.expired(&self.clock) {
            return false;
        }
        let remaining = deadline.remaining(&self.clock);
        let actual = requested.min(remaining);
        if !actual.is_zero() {
            call_log.push(format!("waiting {}ms", actual.as_millis()));
            self.clock.sleep(actual);
        }
        requested <= remaining && !deadline.expired(&self.clock)
    }
}

enum ActionLoopResult<T> {
    Done(T),
    RetryLocator,
    TimedOut,
}

fn backoff_delay(schedule: &[u64], attempt: usize) -> Duration {
    Duration::from_millis(schedule[attempt.min(schedule.len() - 1)])
}

fn state_description(required: RequiredStates) -> Option<String> {
    let mut states = Vec::new();
    if required.visible {
        states.push("visible");
    }
    if required.enabled {
        states.push("enabled");
    }
    if required.editable {
        states.push("editable");
    }
    if required.stable {
        states.push("stable");
    }
    match states.as_slice() {
        [] => None,
        [only] => Some((*only).to_string()),
        [left, right] => Some(format!("{left} and {right}")),
        _ => {
            let last = states.pop().unwrap();
            Some(format!("{} and {last}", states.join(", ")))
        }
    }
}

fn missing_state(
    required: RequiredStates,
    state: &ElementState,
) -> Option<ActionabilityDiagnostic> {
    if required.visible && !state.visible {
        Some(ActionabilityDiagnostic::NotVisible)
    } else if required.enabled && !state.enabled {
        Some(ActionabilityDiagnostic::NotEnabled)
    } else if required.editable && state.editable != Some(true) {
        Some(ActionabilityDiagnostic::NotEditable)
    } else if required.stable && !state.stable {
        Some(ActionabilityDiagnostic::NotStable)
    } else {
        None
    }
}

fn timeout_error(
    timeout: Duration,
    diagnostic: ActionabilityDiagnostic,
    call_log: CallLog,
) -> ActionabilityError {
    ActionabilityError::Timeout(TimeoutError {
        timeout,
        diagnostic,
        call_log,
    })
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::collections::VecDeque;
    use std::rc::Rc;

    use crate::CheckedState;

    use super::*;

    #[derive(Clone, Default)]
    struct MockClock {
        now: Rc<Cell<Duration>>,
        sleeps: Rc<RefCell<Vec<Duration>>>,
    }

    impl Clock for MockClock {
        fn now(&self) -> Duration {
            self.now.get()
        }

        fn sleep(&self, duration: Duration) {
            self.sleeps.borrow_mut().push(duration);
            self.now.set(self.now.get().saturating_add(duration));
        }
    }

    struct MockDriver {
        locator_outcomes: VecDeque<LocatorOutcome>,
        states: VecDeque<Result<ElementState, ActionabilityDiagnostic>>,
        actions: VecDeque<Result<&'static str, ActionabilityDiagnostic>>,
        resolve_calls: usize,
        state_calls: usize,
        action_calls: usize,
    }

    impl MockDriver {
        fn new() -> Self {
            Self {
                locator_outcomes: VecDeque::from([LocatorOutcome::Found {
                    preview: "<button>Save</button>".to_string(),
                }]),
                states: VecDeque::new(),
                actions: VecDeque::from([Ok("done")]),
                resolve_calls: 0,
                state_calls: 0,
                action_calls: 0,
            }
        }
    }

    impl ActionabilityDriver for MockDriver {
        type Output = &'static str;

        fn resolve(&mut self) -> LocatorOutcome {
            self.resolve_calls += 1;
            self.locator_outcomes
                .pop_front()
                .unwrap_or(LocatorOutcome::NotFound)
        }

        fn element_state(&mut self) -> Result<ElementState, ActionabilityDiagnostic> {
            self.state_calls += 1;
            self.states.pop_front().unwrap_or_else(|| Ok(good_state()))
        }

        fn perform(&mut self) -> Result<Self::Output, ActionabilityDiagnostic> {
            self.action_calls += 1;
            self.actions.pop_front().unwrap_or(Ok("done"))
        }
    }

    fn good_state() -> ElementState {
        ElementState {
            visible: true,
            enabled: true,
            editable: Some(true),
            checked: Some(CheckedState::Unchecked),
            stable: true,
        }
    }

    fn states(
        visible: bool,
        stable: bool,
        receives_events: bool,
        enabled: bool,
        editable: bool,
    ) -> RequiredStates {
        RequiredStates {
            visible,
            stable,
            receives_events,
            enabled,
            editable,
        }
    }

    #[test]
    fn every_action_maps_to_the_playwright_state_matrix() {
        let cases = [
            (ActionKind::Click, states(true, true, true, true, false)),
            (ActionKind::DblClick, states(true, true, true, true, false)),
            (ActionKind::Hover, states(true, true, true, false, false)),
            (ActionKind::Tap, states(true, true, true, true, false)),
            (ActionKind::Fill, states(true, false, false, true, true)),
            (ActionKind::Type, RequiredStates::default()),
            (ActionKind::Press, RequiredStates::default()),
            (ActionKind::Check, states(true, true, true, true, false)),
            (ActionKind::Uncheck, states(true, true, true, true, false)),
            (
                ActionKind::SelectOption,
                states(true, false, false, true, false),
            ),
            (ActionKind::SetInputFiles, RequiredStates::default()),
            (
                ActionKind::DragSource,
                states(true, true, true, false, false),
            ),
            (
                ActionKind::DragTarget,
                states(true, true, true, false, false),
            ),
            (ActionKind::Focus, RequiredStates::default()),
            (
                ActionKind::ScrollIntoViewIfNeeded,
                states(false, true, false, false, false),
            ),
        ];

        assert_eq!(cases.len(), 15);
        for (action, expected) in cases {
            assert_eq!(required_states(action), expected, "{action:?}");
        }
    }

    #[test]
    fn retry_backoff_schedules_are_exact_and_repeat_the_last_delay() {
        let locator = (0..9)
            .map(|attempt| backoff_delay(LOCATOR_RETRY_BACKOFF_MS, attempt).as_millis())
            .collect::<Vec<_>>();
        let action = (0..8)
            .map(|attempt| backoff_delay(ACTION_RETRY_BACKOFF_MS, attempt).as_millis())
            .collect::<Vec<_>>();

        assert_eq!(locator, vec![0, 20, 50, 100, 100, 500, 500, 500, 500]);
        assert_eq!(action, vec![0, 20, 100, 100, 500, 500, 500, 500]);
    }

    #[test]
    fn locator_retry_uses_its_exact_schedule() {
        let clock = MockClock::default();
        let engine = ActionabilityEngine::new(clock.clone(), ActionabilityTimeouts::default());
        let mut driver = MockDriver::new();
        driver.locator_outcomes = VecDeque::from([
            LocatorOutcome::NotFound,
            LocatorOutcome::NotFound,
            LocatorOutcome::NotFound,
            LocatorOutcome::Found {
                preview: "<button>Save</button>".to_string(),
            },
        ]);

        assert_eq!(
            engine.execute("button", ActionKind::Click, &mut driver),
            Ok("done")
        );
        assert_eq!(
            *clock.sleeps.borrow(),
            vec![
                Duration::from_millis(20),
                Duration::from_millis(50),
                Duration::from_millis(100)
            ]
        );
    }

    #[test]
    fn action_retry_uses_its_independent_exact_schedule() {
        let clock = MockClock::default();
        let engine = ActionabilityEngine::new(clock.clone(), ActionabilityTimeouts::default());
        let mut driver = MockDriver::new();
        let mut bad = good_state();
        bad.stable = false;
        driver.states = VecDeque::from([
            Ok(bad.clone()),
            Ok(bad.clone()),
            Ok(bad.clone()),
            Ok(bad),
            Ok(good_state()),
        ]);

        assert_eq!(
            engine.execute("button", ActionKind::Click, &mut driver),
            Ok("done")
        );
        assert_eq!(
            *clock.sleeps.borrow(),
            vec![
                Duration::from_millis(20),
                Duration::from_millis(100),
                Duration::from_millis(100)
            ]
        );
        assert_eq!(driver.state_calls, 5);
    }

    #[test]
    fn deadline_trims_a_long_wait_and_does_not_poll_after_expiry() {
        let clock = MockClock::default();
        let engine = ActionabilityEngine::new(clock.clone(), ActionabilityTimeouts::default());
        let mut driver = MockDriver::new();
        driver.locator_outcomes = VecDeque::from([
            LocatorOutcome::NotFound,
            LocatorOutcome::NotFound,
            LocatorOutcome::NotFound,
        ]);

        let result = engine.execute_with_timeout(
            "button",
            ActionKind::Click,
            Duration::from_millis(25),
            &mut driver,
        );

        assert!(matches!(result, Err(ActionabilityError::Timeout(_))));
        assert_eq!(
            *clock.sleeps.borrow(),
            vec![Duration::from_millis(20), Duration::from_millis(5)]
        );
        assert_eq!(driver.resolve_calls, 2);
    }

    #[test]
    fn state_that_becomes_good_on_third_poll_succeeds() {
        let clock = MockClock::default();
        let engine = ActionabilityEngine::new(clock, ActionabilityTimeouts::default());
        let mut driver = MockDriver::new();
        let mut bad = good_state();
        bad.visible = false;
        driver.states = VecDeque::from([Ok(bad.clone()), Ok(bad), Ok(good_state())]);

        assert_eq!(
            engine.execute("button", ActionKind::Click, &mut driver),
            Ok("done")
        );
        assert_eq!(driver.state_calls, 3);
        assert_eq!(driver.action_calls, 1);
    }

    #[test]
    fn permanently_bad_state_times_out_with_final_diagnostic_and_log() {
        let clock = MockClock::default();
        let engine = ActionabilityEngine::new(clock, ActionabilityTimeouts::default());
        let mut driver = MockDriver::new();
        let mut bad = good_state();
        bad.enabled = false;
        driver.states = VecDeque::from(vec![Ok(bad); 20]);

        let result = engine.execute_with_timeout(
            "button",
            ActionKind::Click,
            Duration::from_millis(250),
            &mut driver,
        );
        let Err(ActionabilityError::Timeout(error)) = result else {
            panic!("expected timeout")
        };

        assert_eq!(error.diagnostic, ActionabilityDiagnostic::NotEnabled);
        assert_eq!(error.timeout, Duration::from_millis(250));
        assert!(error
            .call_log
            .entries
            .contains(&"element is not enabled".to_string()));
        assert!(error.to_string().contains("Timeout 250ms exceeded."));
        assert!(error.to_string().contains("Call log:"));
    }

    #[test]
    fn click_call_log_is_ordered_across_two_retries_then_success() {
        let clock = MockClock::default();
        let engine = ActionabilityEngine::new(clock, ActionabilityTimeouts::default());
        let mut driver = MockDriver::new();
        let mut bad = good_state();
        bad.stable = false;
        driver.states = VecDeque::from([Ok(bad.clone()), Ok(bad), Ok(good_state())]);

        let success = engine
            .execute_logged("button", ActionKind::Click, &mut driver)
            .unwrap();

        assert_eq!(success.output, "done");
        assert_eq!(
            success.call_log.entries,
            vec![
                "waiting for button",
                "locator resolved to <button>Save</button>",
                "attempting click action",
                "waiting for element to be visible, enabled and stable",
                "element is not stable",
                "retrying click action",
                "waiting for element to be visible, enabled and stable",
                "element is not stable",
                "retrying click action",
                "waiting 20ms",
                "waiting for element to be visible, enabled and stable",
                "element is visible, enabled and stable",
            ]
        );
    }

    #[test]
    fn strict_multiple_match_failure_is_not_retried() {
        let clock = MockClock::default();
        let engine = ActionabilityEngine::new(clock.clone(), ActionabilityTimeouts::default());
        let mut driver = MockDriver::new();
        driver.locator_outcomes = VecDeque::from([LocatorOutcome::MultipleMatches { count: 3 }]);

        let result = engine.execute("button", ActionKind::Click, &mut driver);
        assert!(matches!(
            result,
            Err(ActionabilityError::Failed {
                diagnostic: ActionabilityDiagnostic::MultipleMatches { count: 3 },
                ..
            })
        ));
        assert_eq!(driver.resolve_calls, 1);
        assert!(clock.sleeps.borrow().is_empty());
    }

    #[test]
    fn detached_action_restarts_locator_retry_loop() {
        let clock = MockClock::default();
        let engine = ActionabilityEngine::new(clock.clone(), ActionabilityTimeouts::default());
        let mut driver = MockDriver::new();
        driver.locator_outcomes = VecDeque::from([
            LocatorOutcome::Found {
                preview: "<button>Old</button>".to_string(),
            },
            LocatorOutcome::Found {
                preview: "<button>New</button>".to_string(),
            },
        ]);
        driver.actions = VecDeque::from([Err(ActionabilityDiagnostic::Detached), Ok("done")]);

        assert_eq!(
            engine.execute("button", ActionKind::Click, &mut driver),
            Ok("done")
        );
        assert_eq!(driver.resolve_calls, 2);
        assert_eq!(*clock.sleeps.borrow(), vec![Duration::from_millis(20)]);
    }

    #[test]
    fn pointer_interception_retries_until_action_succeeds() {
        let clock = MockClock::default();
        let engine = ActionabilityEngine::new(clock, ActionabilityTimeouts::default());
        let mut driver = MockDriver::new();
        driver.actions = VecDeque::from([
            Err(ActionabilityDiagnostic::InterceptsPointerEvents {
                description: "<div class=overlay>".to_string(),
            }),
            Ok("done"),
        ]);

        assert_eq!(
            engine.execute("button", ActionKind::Click, &mut driver),
            Ok("done")
        );
        assert_eq!(driver.action_calls, 2);
    }

    #[test]
    fn configured_timeout_kinds_match_playwright_mcp_defaults() {
        let timeouts = ActionabilityTimeouts::default();
        assert_eq!(timeouts.action, Duration::from_millis(5_000));
        assert_eq!(timeouts.navigation, Duration::from_millis(60_000));
        assert_eq!(timeouts.expect, Duration::from_millis(5_000));
    }
}
