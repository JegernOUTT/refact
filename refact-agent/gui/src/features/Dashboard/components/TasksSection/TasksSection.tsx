import React, { useCallback, useState } from "react";
import { Badge, Flex, Skeleton, Text } from "@radix-ui/themes";
import { ChevronDownIcon, ChevronUpIcon } from "@radix-ui/react-icons";
import { useAppDispatch } from "../../../../hooks";
import { push } from "../../../Pages/pagesSlice";
import { useListTasksQuery } from "../../../../services/refact/tasks";
import { StatusDot } from "../../../../components/StatusDot";
import { getTaskStatusDotState } from "../../../../utils/sessionStatus";
import type { TaskMeta } from "../../../../services/refact/tasks";
import type { DashboardBreakpoint } from "../../types";
import styles from "./TasksSection.module.css";

type TasksSectionProps = {
  breakpoint: DashboardBreakpoint;
  compact?: boolean;
};

const INITIAL_VISIBLE = 4;

function formatTaskTime(dateStr: string): string {
  const date = new Date(dateStr);
  const now = new Date();
  const diffMs = now.getTime() - date.getTime();
  const diffHr = Math.floor(diffMs / 3_600_000);
  const diffDay = Math.floor(diffMs / 86_400_000);

  if (diffHr < 1) return "just now";
  if (diffHr < 24) return `${diffHr}h ago`;
  if (diffDay < 7) return `${diffDay}d ago`;
  return date.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

function getStatusColor(status: string): "blue" | "purple" | "amber" | "green" | "red" | "gray" {
  switch (status) {
    case "active": return "blue";
    case "planning": return "purple";
    case "paused": return "amber";
    case "completed": return "green";
    case "abandoned": return "red";
    default: return "gray";
  }
}

export const TasksSection: React.FC<TasksSectionProps> = ({
  breakpoint,
  compact,
}) => {
  const dispatch = useAppDispatch();
  const { data: tasks, isLoading, isError } = useListTasksQuery(undefined);
  const [showAll, setShowAll] = useState(false);

  const sortedTasks = React.useMemo(() => {
    if (!tasks) return [];
    // Active/planning/paused first, then completed/abandoned
    const priority = new Map([
      ["active", 0], ["planning", 1], ["paused", 2], ["completed", 3], ["abandoned", 4],
    ]);
    return [...tasks].sort((a, b) => {
      const pa = priority.get(a.status) ?? 999;
      const pb = priority.get(b.status) ?? 999;
      if (pa !== pb) return pa - pb;
      return new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime();
    });
  }, [tasks]);

  const handleTaskClick = useCallback(
    (task: TaskMeta) => {
      dispatch(push({ name: "task workspace", taskId: task.id }));
    },
    [dispatch],
  );

  const toggleShowAll = useCallback(() => {
    setShowAll((prev) => !prev);
  }, []);

  if (isLoading) {
    return (
      <div className={styles.section}>
        <div className={styles.header}>
          <Text size="1" weight="bold" color="gray" className={styles.label}>TASKS</Text>
        </div>
        <Skeleton height="32px" />
      </div>
    );
  }

  if (isError) {
    return (
      <div className={styles.section}>
        <div className={styles.header}>
          <Text size="1" weight="bold" color="gray" className={styles.label}>TASKS</Text>
        </div>
        <Text size="1" color="red">Failed to load tasks</Text>
      </div>
    );
  }

  if (sortedTasks.length === 0) return null;

  if (compact) {
    const activeCount = sortedTasks.filter(
      (t) => t.status === "active" || t.status === "planning" || t.status === "paused",
    ).length;
    return (
      <Text size="1" color="gray">
        📋 {activeCount} active / {sortedTasks.length} total tasks
      </Text>
    );
  }

  const visibleTasks = showAll ? sortedTasks : sortedTasks.slice(0, INITIAL_VISIBLE);
  const hiddenCount = sortedTasks.length - INITIAL_VISIBLE;

  return (
    <div className={styles.section}>
      <div className={styles.header}>
        <Text size="1" weight="bold" color="gray" className={styles.label}>
          TASKS ({sortedTasks.length})
        </Text>
      </div>
      <div className={styles.list}>
        {visibleTasks.map((task) => {
          const progress = task.cards_total > 0
            ? Math.round((task.cards_done / task.cards_total) * 100)
            : 0;
          return (
            <button
              key={task.id}
              type="button"
              className={styles.taskRow}
              onClick={() => handleTaskClick(task)}
              aria-label={`Task: ${task.name}, status: ${task.status}`}
            >
              <StatusDot state={getTaskStatusDotState(task)} size="small" />
              <div className={styles.taskInfo}>
                <Text size="2" weight="medium" truncate className={styles.taskName}>
                  {task.name}
                </Text>
                <div className={styles.taskMeta}>
                  {task.cards_total > 0 && (
                    <>
                      <div className={styles.progressBar}>
                        <div
                          className={styles.progressFill}
                          style={{ width: `${progress}%` }}
                        />
                      </div>
                      <Text size="1" color="gray">
                        {task.cards_done}/{task.cards_total}
                        {task.cards_failed > 0 && (
                          <Text size="1" color="red"> ({task.cards_failed} failed)</Text>
                        )}
                      </Text>
                    </>
                  )}
                  {breakpoint !== "narrow" && task.agents_active > 0 && (
                    <Text size="1" color="gray">
                      {task.agents_active} agent{task.agents_active !== 1 ? "s" : ""}
                    </Text>
                  )}
                  {breakpoint !== "narrow" && (
                    <Text size="1" color="gray">
                      {formatTaskTime(task.updated_at)}
                    </Text>
                  )}
                </div>
              </div>
              <Flex gap="1" align="center" flexShrink="0">
                <Badge size="1" variant="soft" color={getStatusColor(task.status)}>
                  {task.status}
                </Badge>
              </Flex>
            </button>
          );
        })}
      </div>
      {hiddenCount > 0 && (
        <button
          type="button"
          className={styles.viewAll}
          onClick={toggleShowAll}
        >
          <Text size="1" color="gray">
            {showAll ? "Show less" : `View all ${sortedTasks.length} tasks`}
          </Text>
          {showAll
            ? <ChevronUpIcon width={12} height={12} color="var(--gray-9)" />
            : <ChevronDownIcon width={12} height={12} color="var(--gray-9)" />
          }
        </button>
      )}
    </div>
  );
};
