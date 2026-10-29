import React from "react";
import { Activity } from "lucide-react";
import { StatusDot, Text } from "../../components/ui";
import { useAppSelector } from "../../hooks";
import { selectPulse } from "./buddySlice";
import { BuddyLoading, BuddySection } from "./ui";
import styles from "./BuddyPulseCard.module.css";

function count(value: number | null | undefined): number {
  return value ?? 0;
}

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
      <BuddySection title="Pulse" icon={Activity}>
        <BuddyLoading variant="rows" count={3} compact label="Loading pulse" />
      </BuddySection>
    );
  }

  const memoryOps = [
    { label: "pending", value: count(pulse.memory.pending_ops) },
    { label: "applied", value: count(pulse.memory.applied_ops) },
    { label: "failed", value: count(pulse.memory.failed_ops) },
  ].filter((item) => item.value > 0);
  const memoryCandidateTotal =
    count(pulse.memory.merge_candidates) +
    count(pulse.memory.archive_candidates) +
    count(pulse.memory.review_candidates) +
    count(pulse.memory.conflict_candidates);
  const memoryDetails = [
    ...memoryOps.map((item) => `${item.value} ${item.label}`),
    ...(memoryCandidateTotal > 0 ? [`${memoryCandidateTotal} candidates`] : []),
  ];

  return (
    <BuddySection title="Pulse" icon={Activity} testId="buddy-pulse-card">
      {pulse.humor && (
        <Text size="1" className={styles.humor}>
          {pulse.humor}
        </Text>
      )}
      <div className={styles.rows} role="list">
        <PulseRow label="Tasks">
          {count(pulse.tasks.total)} open · {count(pulse.tasks.stuck)} stuck ·{" "}
          {count(pulse.tasks.abandoned)} abandoned
        </PulseRow>
        <PulseRow label="Trajectories">
          {count(pulse.trajectories.total)} ·{" "}
          {count(pulse.trajectories.untitled)} untitled · oldest{" "}
          {count(pulse.trajectories.oldest_age_days)}d
        </PulseRow>
        <PulseRow label="Memory">
          {count(pulse.memory.total)} docs · {count(pulse.memory.orphan)} orphan
          · {count(pulse.memory.stale_conflicts)} conflict
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
          defaults · {count(pulse.providers.broken_refs)} broken refs
        </PulseRow>
        <PulseRow label="MCP">
          {count(pulse.mcp.total)} · {count(pulse.mcp.failing)} failing ·{" "}
          {count(pulse.mcp.auth_expiring)} expiring
        </PulseRow>
        <PulseRow label="Customization">
          {count(pulse.customization.modes)}M ·{" "}
          {count(pulse.customization.skills)}S ·{" "}
          {count(pulse.customization.commands)}C ·{" "}
          {count(pulse.customization.subagents)}A ·{" "}
          {count(pulse.customization.hooks)}H
        </PulseRow>
        <PulseRow
          label="Diagnostics"
          title={`${count(pulse.diagnostics.last_hour)} in last hour${
            pulse.diagnostics.top_error_types.length > 0
              ? ` [${pulse.diagnostics.top_error_types.join(", ")}]`
              : ""
          }`}
        >
          {count(pulse.diagnostics.last_hour)} in last hour
          {pulse.diagnostics.top_error_types.length > 0
            ? ` [${pulse.diagnostics.top_error_types.join(", ")}]`
            : ""}
        </PulseRow>
        <PulseRow label="Git">
          {count(pulse.git.uncommitted_files)} files ·{" "}
          {count(pulse.git.diff_lines_4h)} lines / 4h
        </PulseRow>
        <PulseRow label="Worktrees">
          {count(pulse.worktrees.total)} total ·{" "}
          {count(pulse.worktrees.abandoned_clean)} clean abandoned ·{" "}
          {count(pulse.worktrees.dirty)} dirty
        </PulseRow>
      </div>
    </BuddySection>
  );
};
