import React, { useCallback, useState } from "react";
import {
  Check,
  ChevronDown,
  ChevronRight,
  Circle,
  GitBranch,
  Link,
  Pencil,
  Repeat,
  User,
  Settings,
  X,
  Zap,
} from "lucide-react";
import {
  DashboardBadge as Badge,
  DashboardFlex as Flex,
  DashboardHoverCard as HoverCard,
  DashboardText as Text,
  DashboardTextField as TextField,
  DashboardTooltip as Tooltip,
  dashboardToneFromMode,
} from "../DashboardPrimitives";
import { Icon, StatusDot } from "../../../../components/ui";
import { getStatusFromSessionState } from "../../../../utils/sessionStatus";
import { DotTrail } from "../DotTrail/DotTrail";
import type { HistoryTreeNode } from "../../../History/historySlice";
import type { DashboardBreakpoint } from "../../types";
import styles from "./RecentItem.module.css";

type RecentItemProps = {
  node: HistoryTreeNode;
  breakpoint: DashboardBreakpoint;
  depth: number;
  isExpanded: boolean;
  onToggleExpand: (id: string) => void;
  onClick: () => void;
  onDotClick?: (chatId: string) => void;
  onDelete?: (id: string) => void;
  onRename?: (id: string, newTitle: string) => void;
};

function formatRelativeTime(dateStr: string): string {
  const date = new Date(dateStr);
  const now = new Date();
  const diffMs = now.getTime() - date.getTime();
  const diffMin = Math.floor(diffMs / 60_000);
  const diffHr = Math.floor(diffMs / 3_600_000);
  const diffDay = Math.floor(diffMs / 86_400_000);

  if (diffMin < 1) return "just now";
  if (diffMin < 60) return `${diffMin}m ago`;
  if (diffHr < 24) return `${diffHr}h ago`;
  if (diffDay < 7) return `${diffDay}d ago`;
  return date.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

type RelationInfo = {
  icon: React.ReactNode;
  label: string;
  tone: "muted" | "accent" | "success" | "warning";
};

function compactWorktreeLabel(label: string): string {
  const normalized = label.replace(/[\\/]+$/, "");
  const parts = normalized.split(/[\\/]/).filter(Boolean);
  if (parts.length <= 3) return normalized || label;
  return parts.slice(-3).join("/");
}

function worktreeLabel(node: HistoryTreeNode): string | null {
  const branch = node.worktree?.branch?.trim();
  const label =
    branch !== undefined && branch.length > 0 ? branch : node.worktree?.root;
  return label ? compactWorktreeLabel(label) : null;
}

function hasDiffStats(node: HistoryTreeNode): boolean {
  return (
    (node.total_lines_added ?? 0) > 0 || (node.total_lines_removed ?? 0) > 0
  );
}

function WorktreeBadge({ node }: { node: HistoryTreeNode }) {
  const label = worktreeLabel(node);
  if (!label) return null;

  const added = node.total_lines_added ?? 0;
  const removed = node.total_lines_removed ?? 0;
  const branch = node.worktree?.branch?.trim();
  const fullLabel =
    branch !== undefined && branch.length > 0
      ? branch
      : node.worktree?.root !== undefined && node.worktree.root.length > 0
        ? node.worktree.root
        : label;

  return (
    <Tooltip content={`Worktree: ${fullLabel}`}>
      <Badge
        tone="success"
        className={styles.worktreeBadge}
      >
        <span className={styles.worktreeName}>{label}</span>
        {hasDiffStats(node) && (
          <span className={styles.diffStatsBadge}>
            <span>(</span>
            <span className={styles.diffStatsAdd}>+{added}</span>
            <span className={styles.diffStatsRemove}>-{removed}</span>
            <span>)</span>
          </span>
        )}
      </Badge>
    </Tooltip>
  );
}

function getRelationInfo(
  node: HistoryTreeNode,
  depth: number,
): RelationInfo | null {
  if (node.task_id) {
    return node.task_role === "planner"
      ? {
          icon: <Icon icon={Settings} size="sm" tone="accent" />,
          label: "Planner",
          tone: "accent",
        }
      : {
          icon: <Icon icon={User} size="sm" tone="accent" />,
          label: "Agent",
          tone: "accent",
        };
  }
  switch (node.link_type) {
    case "subagent":
      return {
        icon: <Icon icon={Link} size="sm" tone="success" />,
        label: "Subagent",
        tone: "success",
      };
    case "handoff":
      return {
        icon: <Icon icon={Repeat} size="sm" tone="success" />,
        label: "Handoff",
        tone: "success",
      };
    case "branch":
      return {
        icon: <Icon icon={GitBranch} size="sm" tone="warning" />,
        label: "Branched",
        tone: "warning",
      };
    case "mode_transition":
      return {
        icon: <Icon icon={Zap} size="sm" tone="warning" />,
        label: "Mode Switch",
        tone: "warning",
      };
    default:
      if (depth > 0 && !node.link_type && !node.task_id) {
        return {
          icon: <Icon icon={Circle} size="sm" tone="muted" />,
          label: "Original",
          tone: "muted",
        };
      }
      return null;
  }
}

function ItemHoverContent({
  node,
  relation,
}: {
  node: HistoryTreeNode;
  relation: RelationInfo | null;
}) {
  const messageCount = node.message_count ?? 0;
  const label = worktreeLabel(node);
  return (
    <Flex direction="column" gap="2">
      <Text size="2" weight="bold" truncate>
        {node.title || "New Chat"}
      </Text>

      {node.model && (
        <Flex gap="1" align="center">
          <Text size="1" tone="muted">
            Model:
          </Text>
          <Text size="1">{node.model}</Text>
        </Flex>
      )}

      {node.mode && (
        <Flex gap="1" align="center">
          <Text size="1" tone="muted">
            Mode:
          </Text>
          <Badge size="1" tone={dashboardToneFromMode(node.mode)}>
            {node.mode}
          </Badge>
        </Flex>
      )}

      {relation && (
        <Flex gap="1" align="center">
          <Text size="1" tone="muted">
            Type:
          </Text>
          <Flex align="center" gap="1">
            {relation.icon}
            <Text size="1">{relation.label}</Text>
          </Flex>
        </Flex>
      )}

      {label && (
        <Flex gap="1" align="center">
          <Text size="1" tone="muted">
            Worktree:
          </Text>
          <Text size="1">{label}</Text>
        </Flex>
      )}

      {messageCount > 0 && (
        <Flex gap="1" align="center">
          <Text size="1" tone="muted">
            Messages:
          </Text>
          <Text size="1">{messageCount}</Text>
        </Flex>
      )}

      {((node.total_lines_added ?? 0) > 0 ||
        (node.total_lines_removed ?? 0) > 0) && (
        <Flex gap="1" align="center">
          <Text size="1" tone="muted">
            Changes:
          </Text>
          {(node.total_lines_added ?? 0) > 0 && (
            <Text size="1" tone="success">
              +{node.total_lines_added}
            </Text>
          )}
          {(node.total_lines_removed ?? 0) > 0 && (
            <Text size="1" tone="danger">
              −{node.total_lines_removed}
            </Text>
          )}
        </Flex>
      )}

      {(node.tasks_total ?? 0) > 0 && (
        <Flex gap="1" align="center">
          <Text size="1" tone="muted">
            Tasks:
          </Text>
          <Text size="1">
            {node.tasks_done ?? 0}/{node.tasks_total}
            {(node.tasks_failed ?? 0) > 0 && (
              <Text size="1" tone="danger">
                {" "}
                ({node.tasks_failed} failed)
              </Text>
            )}
          </Text>
        </Flex>
      )}

      {node.session_state && node.session_state !== "idle" && (
        <Flex gap="1" align="center">
          <Text size="1" tone="muted">
            Status:
          </Text>
          <Text size="1">{node.session_state}</Text>
        </Flex>
      )}

      <Text size="1" tone="muted">
        {new Date(node.createdAt).toLocaleString()}
      </Text>
    </Flex>
  );
}

export const RecentItem: React.FC<RecentItemProps> = ({
  node,
  breakpoint,
  depth,
  isExpanded,
  onToggleExpand,
  onClick,
  onDotClick,
  onDelete,
  onRename,
}) => {
  const [isEditing, setIsEditing] = useState(false);
  const [editValue, setEditValue] = useState("");

  const statusState = getStatusFromSessionState(node.session_state);
  const hasChildren = node.children.length > 0;
  const hasTrail = hasChildren || node.bubbleChildren.length > 0;
  const messageCount = node.message_count ?? 0;
  const relation = getRelationInfo(node, depth);

  const hasStats =
    messageCount > 0 ||
    (node.total_lines_added ?? 0) > 0 ||
    (node.total_lines_removed ?? 0) > 0;
  const showHover = hasStats || !!relation || !!node.worktree;

  const handleStartEdit = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation();
      setEditValue(node.title || "");
      setIsEditing(true);
    },
    [node.title],
  );

  const handleConfirmEdit = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation();
      const trimmed = editValue.trim();
      if (trimmed && trimmed !== node.title) {
        onRename?.(node.id, trimmed);
      }
      setIsEditing(false);
    },
    [editValue, node.id, node.title, onRename],
  );

  const handleCancelEdit = useCallback((e: React.MouseEvent) => {
    e.stopPropagation();
    setIsEditing(false);
  }, []);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter") {
        const trimmed = editValue.trim();
        if (trimmed && trimmed !== node.title) {
          onRename?.(node.id, trimmed);
        }
        setIsEditing(false);
      } else if (e.key === "Escape") {
        setIsEditing(false);
      }
    },
    [editValue, node.id, node.title, onRename],
  );

  const handleDelete = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation();
      onDelete?.(node.id);
    },
    [node.id, onDelete],
  );

  const handleRowKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (isEditing) return;
      if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        onClick();
      }
    },
    [isEditing, onClick],
  );

  const handleToggle = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation();
      onToggleExpand(node.id);
    },
    [node.id, onToggleExpand],
  );

  const indent = depth * 16;

  const titleElement = isEditing ? (
    <TextField.Root
      size="1"
      value={editValue}
      onChange={(e) => setEditValue(e.target.value)}
      onKeyDown={handleKeyDown}
      onClick={(e) => e.stopPropagation()}
      autoFocus
      className={styles.editInput}
    />
  ) : showHover ? (
    <HoverCard.Root>
      <HoverCard.Trigger>
        <span className={styles.titleTrigger}>
          <Text size="2" truncate className={styles.title}>
            {node.title || "New Chat"}
          </Text>
        </span>
      </HoverCard.Trigger>
      <HoverCard.Content
        className={styles.hoverCard}
      >
        <ItemHoverContent node={node} relation={relation} />
      </HoverCard.Content>
    </HoverCard.Root>
  ) : (
    <Text size="2" truncate className={styles.title}>
      {node.title || "New Chat"}
    </Text>
  );

  return (
    <div
      role="button"
      tabIndex={0}
      className={`${styles.item} rf-enter-rise rf-pressable`}
      style={
        indent > 0
          ? { paddingLeft: `calc(var(--rf-space-2) + ${indent}px)` }
          : undefined
      }
      onClick={isEditing ? undefined : onClick}
      onKeyDown={handleRowKeyDown}
    >
      <div className={styles.left}>
        {hasChildren ? (
          <button
            type="button"
            className={styles.expandButton}
            onClick={handleToggle}
            aria-label={isExpanded ? "Collapse" : "Expand"}
            aria-expanded={isExpanded}
          >
            {isExpanded ? (
              <Icon icon={ChevronDown} size="sm" tone="muted" />
            ) : (
              <Icon icon={ChevronRight} size="sm" tone="muted" />
            )}
          </button>
        ) : (
          <span className={styles.treeIndent} />
        )}
        <StatusDot
          status={statusState}
          size="small"
          pulse={statusState === "in_progress"}
        />
        {relation && (
          <Tooltip content={relation.label}>
            <span className={styles.relationIcon} data-tone={relation.tone}>
              {relation.icon}
            </span>
          </Tooltip>
        )}
        {titleElement}
      </div>
      <div className={styles.right}>
        {hasTrail && (
          <DotTrail
            node={node}
            breakpoint={breakpoint}
            onDotClick={onDotClick}
          />
        )}
        {breakpoint !== "narrow" && node.mode && (
          <Badge size="1" tone={dashboardToneFromMode(node.mode)}>
            {node.mode}
          </Badge>
        )}
        <WorktreeBadge node={node} />
        <Text size="1" tone="muted" className={styles.time}>
          {formatRelativeTime(node.updatedAt)}
        </Text>
        <div className={styles.actions}>
          {isEditing ? (
            <>
              <Tooltip content="Save">
                <button
                  type="button"
                  className={styles.actionButton}
                  onClick={handleConfirmEdit}
                  aria-label="Save rename"
                >
                  <Icon icon={Check} size="sm" tone="success" />
                </button>
              </Tooltip>
              <Tooltip content="Cancel">
                <button
                  type="button"
                  className={styles.actionButton}
                  onClick={handleCancelEdit}
                  aria-label="Cancel rename"
                >
                  <Icon icon={X} size="sm" tone="danger" />
                </button>
              </Tooltip>
            </>
          ) : (
            <>
              {onRename && (
                <Tooltip content="Rename">
                  <button
                    type="button"
                    className={styles.actionButton}
                    onClick={handleStartEdit}
                    aria-label="Rename chat"
                  >
                    <Icon icon={Pencil} size="sm" tone="muted" />
                  </button>
                </Tooltip>
              )}
              {onDelete && (
                <Tooltip content="Delete">
                  <button
                    type="button"
                    className={styles.actionButton}
                    onClick={handleDelete}
                    aria-label="Delete chat"
                  >
                    <Icon icon={X} size="sm" tone="danger" />
                  </button>
                </Tooltip>
              )}
            </>
          )}
        </div>
      </div>
    </div>
  );
};
