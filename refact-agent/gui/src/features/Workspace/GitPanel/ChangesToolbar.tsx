import * as ToolbarPrimitive from "@radix-ui/react-toolbar";
import {
  ArrowDownToLine,
  ArrowUpFromLine,
  ChevronsDownUp,
  CloudDownload,
  ChevronsUpDown,
  FileDiff,
  FolderTree,
  History,
  MoreHorizontal,
  RefreshCw,
  Undo2,
  type LucideIcon,
} from "lucide-react";

import { Icon, IconButton, Menu, Tooltip } from "../../../components/ui";
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
  /** Why Rollback is disabled; shown as its tooltip. */
  rollbackTooltip?: string;
  canShowDiff: boolean;
  groupByDirectory: boolean;
  onRefresh: () => void;
  onRollback: () => void;
  onShowDiff: () => void;
  onToggleGroupByDirectory: () => void;
  onExpandAll: () => void;
  onCollapseAll: () => void;
  onShowLog: () => void;
};

/**
 * Overflow menu shown only on narrow docks (CSS container query on
 * `.toolbarHost`): view actions move here first, then Show Diff/Git Log,
 * so the toolbar always stays on one row with the remote actions visible.
 */
function ViewActionsOverflow({
  canShowDiff,
  groupByDirectory,
  onShowDiff,
  onShowLog,
  onToggleGroupByDirectory,
  onExpandAll,
  onCollapseAll,
}: Pick<
  ChangesToolbarProps,
  | "canShowDiff"
  | "groupByDirectory"
  | "onShowDiff"
  | "onShowLog"
  | "onToggleGroupByDirectory"
  | "onExpandAll"
  | "onCollapseAll"
>) {
  return (
    <span className={styles.toolbarOverflow}>
      <Menu modal={false}>
        <Tooltip content="More actions">
          <ToolbarPrimitive.Button asChild>
            <Menu.Trigger asChild>
              <IconButton
                aria-label="More actions"
                className={styles.toolbarButton}
                icon={MoreHorizontal}
                size="sm"
                variant="ghost"
              />
            </Menu.Trigger>
          </ToolbarPrimitive.Button>
        </Tooltip>
        <Menu.Content align="start">
          <Menu.Item disabled={!canShowDiff} onSelect={onShowDiff}>
            <Icon icon={FileDiff} size="sm" tone="muted" />
            Show Diff
          </Menu.Item>
          <Menu.Item onSelect={onShowLog}>
            <Icon icon={History} size="sm" tone="muted" />
            Git Log
          </Menu.Item>
          <Menu.Separator />
          <Menu.Item onSelect={onToggleGroupByDirectory}>
            <Icon icon={FolderTree} size="sm" tone="muted" />
            {groupByDirectory ? "Ungroup Directories" : "Group by Directory"}
          </Menu.Item>
          <Menu.Item onSelect={onExpandAll}>
            <Icon icon={ChevronsUpDown} size="sm" tone="muted" />
            Expand All
          </Menu.Item>
          <Menu.Item onSelect={onCollapseAll}>
            <Icon icon={ChevronsDownUp} size="sm" tone="muted" />
            Collapse All
          </Menu.Item>
        </Menu.Content>
      </Menu>
    </span>
  );
}

export function ChangesToolbar({
  remote,
  refreshing,
  canRollback,
  rollbackTooltip,
  canShowDiff,
  groupByDirectory,
  onRefresh,
  onRollback,
  onShowDiff,
  onToggleGroupByDirectory,
  onExpandAll,
  onCollapseAll,
  onShowLog,
}: ChangesToolbarProps) {
  return (
    <div className={styles.toolbarHost}>
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
          tooltip={canRollback ? undefined : rollbackTooltip}
        />
        <span className={styles.toolbarNavActions}>
          <ToolbarAction
            disabled={!canShowDiff}
            icon={FileDiff}
            label="Show Diff"
            onClick={onShowDiff}
          />
          <ToolbarPrimitive.Separator className={styles.toolbarSeparator} />
          <ToolbarAction icon={History} label="Git Log" onClick={onShowLog} />
        </span>
        <span className={styles.toolbarViewActions}>
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
        </span>
        <ViewActionsOverflow
          canShowDiff={canShowDiff}
          groupByDirectory={groupByDirectory}
          onShowDiff={onShowDiff}
          onShowLog={onShowLog}
          onCollapseAll={onCollapseAll}
          onExpandAll={onExpandAll}
          onToggleGroupByDirectory={onToggleGroupByDirectory}
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
    </div>
  );
}
