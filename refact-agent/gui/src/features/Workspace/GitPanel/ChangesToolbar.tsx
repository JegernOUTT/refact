import * as ToolbarPrimitive from "@radix-ui/react-toolbar";
import {
  ChevronsDownUp,
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
  icon: LucideIcon;
  onClick: () => void;
  disabled?: boolean;
  loading?: boolean;
  pressed?: boolean;
};

function ToolbarAction({
  label,
  icon,
  onClick,
  disabled,
  loading,
  pressed,
}: ToolbarActionProps) {
  return (
    <Tooltip content={label}>
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
    </Tooltip>
  );
}

export type ChangesToolbarProps = {
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
    </ToolbarPrimitive.Root>
  );
}
