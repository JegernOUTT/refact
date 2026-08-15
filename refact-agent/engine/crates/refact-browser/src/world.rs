use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use headless_chrome::Tab;
use headless_chrome::protocol::cdp::types::Event;
use headless_chrome::protocol::cdp::{Page, Runtime};
use serde_json::Value;

use crate::wrapped_bootstrap;

pub const UTILITY_WORLD_NAME: &str = "__refact_utility__";
pub const BINDING_NAME: &str = "__refact_binding";
pub const INJECTED_INSTANCE_NAME: &str = "__refact_injected__";

pub type BindingCallback = Arc<dyn Fn(BindingCall) + Send + Sync>;

#[derive(Clone, Debug, PartialEq)]
pub struct BindingCall {
    pub tab_target_id: String,
    pub execution_context_id: Runtime::ExecutionContextId,
    pub payload: Value,
}

#[derive(Default)]
struct TabWorldState {
    installed: bool,
    frame_contexts: HashMap<Page::FrameId, Runtime::ExecutionContextId>,
    context_frames: HashMap<Runtime::ExecutionContextId, Page::FrameId>,
    utility_contexts: HashSet<Runtime::ExecutionContextId>,
    initialized_contexts: HashSet<Runtime::ExecutionContextId>,
}

#[derive(Default)]
struct WorldState {
    tabs: HashMap<String, TabWorldState>,
    binding_callbacks: HashMap<String, Vec<BindingCallback>>,
}

#[derive(Clone, Default)]
pub struct WorldManager {
    state: Arc<Mutex<WorldState>>,
}

impl WorldManager {
    pub fn register_binding_callback(&self, name: impl Into<String>, callback: BindingCallback) {
        self.state
            .lock()
            .unwrap()
            .binding_callbacks
            .entry(name.into())
            .or_default()
            .push(callback);
    }

    pub fn ensure_utility_world(&self, tab: &Tab) -> Result<Runtime::ExecutionContextId, String> {
        self.install_for_tab(tab)?;
        let frame_tree = tab
            .call_method(Page::GetFrameTree(None))
            .map_err(|error| format!("Failed to read browser frame tree: {error}"))?
            .frame_tree;
        let main_frame_id = frame_tree.frame.id.clone();
        let mut frame_ids = Vec::new();
        collect_frame_ids(&frame_tree, &mut frame_ids);
        for frame_id in frame_ids {
            let context_id = match self.context_for_frame(tab.get_target_id(), &frame_id) {
                Some(context_id) => context_id,
                None => {
                    tab.call_method(Page::CreateIsolatedWorld {
                        frame_id: frame_id.clone(),
                        world_name: Some(UTILITY_WORLD_NAME.to_string()),
                        grant_univeral_access: Some(true),
                    })
                    .map_err(|error| {
                        format!(
                            "Failed to create browser utility world for frame {frame_id}: {error}"
                        )
                    })?
                    .execution_context_id
                }
            };
            self.record_context(tab.get_target_id(), frame_id, context_id);
            self.ensure_bootstrap(tab, context_id)?;
        }
        self.context_for_frame(tab.get_target_id(), &main_frame_id)
            .ok_or_else(|| "Browser utility world has no main-frame context".to_string())
    }

    pub fn eval_in_utility(&self, tab: &Tab, expression: &str) -> Result<Value, String> {
        let context_id = self.ensure_utility_world(tab)?;
        let result = tab
            .call_method(runtime_evaluate(expression.to_string(), context_id))
            .map_err(|error| format!("Failed to evaluate in browser utility world: {error}"))?;
        if let Some(exception) = result.exception_details {
            return Err(format!(
                "Browser utility-world evaluation failed: {}",
                exception_message(&exception)
            ));
        }
        Ok(result.result.value.unwrap_or(Value::Null))
    }

    pub fn call_injected(&self, tab: &Tab, method: &str, args: Value) -> Result<Value, String> {
        if !args.is_array() {
            return Err("Injected call arguments must be a JSON array".to_string());
        }
        let method = serde_json::to_string(method)
            .map_err(|error| format!("Failed to serialize injected method: {error}"))?;
        let args = serde_json::to_string(&args)
            .map_err(|error| format!("Failed to serialize injected arguments: {error}"))?;
        self.eval_in_utility(
            tab,
            &format!(
                "(() => {{ const instance = globalThis[{instance_name:?}]; if (!instance) throw new Error('RefactInjected is not installed'); const method = instance[{method}]; if (typeof method !== 'function') throw new Error('Unknown RefactInjected method'); return method.apply(instance, {args}); }})()",
                instance_name = INJECTED_INSTANCE_NAME,
            ),
        )
    }

    fn install_for_tab(&self, tab: &Tab) -> Result<(), String> {
        let target_id = tab.get_target_id().to_string();
        if self.is_installed(&target_id) {
            return Ok(());
        }

        let listener_manager = self.clone();
        let listener_target_id = target_id.clone();
        tab.add_event_listener(Arc::new(move |event: &Event| {
            listener_manager.handle_event(&listener_target_id, event);
        }))
        .map_err(|error| format!("Failed to add browser utility-world event listener: {error}"))?;

        tab.expose_function(BINDING_NAME, Arc::new(|_| {}))
            .map_err(|error| format!("Failed to register browser binding callback: {error}"))?;
        tab.call_method(Runtime::RemoveBinding {
            name: BINDING_NAME.to_string(),
        })
        .map_err(|error| format!("Failed to scope browser binding: {error}"))?;
        tab.call_method(Runtime::AddBinding {
            name: BINDING_NAME.to_string(),
            execution_context_id: None,
            execution_context_name: Some(UTILITY_WORLD_NAME.to_string()),
        })
        .map_err(|error| format!("Failed to add browser utility-world binding: {error}"))?;
        tab.call_method(clear_main_world_binding_script())
            .map_err(|error| format!("Failed to isolate browser binding: {error}"))?;
        tab.call_method(Runtime::Enable(None))
            .map_err(|error| format!("Failed to enable browser Runtime domain: {error}"))?;
        tab.call_method(utility_init_script()).map_err(|error| {
            format!("Failed to install browser utility-world bootstrap: {error}")
        })?;

        self.state
            .lock()
            .unwrap()
            .tabs
            .entry(target_id)
            .or_default()
            .installed = true;
        Ok(())
    }

    fn ensure_bootstrap(
        &self,
        tab: &Tab,
        context_id: Runtime::ExecutionContextId,
    ) -> Result<(), String> {
        if self.is_initialized(tab.get_target_id(), context_id) {
            return Ok(());
        }
        let probe = format!(
            "typeof globalThis[{instance_name:?}] === 'object'",
            instance_name = INJECTED_INSTANCE_NAME,
        );
        let installed = tab
            .call_method(runtime_evaluate(probe, context_id))
            .map_err(|error| format!("Failed to inspect browser utility world: {error}"))?;
        if installed.exception_details.is_none()
            && installed.result.value == Some(Value::Bool(true))
        {
            self.mark_initialized(tab.get_target_id(), context_id);
            return Ok(());
        }
        let result = tab
            .call_method(runtime_evaluate(wrapped_bootstrap(), context_id))
            .map_err(|error| format!("Failed to bootstrap browser utility world: {error}"))?;
        if let Some(exception) = result.exception_details {
            return Err(format!(
                "Browser utility-world bootstrap failed: {}",
                exception_message(&exception)
            ));
        }
        let installed = tab
            .call_method(runtime_evaluate(
                format!(
                    "typeof globalThis[{instance_name:?}] === 'object'",
                    instance_name = INJECTED_INSTANCE_NAME,
                ),
                context_id,
            ))
            .map_err(|error| format!("Failed to confirm browser utility world: {error}"))?;
        if installed.result.value != Some(Value::Bool(true)) {
            return Err("Browser utility-world bootstrap did not install its instance".to_string());
        }
        self.mark_initialized(tab.get_target_id(), context_id);
        Ok(())
    }

    fn handle_event(&self, target_id: &str, event: &Event) {
        match event {
            Event::RuntimeExecutionContextCreated(event) => self.context_created(
                target_id,
                event.params.context.id,
                &event.params.context.name,
            ),
            Event::RuntimeExecutionContextDestroyed(event) => {
                self.context_destroyed(target_id, event.params.execution_context_id)
            }
            Event::RuntimeExecutionContextsCleared(_) => self.contexts_cleared(target_id),
            Event::PageFrameNavigated(event) => {
                self.frame_navigated(target_id, &event.params.frame.id)
            }
            Event::RuntimeBindingCalled(event) if event.params.name == BINDING_NAME => self
                .dispatch_binding(
                    target_id,
                    event.params.execution_context_id,
                    &event.params.payload,
                ),
            _ => {}
        }
    }

    fn dispatch_binding(
        &self,
        target_id: &str,
        execution_context_id: Runtime::ExecutionContextId,
        payload: &str,
    ) {
        let is_utility_context = self
            .state
            .lock()
            .unwrap()
            .tabs
            .get(target_id)
            .map(|tab| tab.utility_contexts.contains(&execution_context_id))
            .unwrap_or(false);
        if !is_utility_context {
            return;
        }
        let Ok(payload) = serde_json::from_str::<Value>(payload) else {
            return;
        };
        let Some(name) = payload.get("name").and_then(Value::as_str) else {
            return;
        };
        let callbacks = self
            .state
            .lock()
            .unwrap()
            .binding_callbacks
            .get(name)
            .cloned()
            .unwrap_or_default();
        let call = BindingCall {
            tab_target_id: target_id.to_string(),
            execution_context_id,
            payload,
        };
        for callback in callbacks {
            callback(call.clone());
        }
    }

    fn is_installed(&self, target_id: &str) -> bool {
        self.state
            .lock()
            .unwrap()
            .tabs
            .get(target_id)
            .map(|tab| tab.installed)
            .unwrap_or(false)
    }

    fn context_for_frame(
        &self,
        target_id: &str,
        frame_id: &Page::FrameId,
    ) -> Option<Runtime::ExecutionContextId> {
        self.state
            .lock()
            .unwrap()
            .tabs
            .get(target_id)?
            .frame_contexts
            .get(frame_id)
            .copied()
    }

    fn is_initialized(&self, target_id: &str, context_id: Runtime::ExecutionContextId) -> bool {
        self.state
            .lock()
            .unwrap()
            .tabs
            .get(target_id)
            .map(|tab| tab.initialized_contexts.contains(&context_id))
            .unwrap_or(false)
    }

    fn mark_initialized(&self, target_id: &str, context_id: Runtime::ExecutionContextId) {
        self.state
            .lock()
            .unwrap()
            .tabs
            .entry(target_id.to_string())
            .or_default()
            .initialized_contexts
            .insert(context_id);
    }

    fn context_created(
        &self,
        target_id: &str,
        context_id: Runtime::ExecutionContextId,
        context_name: &str,
    ) {
        if context_name == UTILITY_WORLD_NAME {
            self.state
                .lock()
                .unwrap()
                .tabs
                .entry(target_id.to_string())
                .or_default()
                .utility_contexts
                .insert(context_id);
        }
    }

    fn record_context(
        &self,
        target_id: &str,
        frame_id: Page::FrameId,
        context_id: Runtime::ExecutionContextId,
    ) {
        let mut state = self.state.lock().unwrap();
        let tab = state.tabs.entry(target_id.to_string()).or_default();
        if let Some(previous_context) = tab.frame_contexts.insert(frame_id.clone(), context_id) {
            tab.context_frames.remove(&previous_context);
            tab.utility_contexts.remove(&previous_context);
            tab.initialized_contexts.remove(&previous_context);
        }
        tab.context_frames.insert(context_id, frame_id);
        tab.utility_contexts.insert(context_id);
    }

    fn context_destroyed(&self, target_id: &str, context_id: Runtime::ExecutionContextId) {
        let mut state = self.state.lock().unwrap();
        let Some(tab) = state.tabs.get_mut(target_id) else {
            return;
        };
        tab.utility_contexts.remove(&context_id);
        tab.initialized_contexts.remove(&context_id);
        if let Some(frame_id) = tab.context_frames.remove(&context_id) {
            if tab.frame_contexts.get(&frame_id) == Some(&context_id) {
                tab.frame_contexts.remove(&frame_id);
            }
        }
    }

    fn frame_navigated(&self, target_id: &str, frame_id: &Page::FrameId) {
        let mut state = self.state.lock().unwrap();
        let Some(tab) = state.tabs.get_mut(target_id) else {
            return;
        };
        if let Some(context_id) = tab.frame_contexts.remove(frame_id) {
            tab.context_frames.remove(&context_id);
            tab.utility_contexts.remove(&context_id);
            tab.initialized_contexts.remove(&context_id);
        }
    }

    fn contexts_cleared(&self, target_id: &str) {
        let mut state = self.state.lock().unwrap();
        let Some(tab) = state.tabs.get_mut(target_id) else {
            return;
        };
        tab.frame_contexts.clear();
        tab.context_frames.clear();
        tab.utility_contexts.clear();
        tab.initialized_contexts.clear();
    }
}

fn collect_frame_ids(frame_tree: &Page::FrameTree, output: &mut Vec<Page::FrameId>) {
    output.push(frame_tree.frame.id.clone());
    if let Some(children) = &frame_tree.child_frames {
        for child in children {
            collect_frame_ids(child, output);
        }
    }
}

fn runtime_evaluate(
    expression: String,
    context_id: Runtime::ExecutionContextId,
) -> Runtime::Evaluate {
    Runtime::Evaluate {
        expression,
        object_group: None,
        include_command_line_api: None,
        silent: None,
        context_id: Some(context_id),
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
    }
}

fn exception_message(exception: &Runtime::ExceptionDetails) -> String {
    exception
        .exception
        .as_ref()
        .and_then(|value| value.description.as_deref())
        .unwrap_or(&exception.text)
        .to_string()
}

pub(crate) fn utility_init_script() -> Page::AddScriptToEvaluateOnNewDocument {
    Page::AddScriptToEvaluateOnNewDocument {
        source: wrapped_bootstrap(),
        world_name: Some(UTILITY_WORLD_NAME.to_string()),
        include_command_line_api: None,
        run_immediately: Some(true),
    }
}

fn clear_main_world_binding_script() -> Page::AddScriptToEvaluateOnNewDocument {
    Page::AddScriptToEvaluateOnNewDocument {
        source: format!("delete globalThis[{BINDING_NAME:?}]"),
        world_name: None,
        include_command_line_api: None,
        run_immediately: Some(true),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TAB: &str = "tab";
    const FRAME: &str = "frame";

    #[test]
    fn created_then_recorded_context_is_tracked() {
        let manager = WorldManager::default();
        manager.context_created(TAB, 7, UTILITY_WORLD_NAME);
        manager.record_context(TAB, FRAME.to_string(), 7);
        assert_eq!(manager.context_for_frame(TAB, &FRAME.to_string()), Some(7));
    }

    #[test]
    fn destroyed_context_removes_frame_mapping() {
        let manager = WorldManager::default();
        manager.record_context(TAB, FRAME.to_string(), 7);
        manager.context_destroyed(TAB, 7);
        assert_eq!(manager.context_for_frame(TAB, &FRAME.to_string()), None);
    }

    #[test]
    fn navigation_before_destroy_invalidates_only_old_context() {
        let manager = WorldManager::default();
        manager.record_context(TAB, FRAME.to_string(), 7);
        manager.frame_navigated(TAB, &FRAME.to_string());
        manager.context_destroyed(TAB, 7);
        manager.record_context(TAB, FRAME.to_string(), 8);
        assert_eq!(manager.context_for_frame(TAB, &FRAME.to_string()), Some(8));
    }

    #[test]
    fn replacement_context_ignores_late_old_destruction() {
        let manager = WorldManager::default();
        manager.record_context(TAB, FRAME.to_string(), 7);
        manager.record_context(TAB, FRAME.to_string(), 8);
        manager.context_destroyed(TAB, 7);
        assert_eq!(manager.context_for_frame(TAB, &FRAME.to_string()), Some(8));
    }

    #[test]
    fn cleared_event_removes_every_context() {
        let manager = WorldManager::default();
        manager.record_context(TAB, FRAME.to_string(), 7);
        manager.record_context(TAB, "child".to_string(), 8);
        manager.contexts_cleared(TAB);
        assert_eq!(manager.context_for_frame(TAB, &FRAME.to_string()), None);
        assert_eq!(manager.context_for_frame(TAB, &"child".to_string()), None);
    }

    #[test]
    fn utility_bootstrap_targets_named_world() {
        let script = utility_init_script();
        assert_eq!(script.world_name.as_deref(), Some(UTILITY_WORLD_NAME));
        assert_eq!(script.run_immediately, Some(true));
        assert!(script.source.contains(INJECTED_INSTANCE_NAME));
    }

    #[test]
    fn binding_cleanup_targets_main_world() {
        let script = clear_main_world_binding_script();
        assert_eq!(script.world_name, None);
        assert!(script.source.contains(BINDING_NAME));
    }

    #[test]
    fn binding_name_is_stable() {
        assert_eq!(BINDING_NAME, "__refact_binding");
    }

    #[test]
    fn binding_callbacks_receive_named_payloads_once() {
        let manager = WorldManager::default();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let callback_calls = calls.clone();
        manager.register_binding_callback(
            "record",
            Arc::new(move |call| callback_calls.lock().unwrap().push(call)),
        );
        manager.context_created(TAB, 9, UTILITY_WORLD_NAME);
        manager.dispatch_binding(TAB, 9, r#"{"name":"record","payload":{"ok":true}}"#);
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].execution_context_id, 9);
        assert_eq!(calls[0].payload["payload"]["ok"], true);
    }
}
