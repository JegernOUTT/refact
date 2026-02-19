import React, { useState, useCallback } from "react";
import {
  Text,
  IconButton,
  TextField,
  HoverCard,
  Badge,
} from "@radix-ui/themes";
import { Cross1Icon, Pencil1Icon, CheckIcon } from "@radix-ui/react-icons";
import { StatusDot } from "../StatusDot";
import { getTaskStatusDotState } from "../../utils/sessionStatus";
import { CircularProgress } from "./CircularProgress";
import type { TaskMeta } from "../../services/refact/tasks";
import styles from "./HistoryItemCompact.module.css";

export interface TaskItemCompactProps {
  task: TaskMeta;
  onClick: () => void;
  onDelete: (id: string) => void;
  onRename?: (id: string, newName: string) => void;
  badge?: string;
}

function getTaskTooltip(task: TaskMeta): string {
  const plannerState = task.planner_session_state;

  if (plannerState === "generating" || plannerState === "executing_tools") {
    return "Planner is working...";
  }
  if (plannerState === "paused" || plannerState === "waiting_ide") {
    return "Waiting for confirmation";
  }
  if (plannerState === "error") {
    return "Planner error";
  }
  if (task.status === "abandoned") {
    return "Task failed";
  }
  if (task.status === "completed") {
    return "Task completed";
  }
  if (task.agents_active > 0) {
    return `${task.agents_active} agent${
      task.agents_active > 1 ? "s" : ""
    } working...`;
  }
  if (task.status === "paused") {
    return "Task paused";
  }
  return "Task active";
}

function formatDateTime(dateString: string): string {
  const date = new Date(dateString);
  const now = new Date();
  const diffMs = now.getTime() - date.getTime();
  const diffDays = Math.floor(diffMs / (1000 * 60 * 60 * 24));

  if (diffDays === 0) {
    return date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }
  if (diffDays === 1) {
    return "Yesterday";
  }
  if (diffDays < 7) {
    return date.toLocaleDateString([], { weekday: "short" });
  }
  return date.toLocaleDateString([], { month: "short", day: "numeric" });
}

interface TooltipButtonProps {
  onClick: (e: React.MouseEvent) => void;
  tooltip: string;
  children: React.ReactNode;
  className?: string;
}

const TooltipButton: React.FC<TooltipButtonProps> = ({
  onClick,
  tooltip,
  children,
  className,
}) => (
  <HoverCard.Root openDelay={200} closeDelay={100}>
    <HoverCard.Trigger>
      <IconButton
        size="1"
        variant="ghost"
        onClick={onClick}
        className={className}
        aria-label={tooltip}
      >
        {children}
      </IconButton>
    </HoverCard.Trigger>
    <HoverCard.Content size="1" side="top" align="center">
      <Text as="p" size="1">
        {tooltip}
      </Text>
    </HoverCard.Content>
  </HoverCard.Root>
);

export const TaskItemCompact: React.FC<TaskItemCompactProps> = ({
  task,
  onClick,
  onDelete,
  onRename,
  badge,
}) => {
  const [isEditing, setIsEditing] = useState(false);
  const [editValue, setEditValue] = useState(task.name);
  const statusState = getTaskStatusDotState(task);
  const tooltipText = getTaskTooltip(task);
  const dateTimeString = formatDateTime(task.updated_at);

  const handleStartEdit = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      e.stopPropagation();
      setEditValue(task.name);
      setIsEditing(true);
    },
    [task.name],
  );

  const handleCancelEdit = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      e.stopPropagation();
      setIsEditing(false);
      setEditValue(task.name);
    },
    [task.name],
  );

  const handleConfirmEdit = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (editValue.trim() && onRename) {
        onRename(task.id, editValue.trim());
      }
      setIsEditing(false);
    },
    [editValue, task.id, onRename],
  );

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter") {
        e.preventDefault();
        if (editValue.trim() && onRename) {
          onRename(task.id, editValue.trim());
        }
        setIsEditing(false);
      } else if (e.key === "Escape") {
        setIsEditing(false);
        setEditValue(task.name);
      }
    },
    [editValue, task.id, task.name, onRename],
  );

  const handleDelete = useCallback(
    (e: React.MouseEvent) => {
      e.preventDefault();
      e.stopPropagation();
      onDelete(task.id);
    },
    [task.id, onDelete],
  );

  const handleClick = useCallback(() => {
    if (!isEditing) {
      onClick();
    }
  }, [isEditing, onClick]);

  const handleRowKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.target !== e.currentTarget) return;
      if ((e.key === "Enter" || e.key === " ") && !isEditing) {
        e.preventDefault();
        onClick();
      }
    },
    [isEditing, onClick],
  );

  return (
    <div className={styles.itemContainer}>
      <div
        className={styles.item}
        onClick={handleClick}
        role="button"
        tabIndex={0}
        onKeyDown={handleRowKeyDown}
      >
        <div className={styles.chevronArea} />

        <div className={styles.leftSection}>
          <StatusDot
            state={statusState}
            size="small"
            tooltipText={tooltipText}
          />
          {badge && (
            <Badge
              size="1"
              color="gray"
              variant="soft"
              style={{ flexShrink: 0 }}
            >
              {badge}
            </Badge>
          )}
        </div>

        <div className={styles.titleSection}>
          {isEditing ? (
            <TextField.Root
              size="1"
              value={editValue}
              onChange={(e) => setEditValue(e.target.value)}
              onKeyDown={handleKeyDown}
              onClick={(e) => e.stopPropagation()}
              autoFocus
              className={styles.editInput}
            />
          ) : (
            <Text as="span" size="2" className={styles.title}>
              {task.name}
            </Text>
          )}
        </div>

        <div className={styles.stats}>
          <CircularProgress
            done={task.cards_done}
            total={task.cards_total}
            failed={task.cards_failed}
          />
        </div>

        <Text size="1" color="gray" className={styles.date}>
          {dateTimeString}
        </Text>

        <div className={styles.actions}>
          {isEditing ? (
            <>
              <TooltipButton onClick={handleConfirmEdit} tooltip="Save">
                <CheckIcon width={12} height={12} />
              </TooltipButton>
              <TooltipButton onClick={handleCancelEdit} tooltip="Cancel">
                <Cross1Icon width={10} height={10} />
              </TooltipButton>
            </>
          ) : (
            <>
              {onRename && (
                <TooltipButton
                  onClick={handleStartEdit}
                  tooltip="Rename"
                  className={styles.actionButton}
                >
                  <Pencil1Icon width={12} height={12} />
                </TooltipButton>
              )}
              <TooltipButton
                onClick={handleDelete}
                tooltip="Delete"
                className={styles.actionButton}
              >
                <Cross1Icon width={10} height={10} />
              </TooltipButton>
            </>
          )}
        </div>
      </div>
    </div>
  );
};
