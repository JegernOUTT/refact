use crate::settings::AutonomyLevel;
use crate::types::BuddyAction;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionExecution {
    Navigate,
    Prefill,
    Execute,
    Deny,
}

pub fn action_kind_str(action: &BuddyAction) -> &'static str {
    match action {
        BuddyAction::OpenPage { .. } => "open_page",
        BuddyAction::LaunchInvestigationChat { .. } => "launch_investigation_chat",
        BuddyAction::DraftSkill { .. } => "draft_skill",
        BuddyAction::DraftCommand { .. } => "draft_command",
        BuddyAction::DraftDelegate { .. } => "draft_delegate",
        BuddyAction::DraftMode { .. } => "draft_mode",
        BuddyAction::DraftAgentsMdPatch { .. } => "draft_agents_md_patch",
        BuddyAction::DraftDefaultsChange { .. } => "draft_defaults_change",
        BuddyAction::DraftCustomizationChange { .. } => "draft_customization_change",
        BuddyAction::OfferMarketplaceInstall { .. } => "offer_marketplace_install",
        BuddyAction::CreatePulseReport { .. } => "create_pulse_report",
        BuddyAction::ApplyMemoryBatch { .. } => "apply_memory_batch",
        BuddyAction::ApplyConfigPatch { .. } => "apply_config_patch",
        BuddyAction::AcceptQuest { .. } => "accept_quest",
        BuddyAction::OpenBuddyConversation { .. } => "open_buddy_conversation",
        BuddyAction::Dismiss => "dismiss",
    }
}

pub fn action_execution(level: AutonomyLevel, action: &BuddyAction) -> ActionExecution {
    match action {
        BuddyAction::OpenPage { .. }
        | BuddyAction::OpenBuddyConversation { .. }
        | BuddyAction::Dismiss => ActionExecution::Navigate,
        BuddyAction::LaunchInvestigationChat { .. } => match level {
            AutonomyLevel::ReadOnly => ActionExecution::Deny,
            _ => ActionExecution::Execute,
        },
        BuddyAction::DraftSkill { .. }
        | BuddyAction::DraftCommand { .. }
        | BuddyAction::DraftDelegate { .. }
        | BuddyAction::DraftMode { .. }
        | BuddyAction::DraftAgentsMdPatch { .. }
        | BuddyAction::DraftDefaultsChange { .. }
        | BuddyAction::DraftCustomizationChange { .. }
        | BuddyAction::CreatePulseReport { .. } => match level {
            AutonomyLevel::ReadOnly => ActionExecution::Deny,
            _ => ActionExecution::Prefill,
        },
        BuddyAction::OfferMarketplaceInstall { .. } => match level {
            AutonomyLevel::ReadOnly => ActionExecution::Deny,
            _ => ActionExecution::Execute,
        },
        BuddyAction::ApplyMemoryBatch { .. } => match level {
            AutonomyLevel::ReadOnly => ActionExecution::Deny,
            _ => ActionExecution::Execute,
        },
        BuddyAction::ApplyConfigPatch { .. } => match level {
            AutonomyLevel::ReadOnly => ActionExecution::Deny,
            AutonomyLevel::Suggest => ActionExecution::Prefill,
            AutonomyLevel::Propose | AutonomyLevel::SafeAuto => ActionExecution::Execute,
        },
        BuddyAction::AcceptQuest { .. } => ActionExecution::Execute,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch_action() -> BuddyAction {
        BuddyAction::ApplyConfigPatch {
            draft_id: "d1".to_string(),
            target_path: ".refact/buddy/x.yaml".to_string(),
        }
    }

    #[test]
    fn config_patch_execution_by_level() {
        assert_eq!(
            action_execution(AutonomyLevel::Propose, &patch_action()),
            ActionExecution::Execute
        );
        assert_eq!(
            action_execution(AutonomyLevel::Suggest, &patch_action()),
            ActionExecution::Prefill
        );
        assert_eq!(
            action_execution(AutonomyLevel::SafeAuto, &patch_action()),
            ActionExecution::Execute
        );
        assert_eq!(
            action_execution(AutonomyLevel::ReadOnly, &patch_action()),
            ActionExecution::Deny
        );
    }

    #[test]
    fn navigation_always_allowed() {
        for level in [
            AutonomyLevel::ReadOnly,
            AutonomyLevel::Suggest,
            AutonomyLevel::Propose,
            AutonomyLevel::SafeAuto,
        ] {
            assert_eq!(
                action_execution(level, &BuddyAction::Dismiss),
                ActionExecution::Navigate
            );
        }
    }

    #[test]
    fn read_only_denies_mutating_actions() {
        let batch = BuddyAction::ApplyMemoryBatch {
            batch_key: "merge_exact_duplicate".to_string(),
            count_hint: 3,
        };
        assert_eq!(
            action_execution(AutonomyLevel::ReadOnly, &batch),
            ActionExecution::Deny
        );
        assert_eq!(
            action_execution(AutonomyLevel::Propose, &batch),
            ActionExecution::Execute
        );
    }
}
