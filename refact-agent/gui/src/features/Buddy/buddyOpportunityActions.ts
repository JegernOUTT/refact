import type {
  BuddyAction,
  BuddyControl,
  BuddyOpportunity,
  BuddyPage,
  CustomizationKind,
  MarketKind,
  PulseScope,
} from "./types";

const OPPORTUNITY_ACTION_PREFIX = "opportunity_action:";

export function actionLabel(action: BuddyAction): string {
  switch (action.kind) {
    case "open_page":
      return "Open " + humanizePage(action.page);
    case "launch_investigation_chat":
      return "Investigate";
    case "draft_skill":
    case "draft_command":
    case "draft_delegate":
    case "draft_mode":
      return action.label;
    case "draft_agents_md_patch":
      return "Update AGENTS.md";
    case "draft_defaults_change":
      return "Adjust defaults";
    case "draft_customization_change":
      return `Edit ${humanizeCustomizationKind(action.customization_kind)}`;
    case "offer_marketplace_install":
      return `Install ${humanizeMarketKind(action.market_kind)}`;
    case "create_pulse_report":
      return `Create ${humanizePulseScope(action.scope)} report`;
    case "dismiss":
      return "Dismiss";
  }
}

export function opportunitySpeechText(opportunity: BuddyOpportunity): string {
  const priority = opportunity.priority.toUpperCase();
  return opportunity.humor
    ? `${priority} ${opportunity.summary} ${opportunity.humor}`
    : `${priority} ${opportunity.summary}`;
}

export function opportunityActionControls(
  opportunity: BuddyOpportunity,
): BuddyControl[] {
  return opportunity.proposed_actions.map((action, index) => ({
    id: `${opportunity.id}-${index}`,
    label: actionLabel(action),
    action: `${OPPORTUNITY_ACTION_PREFIX}${index}`,
    style: action.kind === "dismiss" ? "ghost" : "primary",
  }));
}

export function getOpportunityActionIndexFromControl(
  control: BuddyControl,
): number | null {
  if (!control.action.startsWith(OPPORTUNITY_ACTION_PREFIX)) return null;

  const index = Number(control.action.slice(OPPORTUNITY_ACTION_PREFIX.length));
  if (!Number.isInteger(index)) return null;

  return index;
}

export function getOpportunityActionFromControl(
  control: BuddyControl,
  opportunity: BuddyOpportunity,
): BuddyAction | null {
  const index = getOpportunityActionIndexFromControl(control);
  if (index == null) return null;

  return opportunity.proposed_actions[index] ?? null;
}

export function getOpportunityDismissAction(opportunity: BuddyOpportunity): {
  action: BuddyAction;
  actionIndex: number;
} {
  const actionIndex = opportunity.proposed_actions.findIndex(
    (action) => action.kind === "dismiss",
  );
  if (actionIndex >= 0) {
    const action = opportunity.proposed_actions[actionIndex];
    return { action, actionIndex };
  }
  return { action: { kind: "dismiss" }, actionIndex: 0 };
}

export function humanizeCustomizationKind(kind: CustomizationKind): string {
  switch (kind) {
    case "mode":
      return "mode";
    case "skill":
      return "skill";
    case "command":
      return "command";
    case "delegate":
      return "delegate";
    case "hook":
      return "hook";
  }
}

export function humanizePulseScope(scope: PulseScope): string {
  switch (scope) {
    case "all":
      return "system";
    case "tasks":
      return "tasks";
    case "trajectories":
      return "trajectory";
    case "memory":
      return "memory";
    case "providers":
      return "provider";
    case "mcp":
      return "MCP";
    case "customization":
      return "customization";
    case "diagnostics":
      return "diagnostic";
    case "git":
      return "git";
    case "worktrees":
      return "worktrees";
  }
}

export function humanizeMarketKind(kind: MarketKind): string {
  switch (kind) {
    case "mcp":
      return "MCP";
    case "skill":
      return "skill";
    case "command":
      return "command";
    case "delegate":
      return "delegate";
  }
}

function humanizePage(page: BuddyPage): string {
  switch (page.type) {
    case "buddy":
      return "Companion";
    case "stats":
      return "Stats";
    case "settings":
      return "Settings";
    case "customization":
      return "Customization";
    case "providers":
      return "Providers";
    case "default_models":
      return "Default Models";
    case "integrations":
      return "Integrations";
    case "extensions":
      return "Extensions";
    case "marketplace_hub":
      return "Marketplace";
    case "marketplace":
      return "MCP Marketplace";
    case "skills_marketplace":
      return "Skills Marketplace";
    case "commands_marketplace":
      return "Commands Marketplace";
    case "delegates_marketplace":
      return "Subagents Marketplace";
    case "tasks_list":
      return "Tasks";
    case "task_workspace":
      return "Task Workspace";
    case "knowledge_graph":
      return "Knowledge Graph";
    case "worktrees":
      return "Worktrees";
    case "setup_mode":
      return "Setup";
  }
}
