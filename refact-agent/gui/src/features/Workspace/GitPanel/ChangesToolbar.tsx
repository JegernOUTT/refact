import * as ToolbarPrimitive from "@radix-ui/react-toolbar";
import {
  ArrowDownToLine,
  ArrowUpFromLine,
  ChevronsDownUp,
  CloudDownload,
  ChevronsUpDown,
  FileDiff,
  FolderTree,
  RefreshCw,
  Undo2,
  type LucideIcon,
} from "lucide-react";

import { IconButton, Tooltip } from "../../../components/ui";
import styles from "./GitDock.module.css";

type ToolbarActionProps = {
  label: string;
  tooltip?: string;
  icon: LucideIcon;
  onClick: () => void;
  disabled?: boolean;
  loading?: boolean;
  pressed?: boolean;
};

function ToolbarAction({
  label,
  tooltip,
  icon,
  onClick,
  disabled,
  loading,
  pressed,
}: ToolbarActionProps) {
  const button = (
    <ToolbarPrimitive.Button asChild>
      <IconButton
        aria-label={label}
        aria-pressed={pressed}
        className={styles.toolbarButton}
        data-pressed={pressed ? "true" : undefined}
        disabled={disabled}
        icon={icon}
        loading={loading}
        onClick={onClick}
        size="sm"
        variant="ghost"
      />
    </ToolbarPrimitive.Button>
  );
  return (
    <Tooltip content={tooltip ?? label}>
      {disabled && tooltip ? (
        <span className={styles.disabledTooltipHost}>{button}</span>
      ) : (
        button
      )}
    </Tooltip>
  );
}

export type RemoteToolbarProps = {
  fetching: boolean;
  pulling: boolean;
  pushing: boolean;
  canPush: boolean;
  updateBlockedReason: string | null;
  onFetch: () => void;
  onPull: () => void;
  onPush: () => void;
};

export type ChangesToolbarProps = {
  remote: RemoteToolbarProps;
  refreshing: boolean;
  canRollback: boolean;
  canShowDiff: boolean;
  groupByDirectory: boolean;
  onRefresh: () => void;
  onRollback: () => void;
  onShowDiff: () => void;
  onToggleGroupByDirectory: () => void;
  onExpandAll: () => void;
  onCollapseAll: () => void;
};

export function ChangesToolbar({
  remote,
  refreshing,
  canRollback,
  canShowDiff,
  groupByDirectory,
  onRefresh,
  onRollback,
  onShowDiff,
  onToggleGroupByDirectory,
  onExpandAll,
  onCollapseAll,
}: ChangesToolbarProps) {
  return (
    <ToolbarPrimitive.Root
      aria-label="Commit tool window actions"
      className={styles.toolbar}
    >
      <ToolbarAction
        icon={RefreshCw}
        label="Refresh"
        loading={refreshing}
        onClick={onRefresh}
      />
      <ToolbarAction
        disabled={!canRollback}
        icon={Undo2}
        label="Rollback…"
        onClick={onRollback}
      />
      <ToolbarAction
        disabled={!canShowDiff}
        icon={FileDiff}
        label="Show Diff"
        onClick={onShowDiff}
      />
      <ToolbarPrimitive.Separator className={styles.toolbarSeparator} />
      <ToolbarAction
        icon={FolderTree}
        label="Group by Directory"
        onClick={onToggleGroupByDirectory}
        pressed={groupByDirectory}
      />
      <ToolbarAction
        icon={ChevronsUpDown}
        label="Expand All"
        onClick={onExpandAll}
      />
      <ToolbarAction
        icon={ChevronsDownUp}
        label="Collapse All"
        onClick={onCollapseAll}
      />
      <span className={styles.toolbarSpacer} />
      <ToolbarAction
        icon={CloudDownload}
        label="Fetch"
        loading={remote.fetching}
        onClick={remote.onFetch}
      />
      <ToolbarAction
        disabled={remote.updateBlockedReason !== null}
        icon={ArrowDownToLine}
        label="Update Project"
        loading={remote.pulling}
        onClick={remote.onPull}
        tooltip={
          remote.updateBlockedReason ??
          "Update Project (pull, fast-forward only)"
        }
      />
      <ToolbarAction
        disabled={!remote.canPush}
        icon={ArrowUpFromLine}
        label="Push…"
        loading={remote.pushing}
        onClick={remote.onPush}
      />
    </ToolbarPrimitive.Root>
  );
}
