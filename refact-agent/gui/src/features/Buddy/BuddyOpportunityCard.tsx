import React, { useRef, useState } from "react";
import { Text } from "@radix-ui/themes";
import classNames from "classnames";
import type { BuddyAction, BuddyOpportunity, GoalBudget } from "./types";
import {
  formatOpportunityActionError,
  useExecuteBuddyAction,
} from "./hooks/useExecuteBuddyAction";
import { actionLabel } from "./buddyOpportunityActions";
import styles from "./BuddyOpportunityCard.module.css";

interface GoalBudgetInputs {
  wallClockHours: string;
  noProgressWakes: string;
  totalTokens: string;
  usd: string;
}

const EMPTY_GOAL_BUDGET_INPUTS: GoalBudgetInputs = {
  wallClockHours: "",
  noProgressWakes: "",
  totalTokens: "",
  usd: "",
};

function isGoalProposalAction(action: BuddyAction): boolean {
  return action.kind === "start_conductor_goal";
}

function parsePositiveNumber(value: string): number | null {
  const trimmed = value.trim();
  if (!trimmed) return null;
  const parsed = Number(trimmed);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : null;
}

function buildGoalBudget(inputs: GoalBudgetInputs): GoalBudget | null {
  const wallClockHours = parsePositiveNumber(inputs.wallClockHours);
  const noProgressWakes = parsePositiveNumber(inputs.noProgressWakes);
  if (wallClockHours == null || noProgressWakes == null) return null;

  const totalTokens = parsePositiveNumber(inputs.totalTokens);
  const usd = parsePositiveNumber(inputs.usd);
  return {
    wall_clock_secs: Math.round(wallClockHours * 60 * 60),
    no_progress_wakes: Math.round(noProgressWakes),
    ...(totalTokens == null ? {} : { total_tokens: Math.round(totalTokens) }),
    ...(usd == null ? {} : { usd }),
  };
}

interface Props {
  opportunity: BuddyOpportunity;
}

export const BuddyOpportunityCard: React.FC<Props> = ({ opportunity }) => {
  const executeAction = useExecuteBuddyAction();
  const [pendingActionIndex, setPendingActionIndex] = useState<number | null>(
    null,
  );
  const [actionError, setActionError] = useState<string | null>(null);
  const [goalBudgetInputs, setGoalBudgetInputs] = useState<GoalBudgetInputs>(
    EMPTY_GOAL_BUDGET_INPUTS,
  );
  const pendingRef = useRef(false);
  const isActive =
    opportunity.status === "new" || opportunity.status === "shown";

  const priorityClass = {
    critical: styles.priorityCritical,
    high: styles.priorityHigh,
    normal: styles.priorityNormal,
    low: styles.priorityLow,
  }[opportunity.priority];

  const handleBudgetChange = (field: keyof GoalBudgetInputs, value: string) => {
    setGoalBudgetInputs((current) => ({ ...current, [field]: value }));
  };

  const handleActionClick = async (idx: number) => {
    if (pendingRef.current || !isActive) return;
    pendingRef.current = true;
    setPendingActionIndex(idx);
    setActionError(null);
    try {
      if (idx < 0 || idx >= opportunity.proposed_actions.length) return;
      const action = opportunity.proposed_actions[idx];
      const budget = isGoalProposalAction(action)
        ? buildGoalBudget(goalBudgetInputs)
        : undefined;
      await executeAction(action, opportunity, idx, budget ?? undefined);
    } catch (error) {
      setActionError(formatOpportunityActionError(error));
    } finally {
      pendingRef.current = false;
      setPendingActionIndex(null);
    }
  };

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
      {opportunity.proposed_actions.some(isGoalProposalAction) && (
        <div className={styles.goalBudget}>
          <Text size="1" weight="bold" className={styles.goalBudgetTitle}>
            Guardrails before Buddy starts the conductor goal
          </Text>
          <div className={styles.goalBudgetGrid}>
            <label className={styles.goalBudgetField}>
              <span>Wall-clock hours required</span>
              <input
                type="number"
                min="0.1"
                step="0.1"
                value={goalBudgetInputs.wallClockHours}
                onChange={(event) =>
                  handleBudgetChange("wallClockHours", event.target.value)
                }
              />
            </label>
            <label className={styles.goalBudgetField}>
              <span>No-progress wakes required</span>
              <input
                type="number"
                min="1"
                step="1"
                value={goalBudgetInputs.noProgressWakes}
                onChange={(event) =>
                  handleBudgetChange("noProgressWakes", event.target.value)
                }
              />
            </label>
            <label className={styles.goalBudgetField}>
              <span>Token ceiling optional</span>
              <input
                type="number"
                min="1"
                step="1"
                value={goalBudgetInputs.totalTokens}
                onChange={(event) =>
                  handleBudgetChange("totalTokens", event.target.value)
                }
              />
            </label>
            <label className={styles.goalBudgetField}>
              <span>USD ceiling optional</span>
              <input
                type="number"
                min="0.01"
                step="0.01"
                value={goalBudgetInputs.usd}
                onChange={(event) =>
                  handleBudgetChange("usd", event.target.value)
                }
              />
            </label>
          </div>
        </div>
      )}
      {opportunity.proposed_actions.length > 0 && (
        <div className={styles.actions}>
          {opportunity.proposed_actions.map((action, idx) => {
            const goalBudgetMissing =
              isGoalProposalAction(action) &&
              buildGoalBudget(goalBudgetInputs) == null;
            return (
              <button
                key={idx}
                type="button"
                className={classNames(
                  styles.actionButton,
                  action.kind === "dismiss"
                    ? styles.actionButtonGhost
                    : styles.actionButtonPrimary,
                )}
                disabled={
                  !isActive || pendingActionIndex !== null || goalBudgetMissing
                }
                aria-label={actionLabel(action)}
                aria-busy={pendingActionIndex === idx}
                onClick={() => void handleActionClick(idx)}
              >
                {pendingActionIndex === idx ? "Working…" : actionLabel(action)}
              </button>
            );
          })}
        </div>
      )}
      {actionError && (
        <Text size="1" color="red" className={styles.actionError} role="alert">
          {actionError}
        </Text>
      )}
    </div>
  );
};
