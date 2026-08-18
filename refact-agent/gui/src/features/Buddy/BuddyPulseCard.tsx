import React from "react";
import { Activity } from "lucide-react";
import { LoadingState, StatusDot, Surface, Text } from "../../components/ui";
import { useAppSelector } from "../../hooks";
import { selectPulse } from "./buddySlice";
import { BuddySectionHeader } from "./BuddySectionHeader";
import styles from "./BuddyPulseCard.module.css";

const PulseRow: React.FC<{
  label: string;
  children: React.ReactNode;
  title?: string;
}> = ({ label, children, title }) => (
  <div className={styles.row} role="listitem">
    <Text size="1" color="gray" className={styles.rowLabel}>
      {label}
    </Text>
    <Text size="1" className={styles.rowValue} title={title}>
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
          {pulse.tasks.total ?? 0} open · {pulse.tasks.stuck ?? 0} stuck ·{" "}
          {pulse.tasks.abandoned ?? 0} abandoned
        </PulseRow>
        <PulseRow label="Trajectories">
          {pulse.trajectories.total ?? 0} · {pulse.trajectories.untitled ?? 0}{" "}
          untitled · oldest {pulse.trajectories.oldest_age_days ?? 0}d
        </PulseRow>
        <PulseRow label="Memory">
          {pulse.memory.total ?? 0} docs · {pulse.memory.orphan ?? 0} orphan ·{" "}
          {pulse.memory.stale_conflicts ?? 0} conflict
          {memoryDetails.length > 0 ? ` · ${memoryDetails.join(" · ")}` : ""}
        </PulseRow>
        <PulseRow label="Providers">
          <span
            role="img"
            aria-label={
              pulse.providers.defaults_ok ? "defaults ok" : "defaults broken"
            }
          >
            <StatusDot
              className={styles.rowDot}
              status={pulse.providers.defaults_ok ? "success" : "warning"}
            />
          </span>{" "}
          defaults · {pulse.providers.broken_refs ?? 0} broken refs
        </PulseRow>
        <PulseRow label="MCP">
          {pulse.mcp.total ?? 0} · {pulse.mcp.failing ?? 0} failing ·{" "}
          {pulse.mcp.auth_expiring ?? 0} expiring
        </PulseRow>
        <PulseRow label="Customization">
          {pulse.customization.modes ?? 0}M · {pulse.customization.skills ?? 0}S
          · {pulse.customization.commands ?? 0}C ·{" "}
          {pulse.customization.subagents ?? 0}A ·{" "}
          {pulse.customization.hooks ?? 0}H
        </PulseRow>
        <PulseRow
          label="Diagnostics"
          title={`${pulse.diagnostics.last_hour ?? 0} in last hour${
            pulse.diagnostics.top_error_types.length > 0
              ? ` [${pulse.diagnostics.top_error_types.join(", ")}]`
              : ""
          }`}
        >
          {pulse.diagnostics.last_hour ?? 0} in last hour
          {pulse.diagnostics.top_error_types.length > 0
            ? ` [${pulse.diagnostics.top_error_types.join(", ")}]`
            : ""}
        </PulseRow>
        <PulseRow label="Git">
          {pulse.git.uncommitted_files ?? 0} files ·{" "}
          {pulse.git.diff_lines_4h ?? 0} lines / 4h
        </PulseRow>
        <PulseRow label="Worktrees">
          {pulse.worktrees.total ?? 0} total ·{" "}
          {pulse.worktrees.abandoned_clean ?? 0} clean abandoned ·{" "}
          {pulse.worktrees.dirty ?? 0} dirty
        </PulseRow>
      </div>
    </Surface>
  );
};
