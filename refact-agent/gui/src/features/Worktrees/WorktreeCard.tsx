import {
  Copy,
  ExternalLink,
  FileDiff,
  GitBranch,
  GitMerge,
  Trash2,
} from "lucide-react";
import { Icon, IconButton } from "../../components/ui";
import { KitTooltip } from "../../components/ComposerKit";
import type {
  WorktreeInspection,
  WorktreeMeta,
  WorktreeRecordView,
} from "../../services/refact/worktrees";
import { WorktreeStatusBadge } from "./WorktreeStatusBadge";
import styles from "./WorktreeCard.module.css";

import { worktreeSize } from "./WorktreeCard.helpers";

type Props = {
  worktree?: WorktreeMeta;
  record?: WorktreeRecordView | null;
  inspection?: WorktreeInspection;
  selected: boolean;
  mainPath?: string;
  mainBranch?: string | null;
  unavailable?: boolean;
  selectionDisabled?: boolean;
  managementDisabled?: boolean;
  onSelect?: () => void;
  onDiff?: () => void;
  onMerge?: () => void;
  onDelete?: () => void;
  onCopy?: () => void;
  onOpen?: () => void;
  openCopiesPath?: boolean;
};

export function WorktreeCard({
  worktree,
  record,
  inspection,
  selected,
  mainPath,
  mainBranch,
  unavailable,
  selectionDisabled,
  managementDisabled,
  onSelect,
  onDiff,
  onMerge,
  onDelete,
  onCopy,
  onOpen,
  openCopiesPath,
}: Props) {
  const label = worktree?.branch?.trim()
    ? worktree.branch
    : worktree?.root ?? "Main workspace";
  const path = worktree?.root ?? mainPath;
  const managed = worktree?.kind === "task_agent";
  const suffix = selected ? "" : ` ${label}`;
  const mergeLabel =
    inspection?.branch_merged === true
      ? "Merged into base"
      : inspection?.branch_merged === false
        ? "Not in base"
        : "Merge status unknown";
  const actions = [
    { label: `View worktree diff${suffix}`, icon: FileDiff, onClick: onDiff },
    {
      label: `Merge worktree${suffix}`,
      icon: GitMerge,
      onClick: onMerge,
      disabled: managed || managementDisabled,
    },
    {
      label: selected ? "Copy workspace path" : `Copy workspace path ${label}`,
      icon: Copy,
      onClick: onCopy,
    },
    {
      label:
        openCopiesPath === true
          ? `Copy folder path${suffix}`
          : selected
            ? "Open worktree in new window"
            : `Open worktree in new window ${label}`,
      icon: ExternalLink,
      onClick: onOpen,
    },
    {
      label: `Delete or discard worktree${suffix}`,
      icon: Trash2,
      onClick: onDelete,
      disabled: managed || managementDisabled,
      danger: true,
    },
  ];
  return (
    <article
      className={styles.card}
      data-selected={selected || undefined}
      aria-label={label}
    >
      <button
        type="button"
        className={styles.identity}
        data-worktree-selected={selected || undefined}
        aria-current={selected ? "true" : undefined}
        aria-label={
          worktree
            ? `Select worktree ${label}`
            : "Detach worktree and use main workspace"
        }
        disabled={selectionDisabled === true || managed || !onSelect}
        onClick={onSelect}
        title={managed ? "Managed by task lifecycle" : path}
      >
        <span className={styles.heading}>
          <Icon icon={GitBranch} size="sm" tone="muted" />
          <span className={styles.name}>{label}</span>
          {selected && <span className={styles.selected}>Selected</span>}
        </span>
        {!worktree && mainBranch && (
          <span className={styles.path}>{mainBranch}</span>
        )}
        {path && <span className={styles.path}>{path}</span>}
      </button>
      <div className={styles.metadata}>
        {worktree ? (
          <>
            <WorktreeStatusBadge worktree={worktree} record={record} />
            <span>{worktreeSize(inspection?.disk_usage_bytes)}</span>
            <KitTooltip label="Git ancestry against the base branch, not durable merge history. Squash merges may still appear as Not in base.">
              <span tabIndex={0}>{mergeLabel}</span>
            </KitTooltip>
            <span>
              {record?.reference_count ?? worktree.reference_count ?? 0}{" "}
              {(record?.reference_count ?? worktree.reference_count ?? 0) === 1
                ? "ref"
                : "refs"}
            </span>
            {managed && <span>Task-managed</span>}
            {unavailable && <span>Attached · not in this list</span>}
          </>
        ) : (
          <span>Main workspace</span>
        )}
      </div>
      <div
        className={styles.actions}
        role="group"
        aria-label={`Manage ${label}`}
      >
        {actions
          .filter((action) => action.onClick)
          .map((action) => (
            <KitTooltip
              key={action.label}
              label={
                action.disabled && managed
                  ? "Managed by task lifecycle"
                  : action.label
              }
            >
              <IconButton
                className={styles.action}
                type="button"
                icon={action.icon}
                size="sm"
                variant={action.danger ? "danger" : "ghost"}
                aria-label={action.label}
                disabled={action.disabled}
                onClick={action.onClick}
              />
            </KitTooltip>
          ))}
      </div>
    </article>
  );
}
