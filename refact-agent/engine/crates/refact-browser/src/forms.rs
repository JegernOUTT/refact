use headless_chrome::Tab;
use refact_integrations::browser_models::{
    ActionabilityDiagnostics, ElementInfo, FieldKind, FillStrategy,
};
use serde_json::Value;

use crate::{
    ActionKind, ActionabilityDiagnostic, ActionabilityDriver, ActionabilityEngine,
    ActionabilityTimeouts, CdpKeyboardDispatcher, CdpMouseDispatcher, ElementHandle,
    HitTargetController, HitTargetPoint, HitTargetResult, Keyboard, LocatorOutcome, Mouse,
    MouseButton, SystemClock, WorldManager,
};

#[derive(Clone, Debug, PartialEq)]
pub struct FillOutcome {
    pub strategy: FillStrategy,
    pub verified: Option<bool>,
    pub retries: u32,
    pub actionability: Option<ActionabilityDiagnostics>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClearOutcome {
    pub strategy: Option<FillStrategy>,
    pub verified: Option<bool>,
    pub retries: u32,
    pub actionability: Option<ActionabilityDiagnostics>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SelectOptionOutcome {
    pub selected: Vec<String>,
    pub actionability: Option<ActionabilityDiagnostics>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CheckedOutcome {
    pub checked: bool,
    pub changed: bool,
    pub verified: bool,
    pub actionability: Option<ActionabilityDiagnostics>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FormError {
    pub message: String,
    pub retries: u32,
    pub actionability: Option<ActionabilityDiagnostics>,
}

impl FormError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retries: 0,
            actionability: None,
        }
    }

    fn after_retries(message: impl Into<String>, retries: u32) -> Self {
        Self {
            message: message.into(),
            retries,
            actionability: None,
        }
    }

    fn with_actionability(mut self, actionability: Option<ActionabilityDiagnostics>) -> Self {
        self.actionability = actionability;
        self
    }
}

struct FormActionabilityDriver<'a> {
    tab: &'a Tab,
    world: &'a WorldManager,
    handle: &'a ElementHandle,
    action: ActionKind,
    perform_action: bool,
}

impl ActionabilityDriver for FormActionabilityDriver<'_> {
    type Output = ();

    fn resolve(&mut self) -> LocatorOutcome {
        LocatorOutcome::Found {
            preview: "<resolved form element>".to_string(),
        }
    }

    fn element_state(&mut self) -> Result<crate::ElementState, ActionabilityDiagnostic> {
        self.world
            .element_states(self.tab, self.handle)
            .map_err(|error| match error {
                crate::HandleError::Invalidated { .. } => ActionabilityDiagnostic::Detached,
                _ => ActionabilityDiagnostic::PrecheckFailed {
                    description: error.to_string(),
                },
            })
    }

    fn perform(&mut self) -> Result<Self::Output, ActionabilityDiagnostic> {
        if !self.perform_action {
            return Ok(());
        }
        trusted_click(self.tab, self.world, self.handle, self.action)
    }
}

fn run_actionability(
    tab: &Tab,
    world: &WorldManager,
    handle: &ElementHandle,
    action: ActionKind,
    perform_action: bool,
) -> Result<ActionabilityDiagnostics, FormError> {
    let engine = ActionabilityEngine::new(SystemClock::default(), ActionabilityTimeouts::default());
    let mut driver = FormActionabilityDriver {
        tab,
        world,
        handle,
        action,
        perform_action,
    };
    match engine.execute_form_logged("resolved form element", action, &mut driver) {
        Ok(success) => Ok(success.diagnostics(action)),
        Err(error) => {
            let diagnostics = error.diagnostics(action);
            Err(FormError::after_retries(
                error.to_string(),
                diagnostics.attempts.unwrap_or_default(),
            )
            .with_actionability(Some(diagnostics)))
        }
    }
}

fn trusted_click(
    tab: &Tab,
    world: &WorldManager,
    handle: &ElementHandle,
    action: ActionKind,
) -> Result<(), ActionabilityDiagnostic> {
    let dispatcher = CdpMouseDispatcher::new(tab);
    let point = dispatcher.clickable_point(handle).map_err(|error| {
        ActionabilityDiagnostic::PrecheckFailed {
            description: error.to_string(),
        }
    })?;
    let hit_target = HitTargetController::default();
    let hit_target_point = HitTargetPoint {
        x: point.x,
        y: point.y,
    };
    match hit_target.expect_hit_target(tab, world, handle, hit_target_point) {
        Ok(HitTargetResult::Done | HitTargetResult::Skipped) => {}
        Ok(HitTargetResult::Intercepted { description }) => {
            return Err(ActionabilityDiagnostic::InterceptsPointerEvents { description });
        }
        Ok(HitTargetResult::NotConnected) => return Err(ActionabilityDiagnostic::Detached),
        Err(error) => {
            return Err(ActionabilityDiagnostic::PrecheckFailed {
                description: error.to_string(),
            });
        }
    }
    let token = hit_target
        .install_interceptor(tab, world, handle, action, Some(hit_target_point))
        .map_err(|error| ActionabilityDiagnostic::PrecheckFailed {
            description: error.to_string(),
        })?;
    let keyboard = Keyboard::new(CdpKeyboardDispatcher::new(tab));
    let mut mouse = Mouse::new(dispatcher, &keyboard);
    let action_result = mouse.click(point.x, point.y, MouseButton::Left);
    let hit_result = hit_target.take_result(tab, world, token);
    action_result.map_err(|error| ActionabilityDiagnostic::PrecheckFailed {
        description: error.to_string(),
    })?;
    match hit_result {
        Ok(HitTargetResult::Done | HitTargetResult::Skipped) => Ok(()),
        Ok(HitTargetResult::Intercepted { description }) => {
            Err(ActionabilityDiagnostic::InterceptsPointerEvents { description })
        }
        Ok(HitTargetResult::NotConnected) => Err(ActionabilityDiagnostic::Detached),
        Err(error) => Err(ActionabilityDiagnostic::PrecheckFailed {
            description: error.to_string(),
        }),
    }
}

pub fn choose_fill_strategies(field_kind: &FieldKind) -> Vec<FillStrategy> {
    match field_kind {
        FieldKind::ContentEditable => vec![
            FillStrategy::NativeTyping,
            FillStrategy::ContentEditablePath,
            FillStrategy::ClickAndType,
        ],
        FieldKind::Textarea => vec![
            FillStrategy::NativeTyping,
            FillStrategy::DomValueSetter,
            FillStrategy::NativePrototypeSetter,
            FillStrategy::ClickAndType,
        ],
        FieldKind::Select
        | FieldKind::Checkbox
        | FieldKind::Radio
        | FieldKind::FileInput
        | FieldKind::HiddenInput => Vec::new(),
        _ => vec![
            FillStrategy::NativeTyping,
            FillStrategy::DomValueSetter,
            FillStrategy::NativePrototypeSetter,
            FillStrategy::ClickAndType,
        ],
    }
}

pub fn diagnostic_value(field_kind: &FieldKind, value: &str) -> String {
    if matches!(field_kind, FieldKind::PasswordInput) {
        "[REDACTED]".to_string()
    } else {
        value.to_string()
    }
}

pub fn fill(
    tab: &Tab,
    world: &WorldManager,
    handle: &ElementHandle,
    info: &ElementInfo,
    text: &str,
    clear_first: bool,
    verify: bool,
) -> Result<FillOutcome, FormError> {
    if info.readonly {
        return Err(FormError::new("Element is readonly"));
    }
    let strategies = choose_fill_strategies(&info.field_kind);
    if strategies.is_empty() {
        return Err(FormError::new(format!(
            "Cannot fill {:?} — use select_option or check/uncheck instead",
            info.field_kind
        )));
    }
    let actionability = run_actionability(tab, world, handle, ActionKind::Fill, false)?;
    let mut executor = CdpFillExecutor { tab, world, handle };
    run_fill(
        &mut executor,
        &info.field_kind,
        &strategies,
        text,
        clear_first,
        verify,
    )
    .map(|mut outcome| {
        outcome.actionability = Some(actionability.clone());
        outcome
    })
    .map_err(|error| error.with_actionability(Some(actionability)))
}

pub fn clear(
    tab: &Tab,
    world: &WorldManager,
    handle: &ElementHandle,
    info: &ElementInfo,
    verify: bool,
) -> Result<ClearOutcome, FormError> {
    match info.field_kind {
        FieldKind::Checkbox | FieldKind::Radio => {
            return Err(FormError::new(format!(
                "Use uncheck instead for <{}> ({:?})",
                info.tag, info.field_kind
            )));
        }
        FieldKind::FileInput => {
            return Err(FormError::new(
                "Security restrictions prevent clearing file inputs programmatically",
            ));
        }
        FieldKind::HiddenInput => {
            return Err(FormError::new(format!(
                "Element <{}> is a hidden input",
                info.tag
            )));
        }
        FieldKind::Select => {
            let actionability =
                run_actionability(tab, world, handle, ActionKind::SelectOption, false)?;
            let result = call_json(
                tab,
                world,
                handle,
                r#"function() {
  const injected = globalThis.__refact_injected__;
  const el = this.tagName === 'LABEL' && this.control ? this.control : this;
  if (!el || el.tagName !== 'SELECT') throw new Error('Element is not a <select> element');
  let hadEmpty = false;
  el.selectedIndex = -1;
  for (const option of el.options) {
    if (option.value === '' || option.text.trim() === '') {
      option.selected = true;
      hadEmpty = true;
      break;
    }
  }
  el.dispatchEvent(new Event('input', { bubbles: true, composed: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
  return { had_empty_option: hadEmpty };
}"#,
                Vec::new(),
            )
            .map_err(FormError::new)?;
            let had_empty = result
                .get("had_empty_option")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if verify && !had_empty {
                return Err(
                    FormError::new("No option with empty value/text exists to select")
                        .with_actionability(Some(actionability)),
                );
            }
            return Ok(ClearOutcome {
                strategy: None,
                verified: verify.then_some(true),
                retries: 0,
                actionability: Some(actionability),
            });
        }
        _ => {}
    }
    let outcome = fill(tab, world, handle, info, "", true, verify)?;
    Ok(ClearOutcome {
        strategy: Some(outcome.strategy),
        verified: outcome.verified,
        retries: outcome.retries,
        actionability: outcome.actionability,
    })
}

pub fn select_option(
    tab: &Tab,
    world: &WorldManager,
    handle: &ElementHandle,
    value: &str,
) -> Result<SelectOptionOutcome, FormError> {
    let actionability = run_actionability(tab, world, handle, ActionKind::SelectOption, false)?;
    let result = call_json(
        tab,
        world,
        handle,
        r#"function(value) {
  const injected = globalThis.__refact_injected__;
  const el = this.tagName === 'LABEL' && this.control ? this.control : this;
  if (!el || el.tagName !== 'SELECT') throw new Error('Element is not a <select> element');
  const normalized = value.replace(/\s+/g, ' ').trim();
  let selected;
  for (const option of el.options) {
    const label = option.label.replace(/\s+/g, ' ').trim();
    if (option.value === value || option.label === value || label === normalized) {
      selected = option;
      break;
    }
  }
  if (!selected) return { error_code: 'options_not_found' };
  if (selected.disabled || (selected.parentElement && selected.parentElement.tagName === 'OPTGROUP' && selected.parentElement.disabled))
    return { error_code: 'option_not_enabled' };
  for (const option of el.options) option.selected = false;
  selected.selected = true;
  el.dispatchEvent(new Event('input', { bubbles: true, composed: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
  return { selected: [selected.value] };
}"#,
        vec![Value::String(value.to_string())],
    )
    .map_err(|error| FormError::new(error).with_actionability(Some(actionability.clone())))?;
    if let Some(code) = result.get("error_code").and_then(Value::as_str) {
        return Err(
            FormError::new(select_option_diagnostic(code)).with_actionability(Some(actionability))
        );
    }
    let selected = result
        .get("selected")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    Ok(SelectOptionOutcome {
        selected,
        actionability: Some(actionability),
    })
}

pub fn set_checked(
    tab: &Tab,
    world: &WorldManager,
    handle: &ElementHandle,
    check: bool,
) -> Result<CheckedOutcome, FormError> {
    let before = checked_state(tab, world, handle).map_err(FormError::new)?;
    check_transition(before.supported, before.radio, before.checked, check)?;
    let needs_click = needs_checked_click(before.checked, check);
    let action = if check {
        ActionKind::Check
    } else {
        ActionKind::Uncheck
    };
    let actionability = run_actionability(tab, world, handle, action, needs_click)?;
    if !needs_click {
        return Ok(checked_outcome(before.checked, check, Some(actionability)));
    }
    let after = checked_state(tab, world, handle)
        .map_err(|error| FormError::new(error).with_actionability(Some(actionability.clone())))?;
    verify_checked_transition(before.checked, after.checked, check, Some(actionability))
}

trait FillExecutor {
    fn execute(
        &mut self,
        strategy: &FillStrategy,
        text: &str,
        clear_first: bool,
    ) -> Result<(), String>;
    fn matches(&mut self, expected: &str) -> Result<bool, String>;
}

fn run_fill<E: FillExecutor>(
    executor: &mut E,
    field_kind: &FieldKind,
    strategies: &[FillStrategy],
    text: &str,
    clear_first: bool,
    verify: bool,
) -> Result<FillOutcome, FormError> {
    let mut retries = 0;
    let mut last_error = "No fill strategy was attempted".to_string();
    for strategy in strategies {
        match executor.execute(strategy, text, clear_first) {
            Ok(()) => {
                if verify {
                    match executor.matches(text) {
                        Ok(true) => {
                            return Ok(FillOutcome {
                                strategy: strategy.clone(),
                                verified: Some(true),
                                retries,
                                actionability: None,
                            });
                        }
                        Ok(false) => {
                            last_error = "Verification failed: value mismatch".to_string();
                        }
                        Err(error) => {
                            last_error = format!("Verification error: {error}");
                        }
                    }
                } else {
                    return Ok(FillOutcome {
                        strategy: strategy.clone(),
                        verified: None,
                        retries,
                        actionability: None,
                    });
                }
            }
            Err(error) => {
                last_error = sanitize_error(field_kind, text, &error);
            }
        }
        retries += 1;
    }
    Err(FormError::after_retries(last_error, retries))
}

struct CdpFillExecutor<'a> {
    tab: &'a Tab,
    world: &'a WorldManager,
    handle: &'a ElementHandle,
}

impl FillExecutor for CdpFillExecutor<'_> {
    fn execute(
        &mut self,
        strategy: &FillStrategy,
        text: &str,
        clear_first: bool,
    ) -> Result<(), String> {
        if matches!(strategy, FillStrategy::NativeTyping) {
            let preparation = call_json(
                self.tab,
                self.world,
                self.handle,
                PLAYWRIGHT_FILL_PREPARE,
                vec![Value::String(text.to_string())],
            )?;
            if preparation.get("done").and_then(Value::as_bool) == Some(true) {
                return Ok(());
            }
            if preparation.get("needs_input").and_then(Value::as_bool) != Some(true) {
                return Err("Playwright fill did not prepare the field for input".to_string());
            }
            let mut keyboard = Keyboard::new(CdpKeyboardDispatcher::new(self.tab));
            if text.is_empty() {
                keyboard.press("Delete", None)
            } else {
                keyboard.insert_text(text)
            }
        } else {
            call_json(
                self.tab,
                self.world,
                self.handle,
                fallback_fill_function(strategy),
                vec![Value::String(text.to_string()), Value::Bool(clear_first)],
            )?;
            Ok(())
        }
    }

    fn matches(&mut self, expected: &str) -> Result<bool, String> {
        let result = call_json(
            self.tab,
            self.world,
            self.handle,
            VERIFY_VALUE,
            vec![Value::String(expected.to_string())],
        )?;
        Ok(result.get("matches").and_then(Value::as_bool) == Some(true))
    }
}

const PLAYWRIGHT_FILL_PREPARE: &str = r#"function(value) {
  const injected = globalThis.__refact_injected__;
  const el = this.tagName === 'LABEL' && this.control ? this.control : this;
  if (!el || !el.isConnected) throw new Error('Element is not attached');
  if (el.tagName === 'INPUT') {
    const type = el.type.toLowerCase();
    const directTypes = new Set(['color', 'date', 'time', 'datetime-local', 'month', 'range', 'week']);
    const textTypes = new Set(['', 'email', 'number', 'password', 'search', 'tel', 'text', 'url']);
    if (!directTypes.has(type) && !textTypes.has(type)) throw new Error(`Input of type "${type}" cannot be filled`);
    if (type === 'number') {
      value = value.trim();
      if (Number.isNaN(Number(value))) throw new Error('Cannot type text into input[type=number]');
    }
    if (type === 'color') value = value.toLowerCase();
    if (directTypes.has(type)) {
      value = value.trim();
      el.focus();
      el.value = value;
      if (el.value !== value) throw new Error('Malformed value');
      el.dispatchEvent(new Event('input', { bubbles: true, composed: true }));
      el.dispatchEvent(new Event('change', { bubbles: true }));
      return { done: true };
    }
    el.select();
    el.focus();
    return { needs_input: true };
  }
  if (el.tagName === 'TEXTAREA') {
    el.selectionStart = 0;
    el.selectionEnd = el.value.length;
    el.focus();
    return { needs_input: true };
  }
  if (!el.isContentEditable) throw new Error('Element is not an <input>, <textarea> or [contenteditable] element');
  el.focus();
  const range = el.ownerDocument.createRange();
  range.selectNodeContents(el);
  const selection = el.ownerDocument.defaultView.getSelection();
  if (selection) {
    selection.removeAllRanges();
    selection.addRange(range);
  }
  return { needs_input: true };
}"#;

const VERIFY_VALUE: &str = r#"function(expected) {
  const injected = globalThis.__refact_injected__;
  const el = this.tagName === 'LABEL' && this.control ? this.control : this;
  if (!el || !el.isConnected) throw new Error('Element is not attached');
  if (el.isContentEditable) return { matches: (el.innerText || el.textContent || '') === expected };
  let normalized = expected;
  const type = (el.type || '').toLowerCase();
  if (['date', 'time', 'datetime-local', 'month', 'range', 'week', 'number'].includes(type)) normalized = normalized.trim();
  if (type === 'color') normalized = normalized.trim().toLowerCase();
  if (type === 'password') return { matches: String(el.value || '').length === normalized.length };
  return { matches: String(el.value === undefined ? '' : el.value) === normalized };
}"#;

fn fallback_fill_function(strategy: &FillStrategy) -> &'static str {
    match strategy {
        FillStrategy::DomValueSetter => {
            r#"function(text) {
  const injected = globalThis.__refact_injected__;
  const el = this.tagName === 'LABEL' && this.control ? this.control : this;
  el.scrollIntoView({ block: 'center', behavior: 'instant' });
  el.focus();
  el.value = text;
  el.dispatchEvent(new Event('input', { bubbles: true, composed: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
  return true;
}"#
        }
        FillStrategy::NativePrototypeSetter => {
            r#"function(text) {
  const injected = globalThis.__refact_injected__;
  const el = this.tagName === 'LABEL' && this.control ? this.control : this;
  const proto = el.tagName === 'TEXTAREA' ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  const setter = Object.getOwnPropertyDescriptor(proto, 'value')?.set;
  if (!setter) throw new Error('No value setter on prototype');
  el.scrollIntoView({ block: 'center', behavior: 'instant' });
  el.focus();
  setter.call(el, text);
  el.dispatchEvent(new Event('input', { bubbles: true, composed: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
  return true;
}"#
        }
        FillStrategy::ContentEditablePath => {
            r#"function(text) {
  const injected = globalThis.__refact_injected__;
  const el = this.tagName === 'LABEL' && this.control ? this.control : this;
  if (!el.isContentEditable) throw new Error('Element is not contentEditable');
  el.scrollIntoView({ block: 'center', behavior: 'instant' });
  el.focus();
  document.execCommand('selectAll', false, null);
  document.execCommand('delete', false, null);
  document.execCommand('insertText', false, text);
  return true;
}"#
        }
        FillStrategy::ClickAndType => {
            r#"function(text) {
  const injected = globalThis.__refact_injected__;
  const el = this.tagName === 'LABEL' && this.control ? this.control : this;
  el.scrollIntoView({ block: 'center', behavior: 'instant' });
  el.click();
  el.focus();
  if (el.select) el.select(); else document.execCommand('selectAll', false, null);
  document.execCommand('delete', false, null);
  for (const ch of text) {
    el.dispatchEvent(new KeyboardEvent('keydown', { key: ch, bubbles: true }));
    el.dispatchEvent(new KeyboardEvent('keypress', { key: ch, bubbles: true }));
    document.execCommand('insertText', false, ch);
    el.dispatchEvent(new KeyboardEvent('keyup', { key: ch, bubbles: true }));
  }
  el.dispatchEvent(new Event('input', { bubbles: true, composed: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
  return true;
}"#
        }
        FillStrategy::NativeTyping => PLAYWRIGHT_FILL_PREPARE,
    }
}

fn call_json(
    tab: &Tab,
    world: &WorldManager,
    handle: &ElementHandle,
    function: &str,
    arguments: Vec<Value>,
) -> Result<Value, String> {
    let value = world
        .call_function_on(tab, handle, function, arguments)
        .map_err(|error| error.to_string())?;
    if let Some(error) = value.get("error").and_then(Value::as_str) {
        return Err(error.to_string());
    }
    Ok(value)
}

#[derive(Clone, Copy)]
struct CheckedState {
    supported: bool,
    checked: bool,
    radio: bool,
}

fn checked_state(
    tab: &Tab,
    world: &WorldManager,
    handle: &ElementHandle,
) -> Result<CheckedState, String> {
    let state = call_json(
        tab,
        world,
        handle,
        r#"function() {
  const injected = globalThis.__refact_injected__;
  const el = this.tagName === 'LABEL' && this.control ? this.control : this;
  if (!el || !el.isConnected) throw new Error('Element is not attached');
  const role = el.getAttribute('role');
  const native = el.tagName === 'INPUT' && (el.type === 'checkbox' || el.type === 'radio');
  const aria = role === 'checkbox' || role === 'switch' || role === 'radio';
  return {
    supported: native || aria,
    checked: native ? !!el.checked : el.getAttribute('aria-checked') === 'true',
    radio: (native && el.type === 'radio') || role === 'radio'
  };
}"#,
        Vec::new(),
    )?;
    Ok(CheckedState {
        supported: state
            .get("supported")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        checked: state
            .get("checked")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        radio: state.get("radio").and_then(Value::as_bool).unwrap_or(false),
    })
}

fn ensure_check_allowed(supported: bool, radio: bool, check: bool) -> Result<(), FormError> {
    if !supported {
        return Err(FormError::new(
            "Element is not a checkbox, radio button, or switch",
        ));
    }
    if radio && !check {
        return Err(FormError::new(
            "Cannot uncheck radio button. Radio buttons can only be unchecked by selecting another radio button in the same group.",
        ));
    }
    Ok(())
}

fn check_transition(
    supported: bool,
    radio: bool,
    current: bool,
    wanted: bool,
) -> Result<(), FormError> {
    ensure_check_allowed(supported, radio, wanted)?;
    if !needs_checked_click(current, wanted) {
        return Ok(());
    }
    Ok(())
}

fn needs_checked_click(current: bool, wanted: bool) -> bool {
    current != wanted
}

fn checked_outcome(
    before: bool,
    after: bool,
    actionability: Option<ActionabilityDiagnostics>,
) -> CheckedOutcome {
    CheckedOutcome {
        checked: after,
        changed: before != after,
        verified: true,
        actionability,
    }
}

fn verify_checked_transition(
    before: bool,
    after: bool,
    wanted: bool,
    actionability: Option<ActionabilityDiagnostics>,
) -> Result<CheckedOutcome, FormError> {
    if after != wanted {
        return Err(
            FormError::new("Clicking the checkbox did not change its state")
                .with_actionability(actionability),
        );
    }
    Ok(checked_outcome(before, after, actionability))
}

fn select_option_diagnostic(code: &str) -> String {
    match code {
        "options_not_found" => "Select option did not find some options".to_string(),
        "option_not_enabled" => "Option being selected is not enabled".to_string(),
        other => format!("Select option failed: {other}"),
    }
}

fn sanitize_error(field_kind: &FieldKind, text: &str, error: &str) -> String {
    if matches!(field_kind, FieldKind::PasswordInput) && !text.is_empty() {
        error.replace(text, "[REDACTED]")
    } else {
        error.to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;

    struct FakeExecutor {
        executions: Vec<FillStrategy>,
        results: VecDeque<Result<(), String>>,
        matches: VecDeque<Result<bool, String>>,
    }

    impl FillExecutor for FakeExecutor {
        fn execute(
            &mut self,
            strategy: &FillStrategy,
            _text: &str,
            _clear_first: bool,
        ) -> Result<(), String> {
            self.executions.push(strategy.clone());
            self.results.pop_front().unwrap_or(Ok(()))
        }

        fn matches(&mut self, _expected: &str) -> Result<bool, String> {
            self.matches.pop_front().unwrap_or(Ok(true))
        }
    }

    #[test]
    fn strategy_selection_starts_with_playwright_cdp_fill() {
        assert_eq!(
            choose_fill_strategies(&FieldKind::TextInput),
            vec![
                FillStrategy::NativeTyping,
                FillStrategy::DomValueSetter,
                FillStrategy::NativePrototypeSetter,
                FillStrategy::ClickAndType,
            ]
        );
        assert_eq!(
            choose_fill_strategies(&FieldKind::ContentEditable),
            vec![
                FillStrategy::NativeTyping,
                FillStrategy::ContentEditablePath,
                FillStrategy::ClickAndType,
            ]
        );
        assert!(choose_fill_strategies(&FieldKind::Select).is_empty());
        assert!(choose_fill_strategies(&FieldKind::Radio).is_empty());
    }

    #[test]
    fn verification_mismatch_advances_the_pyramid() {
        let mut executor = FakeExecutor {
            executions: Vec::new(),
            results: VecDeque::from([Ok(()), Ok(())]),
            matches: VecDeque::from([Ok(false), Ok(true)]),
        };
        let outcome = run_fill(
            &mut executor,
            &FieldKind::TextInput,
            &choose_fill_strategies(&FieldKind::TextInput),
            "value",
            true,
            true,
        )
        .unwrap();
        assert_eq!(
            executor.executions,
            vec![FillStrategy::NativeTyping, FillStrategy::DomValueSetter]
        );
        assert_eq!(outcome.strategy, FillStrategy::DomValueSetter);
        assert_eq!(outcome.verified, Some(true));
        assert_eq!(outcome.retries, 1);
    }

    #[test]
    fn strategy_error_advances_the_pyramid() {
        let mut executor = FakeExecutor {
            executions: Vec::new(),
            results: VecDeque::from([Err("blocked".to_string()), Ok(())]),
            matches: VecDeque::from([Ok(true)]),
        };
        let outcome = run_fill(
            &mut executor,
            &FieldKind::Textarea,
            &choose_fill_strategies(&FieldKind::Textarea),
            "value",
            true,
            true,
        )
        .unwrap();
        assert_eq!(outcome.strategy, FillStrategy::DomValueSetter);
        assert_eq!(outcome.retries, 1);
    }

    #[test]
    fn form_actions_use_the_required_shared_actionability_rows() {
        assert_eq!(
            crate::required_states(ActionKind::Fill),
            crate::RequiredStates {
                visible: true,
                stable: false,
                receives_events: false,
                enabled: true,
                editable: true,
            }
        );
        assert_eq!(
            crate::required_states(ActionKind::SelectOption),
            crate::RequiredStates {
                visible: true,
                stable: false,
                receives_events: false,
                enabled: true,
                editable: false,
            }
        );
        for action in [ActionKind::Check, ActionKind::Uncheck] {
            assert_eq!(
                crate::required_states(action),
                crate::RequiredStates {
                    visible: true,
                    stable: true,
                    receives_events: true,
                    enabled: true,
                    editable: false,
                }
            );
        }
    }

    #[test]
    fn radio_uncheck_is_rejected() {
        let error = ensure_check_allowed(true, true, false).unwrap_err();
        assert_eq!(
            error.message,
            "Cannot uncheck radio button. Radio buttons can only be unchecked by selecting another radio button in the same group."
        );
        assert!(ensure_check_allowed(true, true, true).is_ok());
    }

    #[test]
    fn checked_transition_skips_idempotent_clicks_and_verifies_changes() {
        assert!(check_transition(true, false, true, true).is_ok());
        assert!(!needs_checked_click(true, true));
        assert_eq!(
            checked_outcome(true, true, None),
            CheckedOutcome {
                checked: true,
                changed: false,
                verified: true,
                actionability: None,
            }
        );
        assert_eq!(
            verify_checked_transition(false, true, true, None).unwrap(),
            CheckedOutcome {
                checked: true,
                changed: true,
                verified: true,
                actionability: None,
            }
        );
        assert_eq!(
            verify_checked_transition(false, false, true, None)
                .unwrap_err()
                .message,
            "Clicking the checkbox did not change its state"
        );
    }

    #[test]
    fn select_option_diagnostics_match_playwright_failures() {
        assert_eq!(
            select_option_diagnostic("options_not_found"),
            "Select option did not find some options"
        );
        assert_eq!(
            select_option_diagnostic("option_not_enabled"),
            "Option being selected is not enabled"
        );
    }

    #[test]
    fn password_diagnostics_mask_echoed_values() {
        assert_eq!(
            diagnostic_value(&FieldKind::PasswordInput, "hunter2"),
            "[REDACTED]"
        );
        assert_eq!(
            sanitize_error(&FieldKind::PasswordInput, "hunter2", "rejected hunter2"),
            "rejected [REDACTED]"
        );
        assert_eq!(diagnostic_value(&FieldKind::TextInput, "hello"), "hello");
    }

    #[test]
    fn playwright_fill_prepares_selection_before_cdp_input() {
        assert!(PLAYWRIGHT_FILL_PREPARE.contains("el.select()"));
        assert!(PLAYWRIGHT_FILL_PREPARE.contains("selection.addRange(range)"));
        assert!(PLAYWRIGHT_FILL_PREPARE.contains("Cannot type text into input[type=number]"));
        assert!(PLAYWRIGHT_FILL_PREPARE.contains("Malformed value"));
    }
}
