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

  const memoryOps = [
    { label: "pending", value: pulse.memory.pending_ops ?? 0 },
    { label: "applied", value: pulse.memory.applied_ops ?? 0 },
    { label: "failed", value: pulse.memory.failed_ops ?? 0 },
  ].filter((item) => item.value > 0);
  const memoryCandidateTotal =
    (pulse.memory.merge_candidates ?? 0) +
    (pulse.memory.archive_candidates ?? 0) +
    (pulse.memory.review_candidates ?? 0) +
    (pulse.memory.conflict_candidates ?? 0);
  const memoryDetails = [
    ...memoryOps.map((item) => `${item.value} ${item.label}`),
    ...(memoryCandidateTotal > 0 ? [`${memoryCandidateTotal} candidates`] : []),
  ];

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
          {memoryDetails.length > 0 ? ` · ${memoryDetails.join(" · ")}` : ""}
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
