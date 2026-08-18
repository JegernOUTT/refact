use std::collections::HashMap;
use std::sync::Arc;

use headless_chrome::Tab;
use headless_chrome::protocol::cdp::Page;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitScript {
    pub id: String,
    pub source: String,
}

#[derive(Default)]
pub struct InitScriptManager {
    scripts: Vec<InitScript>,
    tab_scripts: HashMap<String, Vec<(String, Page::ScriptIdentifier)>>,
    next_id: usize,
}

impl InitScriptManager {
    pub fn len(&self) -> usize {
        self.scripts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.scripts.is_empty()
    }

    pub fn ids(&self) -> Vec<String> {
        self.scripts
            .iter()
            .map(|script| script.id.clone())
            .collect()
    }

    pub fn add(&mut self, tabs: &[Arc<Tab>], source: String) -> Result<String, String> {
        self.next_id += 1;
        let id = format!("init{}", self.next_id);
        for tab in tabs {
            let identifier = add_init_script(tab, &source)?;
            self.tab_scripts
                .entry(tab.get_target_id().to_string())
                .or_default()
                .push((id.clone(), identifier));
        }
        self.scripts.push(InitScript {
            id: id.clone(),
            source,
        });
        Ok(id)
    }

    pub fn remove(&mut self, tabs: &[Arc<Tab>], id: &str) -> Result<(), String> {
        if !self.scripts.iter().any(|script| script.id == id) {
            return Err(format!("No init script with id '{id}'"));
        }
        let mut first_error = None;
        for tab in tabs {
            let Some(entries) = self.tab_scripts.get_mut(tab.get_target_id()) else {
                continue;
            };
            for (_, identifier) in entries.iter().filter(|(entry_id, _)| entry_id == id) {
                if let Err(error) = tab.call_method(Page::RemoveScriptToEvaluateOnNewDocument {
                    identifier: identifier.clone(),
                }) {
                    first_error.get_or_insert(format!("Failed to remove init script: {error}"));
                }
            }
            entries.retain(|(entry_id, _)| entry_id != id);
        }
        self.scripts.retain(|script| script.id != id);
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    pub fn reset(&mut self, tabs: &[Arc<Tab>]) -> Result<usize, String> {
        let removed = self.scripts.len();
        if removed == 0 {
            self.tab_scripts.clear();
            return Ok(0);
        }
        let mut first_error = None;
        for tab in tabs {
            let Some(entries) = self.tab_scripts.get(tab.get_target_id()) else {
                continue;
            };
            for (_, identifier) in entries {
                if let Err(error) = tab.call_method(Page::RemoveScriptToEvaluateOnNewDocument {
                    identifier: identifier.clone(),
                }) {
                    first_error.get_or_insert(format!("Failed to remove init script: {error}"));
                }
            }
        }
        self.scripts.clear();
        self.tab_scripts.clear();
        match first_error {
            Some(error) => Err(error),
            None => Ok(removed),
        }
    }

    pub fn apply_to_tab(&mut self, tab: &Tab) -> Result<(), String> {
        if self.scripts.is_empty() {
            return Ok(());
        }
        let target_id = tab.get_target_id().to_string();
        if self.tab_scripts.contains_key(&target_id) {
            return Ok(());
        }
        let mut entries = Vec::with_capacity(self.scripts.len());
        for script in &self.scripts {
            entries.push((script.id.clone(), add_init_script(tab, &script.source)?));
        }
        self.tab_scripts.insert(target_id, entries);
        Ok(())
    }
}

pub fn page_init_script(source: &str) -> Page::AddScriptToEvaluateOnNewDocument {
    Page::AddScriptToEvaluateOnNewDocument {
        source: source.to_string(),
        world_name: None,
        include_command_line_api: None,
        run_immediately: Some(false),
    }
}

fn add_init_script(tab: &Tab, source: &str) -> Result<Page::ScriptIdentifier, String> {
    tab.call_method(page_init_script(source))
        .map(|response| response.identifier)
        .map_err(|error| format!("Failed to install init script: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_minted_sequentially_and_survive_removal() {
        let mut manager = InitScriptManager::default();
        assert_eq!(manager.add(&[], "a".to_string()).unwrap(), "init1");
        assert_eq!(manager.add(&[], "b".to_string()).unwrap(), "init2");
        assert_eq!(manager.ids(), vec!["init1", "init2"]);

        manager.remove(&[], "init1").unwrap();
        assert_eq!(manager.ids(), vec!["init2"]);

        assert_eq!(manager.add(&[], "c".to_string()).unwrap(), "init3");
        assert_eq!(manager.ids(), vec!["init2", "init3"]);
    }

    #[test]
    fn removing_an_unknown_id_is_an_error() {
        let mut manager = InitScriptManager::default();
        manager.add(&[], "a".to_string()).unwrap();

        let error = manager.remove(&[], "init9").unwrap_err();
        assert!(error.contains("init9"), "{error}");
        assert_eq!(manager.len(), 1);
    }

    #[test]
    fn reset_reports_the_number_of_cleared_scripts_and_is_idempotent() {
        let mut manager = InitScriptManager::default();
        manager.add(&[], "a".to_string()).unwrap();
        manager.add(&[], "b".to_string()).unwrap();

        assert_eq!(manager.reset(&[]).unwrap(), 2);
        assert!(manager.is_empty());
        assert_eq!(manager.reset(&[]).unwrap(), 0);
    }

    #[test]
    fn init_scripts_target_the_main_world_and_do_not_run_immediately() {
        let script = page_init_script("window.flag = 1");
        assert_eq!(script.world_name, None);
        assert_eq!(script.run_immediately, Some(false));
        assert_eq!(script.source, "window.flag = 1");
    }
}
