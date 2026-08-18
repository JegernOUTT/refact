use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime};
use headless_chrome::Tab;
use headless_chrome::protocol::cdp::{Page, Runtime};
use refact_integrations::browser_models::{ClockTicks, ClockTime};

pub const CLOCK_SOURCE: &str = include_str!("clock_source.js");
pub const CLOCK_GLOBAL: &str = "__refactClock";

const MAX_TICK_COMPONENTS: usize = 3;
const SECONDS_PER_COMPONENT: i64 = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockOp {
    Install { time_ms: i64 },
    FastForward { ticks_ms: i64 },
    PauseAt { time_ms: i64 },
    Resume,
    RunFor { ticks_ms: i64 },
    SetFixedTime { time_ms: i64 },
    SetSystemTime { time_ms: i64 },
}

impl ClockOp {
    pub fn method(&self) -> &'static str {
        match self {
            ClockOp::Install { .. } => "install",
            ClockOp::FastForward { .. } => "fastForward",
            ClockOp::PauseAt { .. } => "pauseAt",
            ClockOp::Resume => "resume",
            ClockOp::RunFor { .. } => "runFor",
            ClockOp::SetFixedTime { .. } => "setFixedTime",
            ClockOp::SetSystemTime { .. } => "setSystemTime",
        }
    }

    pub fn action_name(&self) -> &'static str {
        match self {
            ClockOp::Install { .. } => "clock_install",
            ClockOp::FastForward { .. } => "clock_fast_forward",
            ClockOp::PauseAt { .. } => "clock_pause_at",
            ClockOp::Resume => "clock_resume",
            ClockOp::RunFor { .. } => "clock_run_for",
            ClockOp::SetFixedTime { .. } => "clock_set_fixed_time",
            ClockOp::SetSystemTime { .. } => "clock_set_system_time",
        }
    }

    pub fn param(&self) -> Option<i64> {
        match self {
            ClockOp::Install { time_ms }
            | ClockOp::PauseAt { time_ms }
            | ClockOp::SetFixedTime { time_ms }
            | ClockOp::SetSystemTime { time_ms } => Some(*time_ms),
            ClockOp::FastForward { ticks_ms } | ClockOp::RunFor { ticks_ms } => Some(*ticks_ms),
            ClockOp::Resume => None,
        }
    }

    pub fn requires_installed(&self) -> bool {
        matches!(
            self,
            ClockOp::FastForward { .. }
                | ClockOp::PauseAt { .. }
                | ClockOp::Resume
                | ClockOp::RunFor { .. }
        )
    }

    pub fn summary(&self) -> String {
        match self {
            ClockOp::Install { time_ms } => format!("Installed fake clock at {time_ms}"),
            ClockOp::FastForward { ticks_ms } => {
                format!("Fast-forwarded clock by {ticks_ms}ms, firing each due timer at most once")
            }
            ClockOp::PauseAt { time_ms } => format!("Paused clock at {time_ms}"),
            ClockOp::Resume => "Resumed clock".to_string(),
            ClockOp::RunFor { ticks_ms } => {
                format!("Ran clock for {ticks_ms}ms, firing all due callbacks")
            }
            ClockOp::SetFixedTime { time_ms } => format!("Fixed clock time at {time_ms}"),
            ClockOp::SetSystemTime { time_ms } => format!("Set clock system time to {time_ms}"),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClockState {
    pub installed: bool,
    pub paused: bool,
}

impl ClockState {
    pub fn validate(&self, op: &ClockOp) -> Result<(), String> {
        if op.requires_installed() && !self.installed {
            return Err(format!(
                "{} requires an installed clock; run clock_install first",
                op.action_name()
            ));
        }
        if matches!(op, ClockOp::Resume) && !self.paused {
            return Err(
                "clock_resume requires a paused clock; run clock_pause_at first".to_string(),
            );
        }
        Ok(())
    }

    pub fn apply(&mut self, op: &ClockOp) {
        self.installed = true;
        match op {
            ClockOp::PauseAt { .. } => self.paused = true,
            ClockOp::Resume => self.paused = false,
            _ => {}
        }
    }
}

pub fn clock_log_script(op: &ClockOp, at_ms: i64) -> String {
    match op.param() {
        Some(param) => format!(
            "globalThis.{CLOCK_GLOBAL}.controller.log('{}', {at_ms}, {param})",
            op.method()
        ),
        None => format!(
            "globalThis.{CLOCK_GLOBAL}.controller.log('{}', {at_ms})",
            op.method()
        ),
    }
}

pub fn clock_call_script(op: &ClockOp) -> String {
    match op.param() {
        Some(param) => format!(
            "globalThis.{CLOCK_GLOBAL}.controller.{}({param})",
            op.method()
        ),
        None => format!("globalThis.{CLOCK_GLOBAL}.controller.{}()", op.method()),
    }
}

pub fn clock_uninstall_script() -> String {
    format!(
        "(() => {{ const clock = globalThis.{CLOCK_GLOBAL}; if (!clock) return false; clock.controller.uninstall(); delete globalThis.{CLOCK_GLOBAL}; return true; }})()"
    )
}

pub fn parse_clock_ticks(ticks: &ClockTicks) -> Result<i64, String> {
    match ticks {
        ClockTicks::Millis(value) => Ok(*value),
        ClockTicks::Human(text) => parse_human_ticks(text),
    }
}

fn parse_human_ticks(text: &str) -> Result<i64, String> {
    if text.is_empty() {
        return Ok(0);
    }
    let parts: Vec<&str> = text.split(':').collect();
    let shaped = parts.len() <= MAX_TICK_COMPONENTS
        && parts.iter().enumerate().all(|(index, part)| {
            let last = index + 1 == parts.len();
            part.chars().all(|character| character.is_ascii_digit())
                && (part.len() == 2 || (last && part.len() == 1))
        });
    if !shaped {
        return Err(format!(
            "Clock only understands numbers, 'mm:ss' and 'hh:mm:ss', got '{text}'"
        ));
    }
    let mut seconds = 0i64;
    for (index, part) in parts.iter().enumerate() {
        let parsed: i64 = part.parse().map_err(|_| format!("Invalid time '{text}'"))?;
        if parsed >= SECONDS_PER_COMPONENT {
            return Err(format!("Invalid time '{text}'"));
        }
        let power = (parts.len() - index - 1) as u32;
        seconds += parsed * SECONDS_PER_COMPONENT.pow(power);
    }
    Ok(seconds * 1000)
}

pub fn parse_clock_time(time: &ClockTime) -> Result<i64, String> {
    match time {
        ClockTime::UnixMillis(value) => Ok(*value),
        ClockTime::Text(text) => parse_time_text(text),
    }
}

fn parse_time_text(text: &str) -> Result<i64, String> {
    let trimmed = text.trim();
    if let Ok(parsed) = DateTime::parse_from_rfc3339(trimmed) {
        return Ok(parsed.timestamp_millis());
    }
    for format in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M",
    ] {
        if let Ok(parsed) = NaiveDateTime::parse_from_str(trimmed, format) {
            return Ok(parsed.and_utc().timestamp_millis());
        }
    }
    if let Ok(parsed) = NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") {
        return Ok(parsed.and_time(NaiveTime::MIN).and_utc().timestamp_millis());
    }
    Err(format!("Invalid date: '{text}'"))
}

pub fn current_wall_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[derive(Default)]
pub struct ClockManager {
    state: ClockState,
    scripts: Vec<String>,
    tab_scripts: HashMap<String, Vec<Page::ScriptIdentifier>>,
}

impl ClockManager {
    pub fn state(&self) -> ClockState {
        self.state
    }

    pub fn is_installed(&self) -> bool {
        self.state.installed
    }

    pub fn is_paused(&self) -> bool {
        self.state.paused
    }

    pub fn run(&mut self, tabs: &[Arc<Tab>], op: ClockOp, at_ms: i64) -> Result<(), String> {
        self.state.validate(&op)?;
        if tabs.is_empty() {
            return Err(format!(
                "{} requires an open tab in the browser session",
                op.action_name()
            ));
        }
        self.ensure_bootstrap(tabs)?;
        self.push_script(tabs, clock_log_script(&op, at_ms))?;
        let call = clock_call_script(&op);
        for tab in tabs {
            evaluate_main_world(tab, &call)?;
        }
        self.state.apply(&op);
        Ok(())
    }

    pub fn apply_to_tab(&mut self, tab: &Tab) -> Result<(), String> {
        if self.scripts.is_empty() {
            return Ok(());
        }
        let target_id = tab.get_target_id().to_string();
        if self.tab_scripts.contains_key(&target_id) {
            return Ok(());
        }
        let mut identifiers = Vec::with_capacity(self.scripts.len());
        for source in &self.scripts {
            identifiers.push(add_init_script(tab, source)?);
            evaluate_main_world(tab, source)?;
        }
        self.tab_scripts.insert(target_id, identifiers);
        Ok(())
    }

    pub fn reset(&mut self, tabs: &[Arc<Tab>]) -> Result<bool, String> {
        if self.scripts.is_empty() {
            self.tab_scripts.clear();
            self.state = ClockState::default();
            return Ok(false);
        }
        let uninstall = clock_uninstall_script();
        let mut first_error = None;
        for tab in tabs {
            if let Some(identifiers) = self.tab_scripts.get(tab.get_target_id()) {
                for identifier in identifiers {
                    if let Err(error) = tab.call_method(Page::RemoveScriptToEvaluateOnNewDocument {
                        identifier: identifier.clone(),
                    }) {
                        first_error
                            .get_or_insert(format!("Failed to remove clock init script: {error}"));
                    }
                }
            }
            if let Err(error) = evaluate_main_world(tab, &uninstall) {
                first_error.get_or_insert(error);
            }
        }
        self.scripts.clear();
        self.tab_scripts.clear();
        self.state = ClockState::default();
        match first_error {
            Some(error) => Err(error),
            None => Ok(true),
        }
    }

    fn ensure_bootstrap(&mut self, tabs: &[Arc<Tab>]) -> Result<(), String> {
        if !self.scripts.is_empty() {
            return Ok(());
        }
        self.push_script(tabs, CLOCK_SOURCE.to_string())?;
        for tab in tabs {
            evaluate_main_world(tab, CLOCK_SOURCE)?;
        }
        Ok(())
    }

    fn push_script(&mut self, tabs: &[Arc<Tab>], source: String) -> Result<(), String> {
        for tab in tabs {
            let identifier = add_init_script(tab, &source)?;
            self.tab_scripts
                .entry(tab.get_target_id().to_string())
                .or_default()
                .push(identifier);
        }
        self.scripts.push(source);
        Ok(())
    }
}

pub fn clock_init_script(source: &str) -> Page::AddScriptToEvaluateOnNewDocument {
    Page::AddScriptToEvaluateOnNewDocument {
        source: source.to_string(),
        world_name: None,
        include_command_line_api: None,
        run_immediately: Some(false),
    }
}

fn add_init_script(tab: &Tab, source: &str) -> Result<Page::ScriptIdentifier, String> {
    tab.call_method(clock_init_script(source))
        .map(|response| response.identifier)
        .map_err(|error| format!("Failed to install clock init script: {error}"))
}

fn evaluate_main_world(tab: &Tab, expression: &str) -> Result<(), String> {
    let result = tab
        .call_method(Runtime::Evaluate {
            expression: expression.to_string(),
            object_group: None,
            include_command_line_api: None,
            silent: None,
            context_id: None,
            return_by_value: Some(true),
            generate_preview: None,
            user_gesture: None,
            await_promise: Some(true),
            throw_on_side_effect: None,
            timeout: None,
            disable_breaks: None,
            repl_mode: None,
            allow_unsafe_eval_blocked_by_csp: None,
            unique_context_id: None,
            serialization_options: None,
        })
        .map_err(|error| format!("Failed to evaluate clock script: {error}"))?;
    if let Some(exception) = result.exception_details {
        return Err(format!(
            "Clock script failed: {}",
            exception
                .exception
                .as_ref()
                .and_then(|value| value.description.as_deref())
                .unwrap_or(&exception.text)
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_ticks_follow_the_playwright_format() {
        assert_eq!(parse_human_ticks("08").unwrap(), 8_000);
        assert_eq!(parse_human_ticks("8").unwrap(), 8_000);
        assert_eq!(parse_human_ticks("01:00").unwrap(), 60_000);
        assert_eq!(parse_human_ticks("30:00").unwrap(), 1_800_000);
        assert_eq!(parse_human_ticks("02:34:10").unwrap(), 9_250_000);
        assert_eq!(parse_human_ticks("").unwrap(), 0);
    }

    #[test]
    fn human_ticks_reject_malformed_and_out_of_range_components() {
        for text in ["1:2:3:4", "60:00", "01:60", "aa:bb", "1:2:3", "01:00:00:00"] {
            assert!(parse_human_ticks(text).is_err(), "accepted {text}");
        }
    }

    #[test]
    fn numeric_ticks_pass_through_as_milliseconds() {
        assert_eq!(parse_clock_ticks(&ClockTicks::Millis(1000)).unwrap(), 1000);
        assert_eq!(
            parse_clock_ticks(&ClockTicks::Human("01:00".to_string())).unwrap(),
            60_000
        );
    }

    #[test]
    fn clock_time_accepts_unix_millis_and_iso_strings() {
        assert_eq!(
            parse_clock_time(&ClockTime::UnixMillis(1_580_601_600_000)).unwrap(),
            1_580_601_600_000
        );
        assert_eq!(
            parse_clock_time(&ClockTime::Text("2020-02-02T00:00:00Z".to_string())).unwrap(),
            1_580_601_600_000
        );
        assert_eq!(
            parse_clock_time(&ClockTime::Text("2020-02-02".to_string())).unwrap(),
            1_580_601_600_000
        );
        assert_eq!(
            parse_clock_time(&ClockTime::Text("2020-02-02T00:00:00".to_string())).unwrap(),
            1_580_601_600_000
        );
    }

    #[test]
    fn clock_time_rejects_unparseable_text() {
        assert!(parse_clock_time(&ClockTime::Text("not-a-date".to_string())).is_err());
        assert!(parse_clock_time(&ClockTime::Text(String::new())).is_err());
    }

    #[test]
    fn ordering_guard_requires_install_before_time_travel() {
        let state = ClockState::default();
        for op in [
            ClockOp::FastForward { ticks_ms: 1000 },
            ClockOp::RunFor { ticks_ms: 1000 },
            ClockOp::PauseAt { time_ms: 1000 },
            ClockOp::Resume,
        ] {
            let error = state.validate(&op).unwrap_err();
            assert!(error.contains("clock_install"), "{error}");
        }
    }

    #[test]
    fn ordering_guard_allows_standalone_time_setters() {
        let state = ClockState::default();
        assert!(state.validate(&ClockOp::Install { time_ms: 0 }).is_ok());
        assert!(state
            .validate(&ClockOp::SetFixedTime { time_ms: 0 })
            .is_ok());
        assert!(state
            .validate(&ClockOp::SetSystemTime { time_ms: 0 })
            .is_ok());
    }

    #[test]
    fn resume_requires_a_preceding_pause() {
        let mut state = ClockState::default();
        state.apply(&ClockOp::Install { time_ms: 0 });
        let error = state.validate(&ClockOp::Resume).unwrap_err();
        assert!(error.contains("clock_pause_at"), "{error}");
        state.apply(&ClockOp::PauseAt { time_ms: 1000 });
        assert!(state.validate(&ClockOp::Resume).is_ok());
        state.apply(&ClockOp::Resume);
        assert!(state.validate(&ClockOp::Resume).is_err());
    }

    #[test]
    fn fast_forward_and_run_for_keep_the_paused_state() {
        let mut state = ClockState::default();
        state.apply(&ClockOp::Install { time_ms: 0 });
        state.apply(&ClockOp::PauseAt { time_ms: 10 });
        state.apply(&ClockOp::FastForward { ticks_ms: 10 });
        state.apply(&ClockOp::RunFor { ticks_ms: 10 });
        assert!(state.paused);
        assert!(state.installed);
    }

    #[test]
    fn standalone_time_setters_mark_the_clock_installed() {
        let mut state = ClockState::default();
        state.apply(&ClockOp::SetFixedTime { time_ms: 5 });
        assert!(state.installed);
        assert!(state
            .validate(&ClockOp::FastForward { ticks_ms: 1 })
            .is_ok());
    }

    #[test]
    fn log_scripts_carry_the_playwright_method_names_and_wall_time() {
        assert_eq!(
            clock_log_script(&ClockOp::FastForward { ticks_ms: 1000 }, 42),
            "globalThis.__refactClock.controller.log('fastForward', 42, 1000)"
        );
        assert_eq!(
            clock_log_script(&ClockOp::Resume, 42),
            "globalThis.__refactClock.controller.log('resume', 42)"
        );
        assert_eq!(
            clock_log_script(&ClockOp::SetFixedTime { time_ms: 7 }, 42),
            "globalThis.__refactClock.controller.log('setFixedTime', 42, 7)"
        );
    }

    #[test]
    fn call_scripts_target_the_controller_methods() {
        assert_eq!(
            clock_call_script(&ClockOp::RunFor { ticks_ms: 250 }),
            "globalThis.__refactClock.controller.runFor(250)"
        );
        assert_eq!(
            clock_call_script(&ClockOp::Resume),
            "globalThis.__refactClock.controller.resume()"
        );
        assert_eq!(
            clock_call_script(&ClockOp::PauseAt { time_ms: -5 }),
            "globalThis.__refactClock.controller.pauseAt(-5)"
        );
    }

    #[test]
    fn uninstall_script_restores_originals_and_clears_the_global() {
        let script = clock_uninstall_script();
        assert!(script.contains("controller.uninstall()"));
        assert!(script.contains("delete globalThis.__refactClock"));
    }

    #[test]
    fn every_op_maps_to_a_distinct_action_and_controller_method() {
        let ops = [
            ClockOp::Install { time_ms: 0 },
            ClockOp::FastForward { ticks_ms: 0 },
            ClockOp::PauseAt { time_ms: 0 },
            ClockOp::Resume,
            ClockOp::RunFor { ticks_ms: 0 },
            ClockOp::SetFixedTime { time_ms: 0 },
            ClockOp::SetSystemTime { time_ms: 0 },
        ];
        let mut actions: Vec<&str> = ops.iter().map(ClockOp::action_name).collect();
        let mut methods: Vec<&str> = ops.iter().map(ClockOp::method).collect();
        actions.sort_unstable();
        actions.dedup();
        methods.sort_unstable();
        methods.dedup();
        assert_eq!(actions.len(), ops.len());
        assert_eq!(methods.len(), ops.len());
    }

    #[test]
    fn vendored_source_keeps_its_public_clock_contract() {
        for marker in [
            "globalThis.__refactClock = inject(globalThis, 'chromium')",
            "if (globalThis.__refactClock)",
            "Christian Johansen",
            "Modifications copyright (c) Microsoft Corporation.",
            "setTimeout:",
            "clearTimeout:",
            "setInterval:",
            "clearInterval:",
            "requestAnimationFrame:",
            "cancelAnimationFrame:",
            "requestIdleCallback:",
            "cancelIdleCallback:",
            "Date:",
            "performance:",
        ] {
            assert!(CLOCK_SOURCE.contains(marker), "missing {marker}");
        }
    }

    #[test]
    fn init_scripts_never_run_immediately_in_the_current_document() {
        let script = clock_init_script(&clock_log_script(&ClockOp::RunFor { ticks_ms: 1000 }, 0));
        assert_eq!(script.run_immediately, Some(false));
        assert_eq!(script.world_name, None);
        assert!(script.source.contains("controller.log('runFor'"));
    }

    #[test]
    fn clock_notice_names_source_and_commit() {
        let notice = include_str!("../injected/NOTICE.md");
        assert!(notice.contains("../src/clock_source.js"));
        assert!(notice.contains("packages/injected/src/clock.ts"));
        assert!(notice.contains("d5a185a894ab3ab17ff77a44e116a1339c6bdaed"));
        assert!(notice.contains("Christian Johansen"));
    }

    #[test]
    fn manager_starts_uninstalled_and_resets_to_a_clean_state() {
        let mut manager = ClockManager::default();
        assert!(!manager.is_installed());
        assert!(!manager.is_paused());
        assert_eq!(manager.reset(&[]).unwrap(), false);
        assert_eq!(manager.state(), ClockState::default());
    }

    #[test]
    fn manager_rejects_out_of_order_steps_before_touching_the_browser() {
        let mut manager = ClockManager::default();
        let error = manager
            .run(&[], ClockOp::FastForward { ticks_ms: 1000 }, 0)
            .unwrap_err();
        assert!(error.contains("clock_install"), "{error}");
        assert!(!manager.is_installed());
    }

    #[test]
    fn manager_requires_a_tab_once_the_step_order_is_valid() {
        let mut manager = ClockManager::default();
        let error = manager
            .run(&[], ClockOp::Install { time_ms: 0 }, 0)
            .unwrap_err();
        assert!(error.contains("requires an open tab"), "{error}");
    }

    #[test]
    fn wall_clock_helper_returns_a_plausible_unix_timestamp() {
        assert!(current_wall_ms() > 1_600_000_000_000);
    }
}
