import React from "react";
import { Text } from "@radix-ui/themes";
import classNames from "classnames";
import type { BuddyOpportunity } from "./types";
import { useExecuteBuddyAction } from "./hooks/useExecuteBuddyAction";
import { actionLabel } from "./buddyOpportunityActions";
import styles from "./BuddyOpportunityCard.module.css";

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
            <button
              key={idx}
              type="button"
              className={classNames(
                styles.actionButton,
                action.kind === "dismiss"
                  ? styles.actionButtonGhost
                  : styles.actionButtonPrimary,
              )}
              disabled={!isActive}
              aria-label={actionLabel(action)}
              onClick={() => void executeAction(action, opportunity, idx)}
            >
              {actionLabel(action)}
            </button>
          ))}
        </div>
      )}
    </div>
  );
};
