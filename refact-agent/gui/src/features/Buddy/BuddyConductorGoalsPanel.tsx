import React, { useMemo, useState } from "react";
import { Text } from "@radix-ui/themes";
import classNames from "classnames";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { push, openScheduler } from "../Pages/pagesSlice";
import { selectConductorGoals } from "./buddySlice";
import { conductorStateView, goalToConductorState } from "./conductorMood";
import type { ConductorGoal, GoalStatus } from "./types";
import styles from "./BuddyConductorGoalsPanel.module.css";

type GoalFilter = "all" | "active" | GoalStatus;

type Props = {
  compact?: boolean;
};

const ACTIVE_STATUSES = new Set<GoalStatus>([
  "proposed",
  "active",
  "planned",
  "running",
  "waiting_for_human",
  "escalated",
  "paused",
]);

const FILTERS: { id: GoalFilter; label: string }[] = [
  { id: "all", label: "All" },
  { id: "active", label: "Active" },
  { id: "waiting_for_human", label: "Needs human" },
  { id: "escalated", label: "Escalated" },
  { id: "abandoned", label: "Abandoned" },
  { id: "done", label: "Done" },
  { id: "failed", label: "Failed" },
];

function formatStatus(status: GoalStatus): string {
  return status.replace(/_/g, " ");
}

function formatTokens(value: number): string {
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}K`;
  return String(value);
}

function formatUsd(value: number | null | undefined): string {
  if (value == null) return "—";
  return `$${value.toFixed(2)}`;
}

function goalMatchesFilter(goal: ConductorGoal, filter: GoalFilter): boolean {
  if (filter === "all") return true;
  if (filter === "active") return ACTIVE_STATUSES.has(goal.status);
  return goal.status === filter;
}

function formatBudgetPercent(spent: number, budget: number | null | undefined) {
  if (!budget || budget <= 0) return "—";
  return `${Math.min(999, Math.round((spent / budget) * 100))}%`;
}

function GoalCard({ goal }: { goal: ConductorGoal }) {
  const stateView = conductorStateView(goalToConductorState(goal));
  const tokenBudgetPercent = formatBudgetPercent(
    goal.spent.total_tokens,
    goal.budget.total_tokens,
  );

  return (
    <article
      className={classNames(styles.goalCard, styles[`tone_${stateView.tone}`])}
      data-testid="conductor-goal-card"
      data-conductor-state={stateView.state}
    >
      <div className={styles.goalHeader}>
        <div className={styles.goalTitleGroup}>
          <Text size="2" weight="bold" className={styles.goalTitle}>
            {goal.title}
          </Text>
          <span className={styles.stateLine}>
            {stateView.emoji} {stateView.label}
          </span>
        </div>
        <span className={classNames(styles.statusBadge, styles.statusAccent)}>
          {formatStatus(goal.status)}
        </span>
      </div>

      <div className={styles.metaGrid}>
        <div className={styles.metaItem}>
          <span className={styles.metaLabel}>Planner</span>
          <span className={styles.metaValue}>
            {goal.summary.has_planner_task ? "linked" : "—"}
          </span>
        </div>
        <div className={styles.metaItem}>
          <span className={styles.metaLabel}>Planners / Agents</span>
          <span className={styles.metaValue}>
            {goal.summary.has_planner_task ? 1 : 0} / {goal.summary.task_count}
          </span>
        </div>
        <div className={styles.metaItem}>
          <span className={styles.metaLabel}>Tokens</span>
          <span className={styles.metaValue}>
            {formatTokens(goal.spent.total_tokens)} /{" "}
            {goal.budget.total_tokens
              ? formatTokens(goal.budget.total_tokens)
              : "—"}
          </span>
        </div>
        <div className={styles.metaItem}>
          <span className={styles.metaLabel}>Cache</span>
          <span className={styles.metaValue}>
            {formatTokens(goal.spent.cache_read_tokens)} read
          </span>
        </div>
        <div className={styles.metaItem}>
          <span className={styles.metaLabel}>USD</span>
          <span className={styles.metaValue}>
            {formatUsd(goal.spent.usd)} / {formatUsd(goal.budget.usd)}
          </span>
        </div>
        <div className={styles.metaItem}>
          <span className={styles.metaLabel}>Budget</span>
          <span className={styles.metaValue}>{tokenBudgetPercent}</span>
        </div>
        <div className={styles.metaItem}>
          <span className={styles.metaLabel}>Questions</span>
          <span className={styles.metaValue}>
            {goal.summary.open_question_count} open
          </span>
        </div>
        <div className={styles.metaItem}>
          <span className={styles.metaLabel}>Model stats</span>
          <span className={styles.metaValue}>
            P {formatTokens(goal.spent.prompt_tokens)} · C{" "}
            {formatTokens(goal.spent.completion_tokens)}
          </span>
        </div>
      </div>

      {goal.summary.memo_count > 0 || goal.summary.ghost_message_count > 0 ? (
        <div className={styles.recentRail}>
          <div className={styles.recentItem}>
            <span className={styles.metaLabel}>Activity summary</span>
            <span className={styles.recentText}>
              {goal.summary.memo_count} memos · {goal.summary.ghost_message_count} ghost messages · {goal.summary.learning_record_count} lessons
            </span>
          </div>
        </div>
      ) : null}

      <div className={styles.controlsArea}>
        <Text size="1" color="gray" weight="bold">
          Goal controls
        </Text>
        <Text size="1" color="gray">
          Pause, resume, cancel, and recurring wake controls will activate when
          the conductor control backend is available.
        </Text>
      </div>
    </article>
  );
}

export const BuddyConductorGoalsPanel: React.FC<Props> = ({ compact }) => {
  const dispatch = useAppDispatch();
  const goals = useAppSelector(selectConductorGoals);
  const [filter, setFilter] = useState<GoalFilter>(compact ? "active" : "all");

  const filteredGoals = useMemo(
    () => goals.filter((goal) => goalMatchesFilter(goal, filter)),
    [filter, goals],
  );

  const activeCount = goals.filter((goal) =>
    ACTIVE_STATUSES.has(goal.status),
  ).length;

  return (
    <section
      className={classNames(styles.panel, { [styles.compact]: compact })}
      data-testid={
        compact ? "buddy-home-conductor-goals" : "buddy-conductor-page"
      }
    >
      <div className={styles.header}>
        <div className={styles.titleGroup}>
          <Text size="1" className={styles.sectionLabel}>
            Conductor goals
          </Text>
          <Text size="3" weight="bold">
            {activeCount} active · {goals.length} total
          </Text>
        </div>
        <div className={styles.actions}>
          {compact && (
            <button
              type="button"
              className={classNames(styles.chip, styles.chipPrimary)}
              onClick={() => dispatch(push({ name: "conductor" }))}
            >
              Open Conductor
            </button>
          )}
          <button
            type="button"
            className={styles.chip}
            onClick={() => dispatch(openScheduler(undefined))}
          >
            Recurring controls
          </button>
        </div>
      </div>

      <div className={styles.filters} aria-label="Conductor goal filters">
        {FILTERS.map((item) => (
          <button
            key={item.id}
            type="button"
            className={classNames(styles.chip, {
              [styles.filterActive]: filter === item.id,
            })}
            onClick={() => setFilter(item.id)}
          >
            {item.label}
          </button>
        ))}
      </div>

      {filteredGoals.length === 0 ? (
        <Text className={styles.emptyState} data-testid="conductor-empty-state">
          No conductor goals yet. Tiny chaos engine is idling politely.
        </Text>
      ) : (
        <div className={styles.goalList}>
          {filteredGoals.map((goal) => (
            <GoalCard key={goal.id} goal={goal} />
          ))}
        </div>
      )}
    </section>
  );
};
