import React, { useState } from "react";
import { Badge, Button } from "../../components/ui";
import type { BuddyOpportunity } from "./types";
import { buddyActionResultMessage } from "./hooks/useExecuteBuddyAction";
import { useBuddyControlAction } from "./hooks/useBuddyControlAction";
import { actionLabel } from "./buddyOpportunityActions";
import { BuddyRow, BuddyStatusBadge, buddyStatus, toBuddySeverity } from "./ui";
import styles from "./BuddyOpportunityCard.module.css";

interface Props {
  opportunity: BuddyOpportunity;
}

export const BuddyOpportunityCard: React.FC<Props> = ({ opportunity }) => {
  const { opportunity: runOpportunity, pendingActionIds } =
    useBuddyControlAction();
  const [pendingActionIndex, setPendingActionIndex] = useState<number | null>(
    null,
  );
  const [actionError, setActionError] = useState<string | null>(null);
  const isActive =
    opportunity.status === "new" || opportunity.status === "shown";

  const handleActionClick = async (idx: number, never?: boolean) => {
    if (!isActive) return;
    if (idx < 0 || idx >= opportunity.proposed_actions.length) return;
    const action = opportunity.proposed_actions[idx];
    setPendingActionIndex(idx);
    setActionError(null);
    const result = await runOpportunity(action, opportunity, idx, { never });
    if (result.status === "ignored") return;
    setPendingActionIndex(null);
    setActionError(buddyActionResultMessage(result));
  };

  const dismissIndex = opportunity.proposed_actions.findIndex(
    (action) => action.kind === "dismiss",
  );
  const isQuest = opportunity.kind === "quest";
  const busy = pendingActionIds.size > 0;
  const primaryIndex = opportunity.proposed_actions.findIndex(
    (action) => action.kind !== "dismiss",
  );

  const severity = toBuddySeverity(opportunity.priority);
  const actions =
    opportunity.proposed_actions.length > 0 ? (
      <div className={styles.actions}>
        {opportunity.proposed_actions.map((action, idx) => (
          <Button
            key={`${action.kind}-${JSON.stringify(action)}`}
            size="sm"
            type="button"
            variant={
              action.kind !== "dismiss" && idx === primaryIndex
                ? "primary"
                : "ghost"
            }
            disabled={!isActive || busy}
            aria-label={actionLabel(action)}
            aria-busy={pendingActionIndex === idx}
            loading={pendingActionIndex === idx}
            onClick={() => void handleActionClick(idx)}
          >
            {pendingActionIndex === idx ? "Working…" : actionLabel(action)}
          </Button>
        ))}
        {dismissIndex >= 0 && (
          <Button
            size="sm"
            type="button"
            variant="ghost"
            disabled={!isActive || busy}
            aria-label="Never show this again"
            onClick={() => void handleActionClick(dismissIndex, true)}
          >
            Never
          </Button>
        )}
      </div>
    ) : null;

  return (
    <div className={styles.card}>
      <BuddyRow
        title={opportunity.summary}
        titleLines={2}
        description={opportunity.humor ?? undefined}
        leading={{
          tick: severity ? buddyStatus("severity", severity).tone : "default",
        }}
        status={
          <>
            {severity && (
              <span aria-label={`Priority: ${opportunity.priority}`}>
                <BuddyStatusBadge dimension="severity" value={severity} />
              </span>
            )}
            {isQuest && (
              <Badge tone="default" size="xs" aria-label="Quest">
                Quest
              </Badge>
            )}
          </>
        }
      />
      {actions}
      {actionError && (
        <p className={styles.actionError} role="alert">
          {actionError}
        </p>
      )}
    </div>
  );
};
