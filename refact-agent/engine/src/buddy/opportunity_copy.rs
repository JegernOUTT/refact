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
        BuddyPage::TaskWorkspace {
            ..
        } => Some("Open the task's workspace"),
        BuddyPage::KnowledgeGraph => Some("Open Knowledge Graph"),
        BuddyPage::Worktrees => Some("Open Worktrees"),
        BuddyPage::SetupMode {
            ..
        } => Some("Start the guided setup"),
    }
}

pub(super) fn uncertainty_for_body(body: &str) -> &'static str {
    if body.contains("\n\nSuggested next steps:\n") {
        "Buddy has not changed anything and cannot carry out these steps from here."
    } else {
        "Buddy noticed this from project signals and has not changed anything."
    }
}
