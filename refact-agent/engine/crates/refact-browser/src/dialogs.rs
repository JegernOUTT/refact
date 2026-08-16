use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use headless_chrome::Tab;
use headless_chrome::protocol::cdp::Page;
use headless_chrome::protocol::cdp::types::Event;
use refact_integrations::browser_models::{DialogAction, DialogInfo, DialogType};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogResponse {
    pub accept: bool,
    pub prompt_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogDecision {
    pub response: DialogResponse,
    pub report: DialogInfo,
}

#[derive(Default)]
struct DialogState {
    armed: Option<DialogResponse>,
    reports: Vec<DialogInfo>,
    installed_tabs: HashSet<String>,
    dialog_open: bool,
}

#[derive(Clone, Default)]
pub struct DialogManager {
    state: Arc<Mutex<DialogState>>,
}

impl DialogManager {
    pub fn arm(&self, accept: bool, prompt_text: Option<String>) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        if state.dialog_open {
            return Err(
                "A dialog is already open; HandleDialog arms only the next dialog".to_string(),
            );
        }
        state.armed = Some(DialogResponse {
            accept,
            prompt_text,
        });
        Ok(())
    }

    pub fn decide(
        &self,
        dialog_type: DialogType,
        message: &str,
        default_value: &str,
    ) -> DialogDecision {
        let mut state = self.state.lock().unwrap();
        state.dialog_open = true;
        let armed = state.armed.take();
        let automatic = armed.is_none();
        let response = armed.unwrap_or_else(|| DialogResponse {
            accept: dialog_type == DialogType::Beforeunload,
            prompt_text: None,
        });
        let action = if response.accept {
            DialogAction::Accepted
        } else {
            DialogAction::Dismissed
        };
        DialogDecision {
            response,
            report: DialogInfo {
                dialog_type,
                message: refact_core::string_utils::redact_sensitive(message),
                default_value: refact_core::string_utils::redact_sensitive(default_value),
                action,
                automatic,
            },
        }
    }

    pub fn record(&self, report: DialogInfo) {
        self.state.lock().unwrap().reports.push(report);
    }

    pub fn take_reports(&self) -> Vec<DialogInfo> {
        std::mem::take(&mut self.state.lock().unwrap().reports)
    }

    pub fn install(&self, tab: &Tab) -> Result<(), String> {
        let target_id = tab.get_target_id().to_string();
        {
            let mut state = self.state.lock().unwrap();
            if !state.installed_tabs.insert(target_id) {
                return Ok(());
            }
        }

        let manager = self.clone();
        let dialog = tab.get_dialog();
        tab.add_event_listener(Arc::new(move |event: &Event| match event {
            Event::PageJavascriptDialogOpening(event) => {
                let dialog_type = dialog_type_from_cdp(&event.params.Type);
                let decision = manager.decide(
                    dialog_type,
                    &event.params.message,
                    event.params.default_prompt.as_deref().unwrap_or_default(),
                );
                let result = if decision.response.accept {
                    dialog.accept(decision.response.prompt_text.clone())
                } else {
                    dialog.dismiss()
                };
                if result.is_ok() {
                    manager.record(decision.report);
                }
            }
            Event::PageJavascriptDialogClosed(_) => {
                manager.state.lock().unwrap().dialog_open = false;
            }
            _ => {}
        }))
        .map_err(|error| format!("Failed to add browser dialog listener: {error}"))?;
        Ok(())
    }
}

fn dialog_type_from_cdp(value: &Page::DialogType) -> DialogType {
    match value {
        Page::DialogType::Alert => DialogType::Alert,
        Page::DialogType::Confirm => DialogType::Confirm,
        Page::DialogType::Prompt => DialogType::Prompt,
        Page::DialogType::Beforeunload => DialogType::Beforeunload,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unarmed_dialog_decision_table_never_leaves_a_dialog_open() {
        let cases = [
            (DialogType::Alert, false),
            (DialogType::Confirm, false),
            (DialogType::Prompt, false),
            (DialogType::Beforeunload, true),
        ];

        for (dialog_type, accept) in cases {
            let manager = DialogManager::default();
            let decision = manager.decide(dialog_type, "message", "default");
            assert_eq!(decision.response.accept, accept);
            assert!(decision.report.automatic);
            assert_eq!(
                decision.report.action,
                if accept {
                    DialogAction::Accepted
                } else {
                    DialogAction::Dismissed
                }
            );
        }
    }

    #[test]
    fn armed_response_is_consumed_by_only_the_next_dialog() {
        let manager = DialogManager::default();
        manager
            .arm(true, Some("typed response".to_string()))
            .unwrap();

        let armed = manager.decide(DialogType::Prompt, "Question", "Default");
        assert!(armed.response.accept);
        assert_eq!(
            armed.response.prompt_text.as_deref(),
            Some("typed response")
        );
        assert!(!armed.report.automatic);

        manager.state.lock().unwrap().dialog_open = false;
        let next = manager.decide(DialogType::Confirm, "Again", "");
        assert!(!next.response.accept);
        assert!(next.report.automatic);
    }

    #[test]
    fn report_masks_dialog_message_and_default_value() {
        let manager = DialogManager::default();
        let decision = manager.decide(
            DialogType::Prompt,
            "Enter password=hunter2",
            "token=secret-value",
        );

        assert_eq!(decision.report.dialog_type, DialogType::Prompt);
        assert_eq!(decision.report.message, "Enter password=[REDACTED]");
        assert_eq!(decision.report.default_value, "token=[REDACTED]");
        assert_eq!(decision.report.action, DialogAction::Dismissed);
        assert!(decision.report.automatic);
    }
}
