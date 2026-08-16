use refact_integrations::browser_models::{
    BrowserLocator, BrowserStep, LocatorHandlerAction, LocatorHandlerFiring,
};

pub const DEFAULT_DISMISS_OVERLAYS_HANDLER: &str = "dismiss_overlays";
pub const MAX_LOCATOR_HANDLER_STEPS: usize = 16;

#[derive(Clone, Debug)]
pub enum LocatorHandlerOperation {
    Action(LocatorHandlerAction),
    DismissOverlays,
}

impl LocatorHandlerOperation {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Action(LocatorHandlerAction::Click) => "click",
            Self::Action(LocatorHandlerAction::Steps { .. }) => "steps",
            Self::DismissOverlays => "dismiss_overlays",
        }
    }

    pub fn steps(&self) -> Option<&[BrowserStep]> {
        match self {
            Self::Action(LocatorHandlerAction::Steps { steps }) => Some(steps),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct LocatorHandler {
    pub name: String,
    pub locator: BrowserLocator,
    pub operation: LocatorHandlerOperation,
    pub remaining_times: Option<u32>,
    pub no_wait_after: bool,
}

impl LocatorHandler {
    pub fn registered(
        name: String,
        locator: BrowserLocator,
        action: LocatorHandlerAction,
        times: Option<u32>,
        no_wait_after: bool,
    ) -> Result<Option<Self>, String> {
        if name.trim().is_empty() {
            return Err("Locator handler name must not be empty".to_string());
        }
        if let LocatorHandlerAction::Steps { steps } = &action {
            if steps.is_empty() {
                return Err("Locator handler step list must not be empty".to_string());
            }
            if steps.len() > MAX_LOCATOR_HANDLER_STEPS {
                return Err(format!(
                    "Locator handler step list exceeds the {MAX_LOCATOR_HANDLER_STEPS}-step limit"
                ));
            }
            if steps.iter().any(|step| {
                matches!(
                    step,
                    BrowserStep::AddLocatorHandler { .. }
                        | BrowserStep::RemoveLocatorHandler { .. }
                        | BrowserStep::OpenTab { .. }
                        | BrowserStep::CloseTab { .. }
                        | BrowserStep::SwitchTab { .. }
                        | BrowserStep::ListTabs
                        | BrowserStep::WaitForPopup { .. }
                )
            }) {
                return Err(
                    "Locator handler steps cannot manage handlers or browser tabs".to_string(),
                );
            }
        }
        if times == Some(0) {
            return Ok(None);
        }
        Ok(Some(Self {
            name,
            locator,
            operation: LocatorHandlerOperation::Action(action),
            remaining_times: times,
            no_wait_after,
        }))
    }

    pub fn dismiss_overlays() -> Self {
        Self {
            name: DEFAULT_DISMISS_OVERLAYS_HANDLER.to_string(),
            locator: BrowserLocator::css(
                "[id*='cookie'], [class*='cookie'], [id*='consent'], [class*='consent'], [id*='gdpr'], #onetrust-accept-btn-handler, .cc-btn.cc-dismiss, [data-testid*='cookie'], [data-testid*='accept'], dialog[open], [role='dialog'], [style*='position: fixed'], [style*='position:fixed']",
            ),
            operation: LocatorHandlerOperation::DismissOverlays,
            remaining_times: None,
            no_wait_after: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocatorHandlerProbe {
    Hidden,
    Visible,
    MultipleMatches { count: usize },
}

#[derive(Clone, Debug)]
pub struct LocatorHandlerLease {
    pub handler: LocatorHandler,
}

#[derive(Debug)]
pub struct LocatorHandlerRegistry {
    handlers: Vec<LocatorHandler>,
    running: usize,
}

impl Default for LocatorHandlerRegistry {
    fn default() -> Self {
        Self::with_default_handlers(true)
    }
}

impl LocatorHandlerRegistry {
    pub fn with_default_handlers(enabled: bool) -> Self {
        Self {
            handlers: if enabled {
                vec![LocatorHandler::dismiss_overlays()]
            } else {
                Vec::new()
            },
            running: 0,
        }
    }

    pub fn register(&mut self, handler: LocatorHandler) {
        self.unregister(&handler.name);
        self.handlers.push(handler);
    }

    pub fn unregister(&mut self, name: &str) -> bool {
        let before = self.handlers.len();
        self.handlers.retain(|handler| handler.name != name);
        before != self.handlers.len()
    }

    pub fn handlers(&self) -> &[LocatorHandler] {
        &self.handlers
    }

    pub fn get(&self, name: &str) -> Option<LocatorHandler> {
        self.handlers
            .iter()
            .find(|handler| handler.name == name)
            .cloned()
    }

    pub fn is_running(&self) -> bool {
        self.running != 0
    }

    pub fn begin(&mut self, name: &str) -> Option<LocatorHandlerLease> {
        if self.is_running() {
            return None;
        }
        let handler = self
            .handlers
            .iter()
            .find(|handler| handler.name == name && handler.remaining_times != Some(0))?
            .clone();
        self.running += 1;
        Some(LocatorHandlerLease { handler })
    }

    pub fn finish(
        &mut self,
        lease: LocatorHandlerLease,
        ok: bool,
        outcome: impl Into<String>,
    ) -> LocatorHandlerFiring {
        self.running = self.running.saturating_sub(1);
        if let Some(handler) = self
            .handlers
            .iter_mut()
            .find(|handler| handler.name == lease.handler.name)
        {
            if let Some(remaining) = &mut handler.remaining_times {
                *remaining = remaining.saturating_sub(1);
            }
        }
        self.handlers
            .retain(|handler| handler.remaining_times != Some(0));
        LocatorHandlerFiring {
            name: lease.handler.name,
            action: lease.handler.operation.label().to_string(),
            outcome: outcome.into(),
            ok,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn click_handler(name: &str, times: Option<u32>) -> LocatorHandler {
        LocatorHandler::registered(
            name.to_string(),
            BrowserLocator::css("#banner"),
            LocatorHandlerAction::Click,
            times,
            false,
        )
        .unwrap()
        .unwrap()
    }

    #[test]
    fn running_handler_blocks_nested_handler_execution() {
        let mut registry = LocatorHandlerRegistry::with_default_handlers(false);
        registry.register(click_handler("banner", None));

        let lease = registry.begin("banner").unwrap();
        assert!(registry.is_running());
        assert!(registry.begin("banner").is_none());
        registry.finish(lease, true, "clicked");
        assert!(!registry.is_running());
    }

    #[test]
    fn times_decrement_and_remove_exhausted_handler() {
        let mut registry = LocatorHandlerRegistry::with_default_handlers(false);
        registry.register(click_handler("banner", Some(2)));

        let first = registry.begin("banner").unwrap();
        registry.finish(first, true, "clicked");
        assert_eq!(registry.handlers()[0].remaining_times, Some(1));

        let second = registry.begin("banner").unwrap();
        let firing = registry.finish(second, true, "clicked again");
        assert!(registry.handlers().is_empty());
        assert_eq!(firing.name, "banner");
        assert_eq!(firing.action, "click");
        assert_eq!(firing.outcome, "clicked again");
        assert!(firing.ok);
    }

    #[test]
    fn zero_times_does_not_register() {
        let handler = LocatorHandler::registered(
            "banner".to_string(),
            BrowserLocator::css("#banner"),
            LocatorHandlerAction::Click,
            Some(0),
            false,
        )
        .unwrap();

        assert!(handler.is_none());
    }

    #[test]
    fn no_wait_after_is_stored_on_registration() {
        let handler = LocatorHandler::registered(
            "banner".to_string(),
            BrowserLocator::css("#banner"),
            LocatorHandlerAction::Click,
            None,
            true,
        )
        .unwrap()
        .unwrap();

        assert!(handler.no_wait_after);
    }

    #[test]
    fn default_registry_contains_dismiss_overlays_handler() {
        let registry = LocatorHandlerRegistry::default();

        assert_eq!(registry.handlers().len(), 1);
        assert_eq!(
            registry.handlers()[0].name,
            DEFAULT_DISMISS_OVERLAYS_HANDLER
        );
        assert!(matches!(
            registry.handlers()[0].operation,
            LocatorHandlerOperation::DismissOverlays
        ));
    }
}
