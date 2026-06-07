import React, { useRef, useState } from "react";
import { Text } from "../../components/ui";
import { Badge, Button, Surface } from "../../components/ui";
import classNames from "classnames";
import type { BuddyOpportunity } from "./types";
import {
  formatOpportunityActionError,
  useExecuteBuddyAction,
} from "./hooks/useExecuteBuddyAction";
import { actionLabel } from "./buddyOpportunityActions";
import styles from "./BuddyOpportunityCard.module.css";

interface Props {
  opportunity: BuddyOpportunity;
}

export const BuddyOpportunityCard: React.FC<Props> = ({ opportunity }) => {
  const executeAction = useExecuteBuddyAction();
  const [pendingActionIndex, setPendingActionIndex] = useState<number | null>(
    null,
  );
  const [actionError, setActionError] = useState<string | null>(null);
  const pendingRef = useRef(false);
  const isActive =
    opportunity.status === "new" || opportunity.status === "shown";

  const priorityClass = {
    critical: styles.priorityCritical,
    high: styles.priorityHigh,
    normal: styles.priorityNormal,
    low: styles.priorityLow,
  }[opportunity.priority];

  const handleActionClick = async (idx: number) => {
    if (pendingRef.current || !isActive) return;
    pendingRef.current = true;
    setPendingActionIndex(idx);
    setActionError(null);
    try {
      if (idx < 0 || idx >= opportunity.proposed_actions.length) return;
      const action = opportunity.proposed_actions[idx];
      await executeAction(action, opportunity, idx);
    } catch (error) {
      setActionError(formatOpportunityActionError(error));
    } finally {
      pendingRef.current = false;
      setPendingActionIndex(null);
    }
  };

  return (
    <Surface className={styles.card} radius="control" variant="plain">
      <div className={styles.header}>
        <Badge
          className={classNames(styles.priorityBadge, priorityClass)}
          aria-label={`Priority: ${opportunity.priority}`}
          tone="muted"
        >
          {opportunity.priority}
        </Badge>
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
              size="sm"
              type="button"
              variant={action.kind === "dismiss" ? "ghost" : "primary"}
              className={styles.actionButton}
              disabled={!isActive || pendingActionIndex !== null}
              aria-label={actionLabel(action)}
              aria-busy={pendingActionIndex === idx}
              loading={pendingActionIndex === idx}
              onClick={() => void handleActionClick(idx)}
            >
              {pendingActionIndex === idx ? "Working…" : actionLabel(action)}
            </Button>
          ))}
        </div>
      )}
      {actionError && (
        <Text size="1" color="red" className={styles.actionError} role="alert">
          {actionError}
        </Text>
      )}
    </Surface>
  );
};
