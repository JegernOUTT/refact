use std::error::Error;
use std::fmt::{Display, Formatter};
use std::time::{Duration, Instant};

use crate::ElementState;
use refact_integrations::browser_models::ActionabilityDiagnostics;

pub const LOCATOR_RETRY_BACKOFF_MS: &[u64] = &[0, 20, 50, 100, 100, 500, 500];
pub const ACTION_RETRY_BACKOFF_MS: &[u64] = &[0, 20, 100, 100, 500, 500];
pub const MAX_CALL_LOG_ENTRIES: usize = 50;
const CALL_LOG_TRUNCATED_SUFFIX: &str = " (call log truncated to the last 50 entries)";

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
    truncated: bool,
}

impl CallLog {
    pub fn push(&mut self, message: impl Into<String>) {
        let message = mask_diagnostic_text(&message.into());
        if self.truncated {
            let last = self.entries.last_mut().unwrap();
            if let Some(unmarked) = last.strip_suffix(CALL_LOG_TRUNCATED_SUFFIX) {
                last.truncate(unmarked.len());
            }
        }
        if self.entries.len() == MAX_CALL_LOG_ENTRIES {
            self.entries.remove(0);
            self.truncated = true;
        }
        self.entries.push(message);
        if self.truncated {
            self.entries
                .last_mut()
                .unwrap()
                .push_str(CALL_LOG_TRUNCATED_SUFFIX);
        }
    }
}

fn mask_diagnostic_text(message: &str) -> String {
    let message = refact_core::string_utils::redact_sensitive(message);
    let lower = message.to_ascii_lowercase();
    if !lower.contains("type=\"password\"")
        && !lower.contains("type='password'")
        && !lower.contains("type=password")
    {
        return message;
    }
    regex::Regex::new(r#"(?i)\bvalue\s*=\s*(?:\"[^\"]*\"|'[^']*'|[^\s>]+)"#)
        .unwrap()
        .replace_all(&message, "value=\"[REDACTED]\"")
        .into_owned()
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
    PrecheckFailed { description: String },
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
            Self::PrecheckFailed { description } => description.clone(),
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

    fn wait_for_navigation(&mut self) -> Result<(), ActionabilityDiagnostic> {
        Ok(())
    }

    fn locator_handlers_checkpoint(&mut self) -> Result<(), ActionabilityDiagnostic> {
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimeoutError {
    pub timeout: Duration,
    pub diagnostic: ActionabilityDiagnostic,
    pub call_log: CallLog,
    pub elapsed: Duration,
    pub attempts: u32,
    pub attached: Option<bool>,
    pub state: Option<ElementState>,
    pub receives_events: Option<bool>,
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
        elapsed: Duration,
        attempts: u32,
        attached: Option<bool>,
        state: Option<ElementState>,
        receives_events: Option<bool>,
    },
}

impl Display for ActionabilityError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout(error) => Display::fmt(error, formatter),
            Self::Failed {
                diagnostic,
                call_log,
                ..
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
    pub elapsed: Duration,
    pub attempts: u32,
    pub attached: Option<bool>,
    pub state: Option<ElementState>,
    pub receives_events: Option<bool>,
}

impl<T> ActionabilitySuccess<T> {
    pub fn diagnostics(&self, action: ActionKind) -> ActionabilityDiagnostics {
        diagnostics_from_parts(
            action,
            &self.call_log,
            false,
            self.elapsed,
            self.attempts,
            self.attached,
            self.state.as_ref(),
            self.receives_events,
            None,
        )
    }
}

impl ActionabilityError {
    pub fn diagnostics(&self, action: ActionKind) -> ActionabilityDiagnostics {
        match self {
            Self::Timeout(error) => diagnostics_from_parts(
                action,
                &error.call_log,
                true,
                error.elapsed,
                error.attempts,
                error.attached,
                error.state.as_ref(),
                error.receives_events,
                Some(&error.diagnostic),
            ),
            Self::Failed {
                diagnostic,
                call_log,
                elapsed,
                attempts,
                attached,
                state,
                receives_events,
            } => diagnostics_from_parts(
                action,
                call_log,
                false,
                *elapsed,
                *attempts,
                *attached,
                state.as_ref(),
                *receives_events,
                Some(diagnostic),
            ),
        }
    }
}

fn diagnostics_from_parts(
    action: ActionKind,
    call_log: &CallLog,
    timed_out: bool,
    elapsed: Duration,
    attempts: u32,
    attached: Option<bool>,
    state: Option<&ElementState>,
    receives_events: Option<bool>,
    diagnostic: Option<&ActionabilityDiagnostic>,
) -> ActionabilityDiagnostics {
    let required = required_states(action);
    let intercepting_element = diagnostic.and_then(|diagnostic| match diagnostic {
        ActionabilityDiagnostic::InterceptsPointerEvents { description } => {
            Some(mask_diagnostic_text(description))
        }
        _ => None,
    });
    ActionabilityDiagnostics {
        call_log: call_log.entries.clone(),
        timed_out,
        elapsed_ms: Some(elapsed.as_millis().min(u128::from(u64::MAX)) as u64),
        attempts: Some(attempts),
        attached,
        visible: if required.visible {
            state.map(|state| state.visible)
        } else {
            None
        },
        stable: if required.stable {
            state.map(|state| state.stable)
        } else {
            None
        },
        enabled: if required.enabled {
            state.map(|state| state.enabled)
        } else {
            None
        },
        editable: if required.editable {
            state.and_then(|state| state.editable)
        } else {
            None
        },
        receives_events: if required.receives_events {
            receives_events
        } else {
            None
        },
        intercepting_element,
    }
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
        let started_at = self.clock.now();
        let mut call_log = CallLog::default();
        let mut last_diagnostic = ActionabilityDiagnostic::NotFound;
        let mut locator_attempt = 0;
        let mut attempts = 0;
        let mut attached = None;
        let mut last_state = None;
        let mut receives_events = None;
        if let Err(error) = self.perform_action_prechecks(driver, &mut call_log) {
            return Err(self.enrich_error(
                error,
                started_at,
                attempts,
                attached,
                last_state,
                receives_events,
            ));
        }
        call_log.push(format!("waiting for {locator}"));

        loop {
            let delay = backoff_delay(LOCATOR_RETRY_BACKOFF_MS, locator_attempt);
            if !self.wait(delay, deadline, &mut call_log) {
                return Err(timeout_error(
                    timeout,
                    last_diagnostic,
                    call_log,
                    self.clock.now().saturating_sub(started_at),
                    attempts,
                    attached,
                    last_state,
                    receives_events,
                ));
            }
            locator_attempt += 1;

            match driver.resolve() {
                LocatorOutcome::NotFound => {
                    attached = Some(false);
                    last_diagnostic = ActionabilityDiagnostic::NotFound;
                    call_log.push(last_diagnostic.log_line());
                }
                LocatorOutcome::MultipleMatches { count } => {
                    attached = Some(true);
                    let diagnostic = ActionabilityDiagnostic::MultipleMatches { count };
                    call_log.push(diagnostic.log_line());
                    return Err(ActionabilityError::Failed {
                        diagnostic,
                        call_log,
                        elapsed: self.clock.now().saturating_sub(started_at),
                        attempts,
                        attached,
                        state: last_state,
                        receives_events,
                    });
                }
                LocatorOutcome::Found { preview } => {
                    attached = Some(true);
                    call_log.push(format!("locator resolved to {preview}"));
                    match self.run_action(
                        action,
                        deadline,
                        &mut call_log,
                        &mut last_diagnostic,
                        &mut attempts,
                        &mut last_state,
                        &mut receives_events,
                        driver,
                    ) {
                        ActionLoopResult::Done(output) => {
                            return Ok(ActionabilitySuccess {
                                output,
                                call_log,
                                elapsed: self.clock.now().saturating_sub(started_at),
                                attempts,
                                attached,
                                state: last_state,
                                receives_events,
                            });
                        }
                        ActionLoopResult::RetryLocator => {}
                        ActionLoopResult::TimedOut => {
                            return Err(timeout_error(
                                timeout,
                                last_diagnostic,
                                call_log,
                                self.clock.now().saturating_sub(started_at),
                                attempts,
                                attached,
                                last_state,
                                receives_events,
                            ));
                        }
                    }
                }
            }
        }
    }

    fn perform_action_prechecks<D: ActionabilityDriver>(
        &self,
        driver: &mut D,
        call_log: &mut CallLog,
    ) -> Result<(), ActionabilityError> {
        call_log.push("checking pending navigation before locator handlers");
        if let Err(diagnostic) = driver.wait_for_navigation() {
            call_log.push(diagnostic.log_line());
            return Err(ActionabilityError::Failed {
                diagnostic,
                call_log: call_log.clone(),
                elapsed: Duration::ZERO,
                attempts: 0,
                attached: None,
                state: None,
                receives_events: None,
            });
        }
        call_log.push("checking locator handlers");
        if let Err(diagnostic) = driver.locator_handlers_checkpoint() {
            call_log.push(diagnostic.log_line());
            return Err(ActionabilityError::Failed {
                diagnostic,
                call_log: call_log.clone(),
                elapsed: Duration::ZERO,
                attempts: 0,
                attached: None,
                state: None,
                receives_events: None,
            });
        }
        call_log.push("checking pending navigation after locator handlers");
        if let Err(diagnostic) = driver.wait_for_navigation() {
            call_log.push(diagnostic.log_line());
            return Err(ActionabilityError::Failed {
                diagnostic,
                call_log: call_log.clone(),
                elapsed: Duration::ZERO,
                attempts: 0,
                attached: None,
                state: None,
                receives_events: None,
            });
        }
        Ok(())
    }

    fn enrich_error(
        &self,
        error: ActionabilityError,
        started_at: Duration,
        attempts: u32,
        attached: Option<bool>,
        state: Option<ElementState>,
        receives_events: Option<bool>,
    ) -> ActionabilityError {
        match error {
            ActionabilityError::Timeout(mut timeout) => {
                timeout.elapsed = self.clock.now().saturating_sub(started_at);
                timeout.attempts = attempts;
                timeout.attached = attached;
                timeout.state = state;
                timeout.receives_events = receives_events;
                ActionabilityError::Timeout(timeout)
            }
            ActionabilityError::Failed {
                diagnostic,
                call_log,
                ..
            } => ActionabilityError::Failed {
                diagnostic,
                call_log,
                elapsed: self.clock.now().saturating_sub(started_at),
                attempts,
                attached,
                state,
                receives_events,
            },
        }
    }

    fn run_action<D: ActionabilityDriver>(
        &self,
        action: ActionKind,
        deadline: Deadline,
        call_log: &mut CallLog,
        last_diagnostic: &mut ActionabilityDiagnostic,
        attempts: &mut u32,
        last_state: &mut Option<ElementState>,
        receives_events: &mut Option<bool>,
        driver: &mut D,
    ) -> ActionLoopResult<D::Output> {
        let required = required_states(action);
        let state_description = state_description(required);
        let mut retry = 0_u32;

        loop {
            if deadline.expired(&self.clock) {
                return ActionLoopResult::TimedOut;
            }
            if retry == 0 {
                call_log.push(format!("attempting {} action", action.name()));
            } else {
                call_log.push(format!("retrying {} action", action.name()));
                let delay = backoff_delay(ACTION_RETRY_BACKOFF_MS, (retry - 1) as usize);
                if !self.wait(delay, deadline, call_log) {
                    return ActionLoopResult::TimedOut;
                }
            }
            *attempts = retry;

            if let Some(description) = &state_description {
                call_log.push(format!("waiting for element to be {description}"));
                match driver.element_state() {
                    Ok(state) => {
                        *last_state = Some(state.clone());
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
                Ok(output) => {
                    if required.receives_events {
                        *receives_events = Some(true);
                    }
                    return ActionLoopResult::Done(output);
                }
                Err(diagnostic) => {
                    if required.receives_events
                        && matches!(
                            diagnostic,
                            ActionabilityDiagnostic::InterceptsPointerEvents { .. }
                        )
                    {
                        *receives_events = Some(false);
                    }
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
    mut call_log: CallLog,
    elapsed: Duration,
    attempts: u32,
    attached: Option<bool>,
    state: Option<ElementState>,
    receives_events: Option<bool>,
) -> ActionabilityError {
    let reason = diagnostic.log_line();
    if call_log.entries.last().map(String::as_str) != Some(reason.as_str()) {
        call_log.push(reason);
    }
    ActionabilityError::Timeout(TimeoutError {
        timeout,
        diagnostic,
        call_log,
        elapsed,
        attempts,
        attached,
        state,
        receives_events,
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
        precheck_calls: Vec<&'static str>,
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
                precheck_calls: Vec::new(),
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

        fn wait_for_navigation(&mut self) -> Result<(), ActionabilityDiagnostic> {
            self.precheck_calls.push("navigation");
            Ok(())
        }

        fn locator_handlers_checkpoint(&mut self) -> Result<(), ActionabilityDiagnostic> {
            self.precheck_calls.push("handlers");
            Ok(())
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
        assert_eq!(
            error.call_log.entries.last().map(String::as_str),
            Some("element is not enabled")
        );
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
                "checking pending navigation before locator handlers",
                "checking locator handlers",
                "checking pending navigation after locator handlers",
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
    fn prechecks_run_navigation_handler_navigation_before_locator_resolution() {
        let clock = MockClock::default();
        let engine = ActionabilityEngine::new(clock, ActionabilityTimeouts::default());
        let mut driver = MockDriver::new();

        let success = engine
            .execute_logged("button", ActionKind::Click, &mut driver)
            .unwrap();

        assert_eq!(
            driver.precheck_calls,
            vec!["navigation", "handlers", "navigation"]
        );
        assert_eq!(
            &success.call_log.entries[..4],
            [
                "checking pending navigation before locator handlers",
                "checking locator handlers",
                "checking pending navigation after locator handlers",
                "waiting for button",
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
    fn success_after_two_retries_reports_attempts_and_end_state() {
        let clock = MockClock::default();
        let engine = ActionabilityEngine::new(clock, ActionabilityTimeouts::default());
        let mut driver = MockDriver::new();
        let mut bad = good_state();
        bad.stable = false;
        driver.states = VecDeque::from([Ok(bad.clone()), Ok(bad), Ok(good_state())]);

        let success = engine
            .execute_logged("button", ActionKind::Click, &mut driver)
            .unwrap();

        assert_eq!(success.attempts, 2);
        assert_eq!(success.attached, Some(true));
        assert_eq!(success.state, Some(good_state()));
        assert_eq!(success.receives_events, Some(true));
        let diagnostics = success.diagnostics(ActionKind::Click);
        assert_eq!(diagnostics.attempts, Some(2));
        assert_eq!(diagnostics.stable, Some(true));
        assert_eq!(diagnostics.receives_events, Some(true));
    }

    #[test]
    fn call_log_caps_entries_and_marks_the_final_reason() {
        let mut log = CallLog::default();
        for index in 0..60 {
            log.push(format!("entry {index}"));
        }

        assert_eq!(log.entries.len(), MAX_CALL_LOG_ENTRIES);
        assert_eq!(log.entries.first().unwrap(), "entry 10");
        assert_eq!(
            log.entries.last().unwrap(),
            "entry 59 (call log truncated to the last 50 entries)"
        );
    }

    #[test]
    fn call_log_masks_password_text_and_element_previews() {
        let mut log = CallLog::default();
        log.push("locator resolved to <input type=\"password\" value=\"hunter2\">");

        assert_eq!(
            log.entries,
            vec!["locator resolved to <input type=\"password\" value=\"[REDACTED]\">"]
        );
    }

    #[test]
    fn hit_target_failure_reports_masked_intercepting_element() {
        let mut call_log = CallLog::default();
        let diagnostic = ActionabilityDiagnostic::InterceptsPointerEvents {
            description: "<input type=\"password\" value=\"hunter2\">".to_string(),
        };
        call_log.push(diagnostic.log_line());
        let error = ActionabilityError::Failed {
            diagnostic,
            call_log,
            elapsed: Duration::from_millis(10),
            attempts: 1,
            attached: Some(true),
            state: Some(good_state()),
            receives_events: Some(false),
        };

        let diagnostics = error.diagnostics(ActionKind::Click);
        assert_eq!(diagnostics.receives_events, Some(false));
        assert_eq!(
            diagnostics.intercepting_element.as_deref(),
            Some("<input type=\"password\" value=\"[REDACTED]\">")
        );
        assert_eq!(
            diagnostics.call_log.last().map(String::as_str),
            Some("<input type=\"password\" value=\"[REDACTED]\"> intercepts pointer events")
        );
    }

    #[test]
    fn configured_timeout_kinds_match_playwright_mcp_defaults() {
        let timeouts = ActionabilityTimeouts::default();
        assert_eq!(timeouts.action, Duration::from_millis(5_000));
        assert_eq!(timeouts.navigation, Duration::from_millis(60_000));
        assert_eq!(timeouts.expect, Duration::from_millis(5_000));
    }
}
