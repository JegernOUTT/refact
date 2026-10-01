use refact_buddy_core::operator_types::WorkItemSeverity;

use super::types::{BuddyOpportunityKind, BuddyPriority, DraftKind};

#[derive(Clone, Copy)]
pub(super) enum ImportTriage {
    Opportunity {
        priority: BuddyPriority,
        kind: BuddyOpportunityKind,
    },
    Draft(DraftKind),
}

pub(super) fn severity_for_import(triage: ImportTriage) -> Option<WorkItemSeverity> {
    match triage {
        ImportTriage::Opportunity { priority, .. } => Some(severity_for_priority(priority)),
        ImportTriage::Draft(_) => Some(WorkItemSeverity::Unknown),
    }
}

pub(super) fn scope_for_import(triage: ImportTriage) -> Option<&'static str> {
    match triage {
        ImportTriage::Opportunity { kind, .. } => Some(scope_for_kind(kind)),
        ImportTriage::Draft(kind) => scope_for_draft(kind),
    }
}

pub(super) fn severity_for_priority(priority: BuddyPriority) -> WorkItemSeverity {
    match priority {
        BuddyPriority::Low => WorkItemSeverity::Low,
        BuddyPriority::Normal => WorkItemSeverity::Normal,
        BuddyPriority::High => WorkItemSeverity::High,
        BuddyPriority::Critical => WorkItemSeverity::Critical,
    }
}

pub(super) fn scope_for_kind(kind: BuddyOpportunityKind) -> &'static str {
    match kind {
        BuddyOpportunityKind::TaskHealth => "Tasks",
        BuddyOpportunityKind::TrajectoryCleanup => "Trajectories",
        BuddyOpportunityKind::ChatRecap => "Chats",
        BuddyOpportunityKind::MemoryGarden | BuddyOpportunityKind::MemoryOpsBatch => {
            "Knowledge base"
        }
        BuddyOpportunityKind::ConfigDrift => "Configuration",
        BuddyOpportunityKind::AgentsMdGap => "AGENTS.md",
        BuddyOpportunityKind::ProviderTuning => "Providers",
        BuddyOpportunityKind::IntegrationFix => "Integrations",
        BuddyOpportunityKind::DiagnosticInvestigation => "Diagnostics",
        BuddyOpportunityKind::GitHygiene => "Git",
        BuddyOpportunityKind::WorktreeCleanup => "Worktrees",
        BuddyOpportunityKind::JobFinding => "Workflows",
        BuddyOpportunityKind::Quest => "Quests",
    }
}

pub(super) fn scope_for_draft(kind: DraftKind) -> Option<&'static str> {
    match kind {
        DraftKind::Skill => Some("Skills"),
        DraftKind::Command => Some("Commands"),
        DraftKind::Delegate => Some("Subagents"),
        DraftKind::Mode => Some("Modes"),
        DraftKind::AgentsMd => Some("AGENTS.md"),
        DraftKind::DefaultsModel => Some("Providers"),
        DraftKind::Hook => Some("Hooks"),
        DraftKind::PulseReport => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_priority_maps_to_a_work_item_severity() {
        let cases = [
            (BuddyPriority::Low, WorkItemSeverity::Low),
            (BuddyPriority::Normal, WorkItemSeverity::Normal),
            (BuddyPriority::High, WorkItemSeverity::High),
            (BuddyPriority::Critical, WorkItemSeverity::Critical),
        ];

        for (priority, expected) in cases {
            assert_eq!(severity_for_priority(priority), expected);
        }
    }

    #[test]
    fn every_opportunity_kind_maps_to_a_human_scope() {
        let cases = [
            (BuddyOpportunityKind::TaskHealth, "Tasks"),
            (BuddyOpportunityKind::TrajectoryCleanup, "Trajectories"),
            (BuddyOpportunityKind::ChatRecap, "Chats"),
            (BuddyOpportunityKind::MemoryGarden, "Knowledge base"),
            (BuddyOpportunityKind::ConfigDrift, "Configuration"),
            (BuddyOpportunityKind::AgentsMdGap, "AGENTS.md"),
            (BuddyOpportunityKind::ProviderTuning, "Providers"),
            (BuddyOpportunityKind::IntegrationFix, "Integrations"),
            (BuddyOpportunityKind::DiagnosticInvestigation, "Diagnostics"),
            (BuddyOpportunityKind::GitHygiene, "Git"),
            (BuddyOpportunityKind::WorktreeCleanup, "Worktrees"),
            (BuddyOpportunityKind::MemoryOpsBatch, "Knowledge base"),
            (BuddyOpportunityKind::JobFinding, "Workflows"),
            (BuddyOpportunityKind::Quest, "Quests"),
        ];

        for (kind, expected) in cases {
            assert_eq!(scope_for_kind(kind), expected);
            assert!(!scope_for_kind(kind).contains('_'));
        }
    }

    #[test]
    fn draft_scope_is_present_only_for_a_clear_target() {
        let cases = [
            (DraftKind::Skill, Some("Skills")),
            (DraftKind::Command, Some("Commands")),
            (DraftKind::Delegate, Some("Subagents")),
            (DraftKind::Mode, Some("Modes")),
            (DraftKind::AgentsMd, Some("AGENTS.md")),
            (DraftKind::DefaultsModel, Some("Providers")),
            (DraftKind::Hook, Some("Hooks")),
            (DraftKind::PulseReport, None),
        ];

        for (kind, expected) in cases {
            assert_eq!(scope_for_draft(kind), expected);
        }
    }
}
