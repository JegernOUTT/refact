import React, { useRef, useState } from "react";
import { Text } from "@radix-ui/themes";
import classNames from "classnames";
import type {
  BuddyAction,
  BuddyOpportunity,
  CreateConductorGoalRequest,
  GoalAutonomy,
  GoalBudget,
} from "./types";
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
  doneWhenSummary: string;
  doneWhenChecklist: string;
  autonomy: GoalAutonomy;
}

const EMPTY_GOAL_BUDGET_INPUTS: GoalBudgetInputs = {
  wallClockHours: "",
  noProgressWakes: "",
  totalTokens: "",
  usd: "",
  doneWhenSummary: "",
  doneWhenChecklist: "",
  autonomy: "governed",
};

function isGoalProposalAction(
  action: BuddyAction,
): action is Extract<BuddyAction, { kind: "start_conductor_goal" }> {
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

function buildDoneWhen(inputs: GoalBudgetInputs) {
  const summary = inputs.doneWhenSummary.trim();
  const checklist = inputs.doneWhenChecklist
    .split("\n")
    .map((item) => item.trim())
    .filter(Boolean);
  if (!summary && checklist.length === 0) return null;
  return { summary, checklist };
}

function buildConductorGoal(
  action: Extract<BuddyAction, { kind: "start_conductor_goal" }>,
  inputs: GoalBudgetInputs,
): CreateConductorGoalRequest | null {
  const budget = buildGoalBudget(inputs);
  const doneWhen = buildDoneWhen(inputs);
  if (!budget || !doneWhen) return null;
  const contextLine = action.source_task_id
    ? `\n\nSource task: ${action.source_task_id}`
    : "";
  return {
    title: action.title,
    plan_doc_slug: action.plan_doc_slug ?? null,
    plan_markdown: `# ${action.title}\n\nCreated from a Buddy opportunity after the user provided guardrails.${contextLine}`,
    done_when: doneWhen,
    autonomy: inputs.autonomy,
    budget,
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
      const conductorGoal = isGoalProposalAction(action)
        ? buildConductorGoal(action, goalBudgetInputs)
        : undefined;
      await executeAction(action, opportunity, idx, conductorGoal ?? undefined);
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
            Guardrails before Buddy opens the conductor goal
          </Text>
          <label className={styles.goalBudgetField}>
            <span>Done when summary or checklist required</span>
            <textarea
              value={goalBudgetInputs.doneWhenSummary}
              onChange={(event) =>
                handleBudgetChange("doneWhenSummary", event.target.value)
              }
            />
          </label>
          <label className={styles.goalBudgetField}>
            <span>Done when checklist optional</span>
            <textarea
              value={goalBudgetInputs.doneWhenChecklist}
              onChange={(event) =>
                handleBudgetChange("doneWhenChecklist", event.target.value)
              }
            />
          </label>
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
            <label className={styles.goalBudgetField}>
              <span>Autonomy required</span>
              <select
                value={goalBudgetInputs.autonomy}
                onChange={(event) =>
                  handleBudgetChange(
                    "autonomy",
                    event.target.value as GoalAutonomy,
                  )
                }
              >
                <option value="read_only">Read only</option>
                <option value="governed">Governed</option>
                <option value="full_auto">Full auto</option>
              </select>
            </label>
          </div>
        </div>
      )}
      {opportunity.proposed_actions.length > 0 && (
        <div className={styles.actions}>
          {opportunity.proposed_actions.map((action, idx) => {
            const goalBudgetMissing =
              isGoalProposalAction(action) &&
              buildConductorGoal(action, goalBudgetInputs) == null;
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
