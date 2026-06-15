import React from "react";
import { Badge, StatusDot, type StatusDotStatus } from "../../components/ui";
import type { WorktreeMeta, WorktreeRecordView } from "../../services/refact";
import styles from "./Worktrees.module.css";

type WorktreeStatusBadgeProps = {
  worktree?: WorktreeMeta | null;
  record?: WorktreeRecordView | null;
  additions?: number | null;
  deletions?: number | null;
};

function hasDiffStats(
  additions?: number | null,
  deletions?: number | null,
): boolean {
  return (additions ?? 0) > 0 || (deletions ?? 0) > 0;
}

function DiffStats({
  additions,
  deletions,
}: {
  additions?: number | null;
  deletions?: number | null;
}) {
  if (!hasDiffStats(additions, deletions)) return null;
  const added = additions ?? 0;
  const removed = deletions ?? 0;
  return (
    <span className={styles.diffStatsBadge}>
      <span>(</span>
      <span className={styles.diffStatsAdd}>+{added}</span>
      <span className={styles.diffStatsRemove}>-{removed}</span>
      <span>)</span>
    </span>
  );
}

function statusBadge(
  label: string,
  tone: React.ComponentProps<typeof Badge>["tone"],
  status: StatusDotStatus,
  additions?: number | null,
  deletions?: number | null,
) {
  return (
    <Badge tone={tone} className={styles.statusBadge}>
      <StatusDot status={status} size="small" />
      {label} <DiffStats additions={additions} deletions={deletions} />
    </Badge>
  );
}

export const WorktreeStatusBadge: React.FC<WorktreeStatusBadgeProps> = ({
  worktree,
  record,
  additions,
  deletions,
}) => {
  const status = record?.status ?? worktree?.status ?? null;
  const lifecycle = record?.meta.lifecycle_state ?? worktree?.lifecycle_state;

  if (
    lifecycle === "deleted" ||
    worktree?.deleted === true ||
    status?.deleted === true
  ) {
    return statusBadge("deleted", "danger", "error");
  }

  if (lifecycle === "missing" || status?.path_exists === false) {
    return statusBadge("missing", "danger", "error");
  }

  if (lifecycle === "conflicted" || status?.conflicted === true) {
    return statusBadge("conflicted", "warning", "warning");
  }

  if (
    lifecycle === "stale" ||
    worktree?.stale === true ||
    status?.stale === true
  ) {
    return statusBadge("stale", "warning", "warning");
  }

  if (status?.dirty === true) {
    return statusBadge("dirty", "warning", "warning", additions, deletions);
  }

  return statusBadge("worktree", "success", "success", additions, deletions);
};

WorktreeStatusBadge.displayName = "WorktreeStatusBadge";
