import React from "react";
import { Text, Spinner } from "@radix-ui/themes";
import { useAppSelector } from "../../hooks";
import { selectPulse } from "./buddySlice";
import styles from "./BuddyPulseCard.module.css";

export const BuddyPulseCard: React.FC = () => {
  const pulse = useAppSelector(selectPulse);

  if (!pulse) {
    return (
      <div className={styles.card}>
        <Text size="1" weight="bold" color="gray" className={styles.label}>
          PULSE
        </Text>
        <div className={styles.loading}>
          <Spinner size="1" />
        </div>
      </div>
    );
  }

  return (
    <div className={styles.card} data-testid="buddy-pulse-card">
      {pulse.humor && (
        <div className={styles.humor}>
          <Text size="1">{pulse.humor}</Text>
        </div>
      )}
      <Text size="1" weight="bold" color="gray" className={styles.label}>
        PULSE
      </Text>
      <div className={styles.rows} role="list">
        <div className={styles.row} role="listitem">
          <Text size="1" color="gray" className={styles.rowLabel}>
            Tasks
          </Text>
          <Text size="1" className={styles.rowValue}>
            {pulse.tasks.total} open · {pulse.tasks.stuck} stuck ·{" "}
            {pulse.tasks.abandoned} abandoned
          </Text>
        </div>
        <div className={styles.row} role="listitem">
          <Text size="1" color="gray" className={styles.rowLabel}>
            Trajectories
          </Text>
          <Text size="1" className={styles.rowValue}>
            {pulse.trajectories.total} · {pulse.trajectories.untitled} untitled
            · oldest {pulse.trajectories.oldest_age_days}d
          </Text>
        </div>
        <div className={styles.row} role="listitem">
          <Text size="1" color="gray" className={styles.rowLabel}>
            Memory
          </Text>
          <Text size="1" className={styles.rowValue}>
            {pulse.memory.total} docs · {pulse.memory.orphan} orphan ·{" "}
            {pulse.memory.stale_conflicts} conflict
          </Text>
        </div>
        <div className={styles.row} role="listitem">
          <Text size="1" color="gray" className={styles.rowLabel}>
            Providers
          </Text>
          <Text size="1" className={styles.rowValue}>
            {pulse.providers.defaults_ok ? "✓" : "⚠"} defaults ·{" "}
            {pulse.providers.broken_refs} broken refs
          </Text>
        </div>
        <div className={styles.row} role="listitem">
          <Text size="1" color="gray" className={styles.rowLabel}>
            MCP
          </Text>
          <Text size="1" className={styles.rowValue}>
            {pulse.mcp.total} · {pulse.mcp.failing} failing ·{" "}
            {pulse.mcp.auth_expiring} expiring
          </Text>
        </div>
        <div className={styles.row} role="listitem">
          <Text size="1" color="gray" className={styles.rowLabel}>
            Customization
          </Text>
          <Text size="1" className={styles.rowValue}>
            {pulse.customization.modes}M · {pulse.customization.skills}S ·{" "}
            {pulse.customization.commands}C · {pulse.customization.subagents}A ·{" "}
            {pulse.customization.hooks}H
          </Text>
        </div>
        <div className={styles.row} role="listitem">
          <Text size="1" color="gray" className={styles.rowLabel}>
            Diagnostics
          </Text>
          <Text size="1" className={styles.rowValue}>
            {pulse.diagnostics.last_hour} in last hour
            {pulse.diagnostics.top_error_types.length > 0
              ? ` [${pulse.diagnostics.top_error_types.join(", ")}]`
              : ""}
          </Text>
        </div>
        <div className={styles.row} role="listitem">
          <Text size="1" color="gray" className={styles.rowLabel}>
            Git
          </Text>
          <Text size="1" className={styles.rowValue}>
            {pulse.git.uncommitted_files} files · {pulse.git.diff_lines_4h}{" "}
            lines / 4h
          </Text>
        </div>
        <div className={styles.row} role="listitem">
          <Text size="1" color="gray" className={styles.rowLabel}>
            Worktrees
          </Text>
          <Text size="1" className={styles.rowValue}>
            {pulse.worktrees.total} total · {pulse.worktrees.abandoned_clean}{" "}
            clean abandoned · {pulse.worktrees.dirty} dirty
          </Text>
        </div>
      </div>
    </div>
  );
};
