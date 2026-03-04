import React, { useCallback, useState } from "react";
import { Badge, Flex, HoverCard, Text, TextField, Tooltip } from "@radix-ui/themes";
import {
  Pencil1Icon,
  Cross1Icon,
  CheckIcon,
  Link2Icon,
  LoopIcon,
  BorderSplitIcon,
  LightningBoltIcon,
  DotFilledIcon,
  PersonIcon,
  GearIcon,
  ChevronRightIcon,
  ChevronDownIcon,
} from "@radix-ui/react-icons";
import { StatusDot } from "../../../../components/StatusDot";
import { getStatusFromSessionState } from "../../../../utils/sessionStatus";
import { getModeColor } from "../../../../utils/modeColors";
import { formatTokenCount } from "../../../StatsDashboard/utils/formatters";
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

export function getDateGroup(dateStr: string): string {
  const date = new Date(dateStr);
  const now = new Date();
  const todayStart = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const dateStart = new Date(date.getFullYear(), date.getMonth(), date.getDate());
  const diffDay = Math.floor((todayStart.getTime() - dateStart.getTime()) / 86_400_000);

  if (diffDay === 0) return "Today";
  if (diffDay === 1) return "Yesterday";
  if (diffDay < 7) return "Last 7 days";
  return "Older";
}

type RelationInfo = {
  icon: React.ReactNode;
  label: string;
  color: string;
};

const ICON_SIZE = 10;

function getRelationInfo(node: HistoryTreeNode, depth: number): RelationInfo | null {
  if (node.task_id) {
    return node.task_role === "planner"
      ? { icon: <GearIcon width={ICON_SIZE} height={ICON_SIZE} />, label: "Planner", color: "var(--purple-9)" }
      : { icon: <PersonIcon width={ICON_SIZE} height={ICON_SIZE} />, label: "Agent", color: "var(--blue-9)" };
  }
  switch (node.link_type) {
    case "subagent":
      return { icon: <Link2Icon width={ICON_SIZE} height={ICON_SIZE} />, label: "Subagent", color: "var(--green-9)" };
    case "handoff":
      return { icon: <LoopIcon width={ICON_SIZE} height={ICON_SIZE} />, label: "Handoff", color: "var(--green-9)" };
    case "branch":
      return { icon: <BorderSplitIcon width={ICON_SIZE} height={ICON_SIZE} />, label: "Branched", color: "var(--amber-9)" };
    case "mode_transition":
      return { icon: <LightningBoltIcon width={ICON_SIZE} height={ICON_SIZE} />, label: "Mode Switch", color: "var(--amber-9)" };
    default:
      if (depth > 0 && !node.link_type && !node.task_id) {
        return { icon: <DotFilledIcon width={ICON_SIZE} height={ICON_SIZE} />, label: "Original", color: "var(--gray-9)" };
      }
      return null;
  }
}

function ItemHoverContent({ node, relation }: { node: HistoryTreeNode; relation: RelationInfo | null }) {
  const messageCount = node.message_count ?? 0;
  return (
    <Flex direction="column" gap="2">
      <Text size="2" weight="bold" truncate>
        {node.title || "New Chat"}
      </Text>

      {node.model && (
        <Flex gap="1" align="center">
          <Text size="1" color="gray">Model:</Text>
          <Text size="1">{node.model}</Text>
        </Flex>
      )}

      {node.mode && (
        <Flex gap="1" align="center">
          <Text size="1" color="gray">Mode:</Text>
          <Badge size="1" color={getModeColor(node.mode)} variant="soft">{node.mode}</Badge>
        </Flex>
      )}

      {relation && (
        <Flex gap="1" align="center">
          <Text size="1" color="gray">Type:</Text>
          <Flex align="center" gap="1" style={{ color: relation.color }}>{relation.icon}<Text size="1">{relation.label}</Text></Flex>
        </Flex>
      )}

      {messageCount > 0 && (
        <Flex gap="1" align="center">
          <Text size="1" color="gray">Messages:</Text>
          <Text size="1">{messageCount}</Text>
        </Flex>
      )}

      {(node.total_coins ?? 0) > 0 && (
        <Flex gap="1" align="center">
          <Text size="1" color="gray">Tokens:</Text>
          <Text size="1">{formatTokenCount(node.total_coins ?? 0)}</Text>
        </Flex>
      )}

      {((node.total_lines_added ?? 0) > 0 || (node.total_lines_removed ?? 0) > 0) && (
        <Flex gap="1" align="center">
          <Text size="1" color="gray">Changes:</Text>
          {(node.total_lines_added ?? 0) > 0 && (
            <Text size="1" style={{ color: "var(--green-9)" }}>+{node.total_lines_added}</Text>
          )}
          {(node.total_lines_removed ?? 0) > 0 && (
            <Text size="1" style={{ color: "var(--red-9)" }}>−{node.total_lines_removed}</Text>
          )}
        </Flex>
      )}

      {(node.tasks_total ?? 0) > 0 && (
        <Flex gap="1" align="center">
          <Text size="1" color="gray">Tasks:</Text>
          <Text size="1">
            {node.tasks_done ?? 0}/{node.tasks_total}
            {(node.tasks_failed ?? 0) > 0 && (
              <Text size="1" color="red"> ({node.tasks_failed} failed)</Text>
            )}
          </Text>
        </Flex>
      )}

      {node.session_state && node.session_state !== "idle" && (
        <Flex gap="1" align="center">
          <Text size="1" color="gray">Status:</Text>
          <Text size="1">{node.session_state}</Text>
        </Flex>
      )}

      <Text size="1" color="gray">
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
  const messageCount = node.message_count ?? 0;
  const relation = getRelationInfo(node, depth);

  const hasStats = messageCount > 0 || (node.total_coins ?? 0) > 0
    || (node.total_lines_added ?? 0) > 0 || (node.total_lines_removed ?? 0) > 0;
  const showHover = hasStats || !!relation;

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

  const handleCancelEdit = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation();
      setIsEditing(false);
    },
    [],
  );

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
    <HoverCard.Root openDelay={400} closeDelay={100}>
      <HoverCard.Trigger>
        <span className={styles.titleTrigger}>
          <Text size="2" truncate className={styles.title}>
            {node.title || "New Chat"}
          </Text>
        </span>
      </HoverCard.Trigger>
      <HoverCard.Content size="1" side="top" align="center" className={styles.hoverCard} avoidCollisions>
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
      className={styles.item}
      style={indent > 0 ? { paddingLeft: `calc(var(--space-2) + ${indent}px)` } : undefined}
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
          >
            {isExpanded
              ? <ChevronDownIcon width={12} height={12} />
              : <ChevronRightIcon width={12} height={12} />}
          </button>
        ) : depth > 0 ? (
          <span className={styles.treeIndent} />
        ) : null}
        <StatusDot state={statusState} size="small" />
        {relation && (
          <Tooltip content={relation.label}>
            <span className={styles.relationIcon} style={{ color: relation.color }}>
              {relation.icon}
            </span>
          </Tooltip>
        )}
        {titleElement}
      </div>
      <div className={styles.right}>
        {hasChildren && (
          <DotTrail
            node={node}
            breakpoint={breakpoint}
            onDotClick={onDotClick}
          />
        )}
        {breakpoint !== "narrow" && node.mode && (
          <Badge
            size="1"
            color={getModeColor(node.mode)}
            variant="soft"
          >
            {node.mode}
          </Badge>
        )}
        <Text size="1" color="gray" className={styles.time}>
          {formatRelativeTime(node.updatedAt)}
        </Text>
        <div className={styles.actions}>
          {isEditing ? (
            <>
              <Tooltip content="Save">
                <button type="button" className={styles.actionButton} onClick={handleConfirmEdit} aria-label="Save rename">
                  <CheckIcon width={12} height={12} />
                </button>
              </Tooltip>
              <Tooltip content="Cancel">
                <button type="button" className={styles.actionButton} onClick={handleCancelEdit} aria-label="Cancel rename">
                  <Cross1Icon width={10} height={10} />
                </button>
              </Tooltip>
            </>
          ) : (
            <>
              {onRename && (
                <Tooltip content="Rename">
                  <button type="button" className={styles.actionButton} onClick={handleStartEdit} aria-label="Rename chat">
                    <Pencil1Icon width={12} height={12} />
                  </button>
                </Tooltip>
              )}
              {onDelete && (
                <Tooltip content="Delete">
                  <button type="button" className={styles.actionButton} onClick={handleDelete} aria-label="Delete chat">
                    <Cross1Icon width={10} height={10} />
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
