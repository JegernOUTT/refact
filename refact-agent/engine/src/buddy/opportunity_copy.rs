use super::types::BuddyPage;

pub(super) fn page_action(page: &BuddyPage) -> Option<&'static str> {
    match page {
        BuddyPage::Buddy => None,
        BuddyPage::Stats => Some("Open Stats"),
        BuddyPage::Customization => Some("Open Customization in Settings"),
        BuddyPage::Providers => Some("Open Providers in Settings"),
        BuddyPage::DefaultModels => Some("Open Default models in Settings"),
        BuddyPage::Integrations => Some("Open Integrations in Settings"),
        BuddyPage::Extensions => Some("Open Extensions in Settings"),
        BuddyPage::MarketplaceHub => Some("Open Marketplace"),
        BuddyPage::Marketplace => Some("Open MCP Marketplace"),
        BuddyPage::SkillsMarketplace => Some("Open Skills Marketplace"),
        BuddyPage::CommandsMarketplace => Some("Open Commands Marketplace"),
        BuddyPage::DelegatesMarketplace => Some("Open Subagents Marketplace"),
        BuddyPage::TasksList => Some("Open Tasks"),
        BuddyPage::TaskWorkspace { .. } => Some("Open the task's workspace"),
        BuddyPage::KnowledgeGraph => Some("Open Knowledge Graph"),
        BuddyPage::Worktrees => Some("Open Worktrees"),
        BuddyPage::SetupMode { .. } => Some("Start the guided setup"),
    }
}

pub(super) fn uncertainty_for_body(body: &str) -> &'static str {
    if body.contains("\n\nSuggested next steps:\n") {
        "Buddy has not changed anything and cannot carry out these steps from here."
    } else {
        "Buddy noticed this from project signals and has not changed anything."
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_buddy_page_has_human_copy_except_the_current_buddy_page() {
        let pages = [
            BuddyPage::Buddy,
            BuddyPage::Stats,
            BuddyPage::Customization,
            BuddyPage::Providers,
            BuddyPage::DefaultModels,
            BuddyPage::Integrations,
            BuddyPage::Extensions,
            BuddyPage::MarketplaceHub,
            BuddyPage::Marketplace,
            BuddyPage::SkillsMarketplace,
            BuddyPage::CommandsMarketplace,
            BuddyPage::DelegatesMarketplace,
            BuddyPage::TasksList,
            BuddyPage::TaskWorkspace {
                task_id: "opaque-task-id".into(),
            },
            BuddyPage::KnowledgeGraph,
            BuddyPage::Worktrees,
            BuddyPage::SetupMode {
                mode: "setup_agents_md".into(),
            },
        ];

        for page in pages {
            let Some(label) = page_action(&page) else {
                assert!(matches!(page, BuddyPage::Buddy));
                continue;
            };
            assert!(!label.trim().is_empty());
            assert!(!label.contains('_'));
            assert!(
                label.split_whitespace().count() >= 2,
                "id-like label: {label}"
            );
            assert!(!label.chars().any(|character| character.is_ascii_digit()));
            assert!(!label.contains("opaque-task-id"));
            assert!(!label.contains("setup_agents_md"));
        }
    }
}
