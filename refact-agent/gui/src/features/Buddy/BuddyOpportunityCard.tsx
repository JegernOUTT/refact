import React from "react";
import { Text, Button } from "@radix-ui/themes";
import classNames from "classnames";
import type { BuddyOpportunity, BuddyAction, BuddyPage } from "./types";
import { useExecuteBuddyAction } from "./hooks/useExecuteBuddyAction";
import styles from "./BuddyOpportunityCard.module.css";

function actionLabel(action: BuddyAction): string {
  switch (action.kind) {
    case "open_page":
      return "Open " + humanizePage(action.page);
    case "launch_investigation_chat":
      return "Investigate";
    case "draft_skill":
    case "draft_command":
    case "draft_subagent":
    case "draft_mode":
      return action.label;
    case "draft_agents_md_patch":
      return "Update AGENTS.md";
    case "draft_defaults_change":
      return "Adjust defaults";
    case "draft_customization_change":
      return "Edit";
    case "offer_marketplace_install":
      return "Browse marketplace";
    case "create_pulse_report":
      return "Create report";
    case "dismiss":
      return "Dismiss";
  }
}

function humanizePage(page: BuddyPage): string {
  switch (page.type) {
    case "buddy":
      return "Buddy";
    case "stats":
      return "Stats";
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
    case "mcp_marketplace":
      return "MCP Marketplace";
    case "skills_marketplace":
      return "Skills Marketplace";
    case "commands_marketplace":
      return "Commands Marketplace";
    case "subagents_marketplace":
      return "Subagents Marketplace";
    case "tasks_list":
      return "Tasks";
    case "task_workspace":
      return "Task Workspace";
    case "knowledge_graph":
      return "Knowledge Graph";
  }
}

interface Props {
  opportunity: BuddyOpportunity;
}

export const BuddyOpportunityCard: React.FC<Props> = ({ opportunity }) => {
  const executeAction = useExecuteBuddyAction();
  const isActive =
    opportunity.status === "new" || opportunity.status === "shown";

  const priorityClass = {
    critical: styles.priorityCritical,
    high: styles.priorityHigh,
    normal: styles.priorityNormal,
    low: styles.priorityLow,
  }[opportunity.priority];

  return (
    <div className={styles.card}>
      <div className={styles.header}>
        <span
          className={classNames(styles.priorityBadge, priorityClass)}
          aria-label={`Priority: ${opportunity.priority}`}
        >
          {opportunity.priority}
        </span>
        <Text size="2" className={styles.summary}>
          {opportunity.summary}
        </Text>
      </div>
      {opportunity.humor && (
        <Text size="1" className={styles.humor}>
          {opportunity.humor}
        </Text>
      )}
      {opportunity.proposed_actions.length > 0 && (
        <div className={styles.actions}>
          {opportunity.proposed_actions.map((action, idx) => (
            <Button
              key={idx}
              size="1"
              variant={action.kind === "dismiss" ? "ghost" : "soft"}
              disabled={!isActive}
              aria-label={actionLabel(action)}
              onClick={() => void executeAction(action, opportunity)}
            >
              {actionLabel(action)}
            </Button>
          ))}
        </div>
      )}
    </div>
  );
};
