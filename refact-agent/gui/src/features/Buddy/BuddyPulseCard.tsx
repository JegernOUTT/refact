import React from "react";
import { Activity } from "lucide-react";
import { LoadingState, StatusDot, Surface, Text } from "../../components/ui";
import { useAppSelector } from "../../hooks";
import { selectPulse } from "./buddySlice";
import { BuddySectionHeader } from "./BuddySectionHeader";
import styles from "./BuddyPulseCard.module.css";

const PulseRow: React.FC<{ label: string; children: React.ReactNode }> = ({
  label,
  children,
}) => (
  <div className={styles.row} role="listitem">
    <Text size="1" color="gray" className={styles.rowLabel}>
      {label}
    </Text>
    <Text size="1" className={styles.rowValue}>
      {children}
    </Text>
  </div>
);

export const BuddyPulseCard: React.FC = () => {
  const pulse = useAppSelector(selectPulse);

  if (!pulse) {
    return (
      <Surface
        animated="rise"
        className={styles.card}
        radius="card"
        variant="glass"
      >
        <BuddySectionHeader icon={Activity} label="Pulse" />
        <LoadingState label="Loading pulse" variant="compact" />
      </Surface>
    );
  }

  return (
    <Surface
      className={styles.card}
      data-testid="buddy-pulse-card"
      animated="rise"
      radius="card"
      variant="glass"
    >
      <BuddySectionHeader icon={Activity} label="Pulse" />
      {pulse.humor && (
        <Text size="1" className={styles.humor}>
          {pulse.humor}
        </Text>
      )}
      <div className={styles.rows} role="list">
        <PulseRow label="Tasks">
          {pulse.tasks.total} open · {pulse.tasks.stuck} stuck ·{" "}
          {pulse.tasks.abandoned} abandoned
        </PulseRow>
        <PulseRow label="Trajectories">
          {pulse.trajectories.total} · {pulse.trajectories.untitled} untitled ·
          oldest {pulse.trajectories.oldest_age_days}d
        </PulseRow>
        <PulseRow label="Memory">
          {pulse.memory.total} docs · {pulse.memory.orphan} orphan ·{" "}
          {pulse.memory.stale_conflicts} conflict
        </PulseRow>
        <PulseRow label="Providers">
          <StatusDot
            className={styles.rowDot}
            status={pulse.providers.defaults_ok ? "success" : "warning"}
          />{" "}
          defaults · {pulse.providers.broken_refs} broken refs
        </PulseRow>
        <PulseRow label="MCP">
          {pulse.mcp.total} · {pulse.mcp.failing} failing ·{" "}
          {pulse.mcp.auth_expiring} expiring
        </PulseRow>
        <PulseRow label="Customization">
          {pulse.customization.modes}M · {pulse.customization.skills}S ·{" "}
          {pulse.customization.commands}C · {pulse.customization.subagents}A ·{" "}
          {pulse.customization.hooks}H
        </PulseRow>
        <PulseRow label="Diagnostics">
          {pulse.diagnostics.last_hour} in last hour
          {pulse.diagnostics.top_error_types.length > 0
            ? ` [${pulse.diagnostics.top_error_types.join(", ")}]`
            : ""}
        </PulseRow>
        <PulseRow label="Git">
          {pulse.git.uncommitted_files} files · {pulse.git.diff_lines_4h} lines
          / 4h
        </PulseRow>
        <PulseRow label="Worktrees">
          {pulse.worktrees.total} total · {pulse.worktrees.abandoned_clean}{" "}
          clean abandoned · {pulse.worktrees.dirty} dirty
        </PulseRow>
      </div>
    </Surface>
  );
};
