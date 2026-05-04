import React, { useCallback, useDeferredValue, useMemo, useState } from "react";
import { Badge, Flex, Skeleton, Text, TextField } from "@radix-ui/themes";
import {
  MagnifyingGlassIcon,
  ChevronDownIcon,
  ChevronUpIcon,
  PlusIcon,
} from "@radix-ui/react-icons";
import { CollapsePanel } from "../../../../components/shared/CollapsePanel";
import { Virtuoso } from "react-virtuoso";
import { useAppDispatch } from "../../../../hooks";
import { push } from "../../../Pages/pagesSlice";
import {
  useListTasksQuery,
  useCreateTaskMutation,
} from "../../../../services/refact/tasks";
import { StatusDot } from "../../../../components/StatusDot";
import { getTaskStatusDotState } from "../../../../utils/sessionStatus";
import type { TaskMeta } from "../../../../services/refact/tasks";
import type { DashboardBreakpoint } from "../../types";
import styles from "./TasksSection.module.css";

type TasksSectionProps = {
  breakpoint: DashboardBreakpoint;
  collapsed: boolean;
  projectLoading: boolean;
  onToggleCollapsed: () => void;
};

function formatTaskTime(dateStr: string): string {
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

function getDateGroup(dateStr: string): string {
  const date = new Date(dateStr);
  const now = new Date();
  const todayUTC = Date.UTC(now.getFullYear(), now.getMonth(), now.getDate());
  const dateUTC = Date.UTC(date.getFullYear(), date.getMonth(), date.getDate());
  const diffDay = Math.floor((todayUTC - dateUTC) / 86_400_000);

  if (diffDay === 0) return "Today";
  if (diffDay === 1) return "Yesterday";
  return "Earlier";
}

function getStatusColor(
  status: string,
): "blue" | "purple" | "amber" | "green" | "red" | "gray" {
  switch (status) {
    case "active":
      return "blue";
    case "planning":
      return "purple";
    case "paused":
      return "amber";
    case "completed":
      return "green";
    case "abandoned":
      return "red";
    default:
      return "gray";
  }
}

const GROUP_ORDER = ["Today", "Yesterday", "Earlier"] as const;

type FlatItem =
  | { type: "header"; label: string }
  | { type: "task"; task: TaskMeta };

function buildFlatList(tasks: TaskMeta[]): FlatItem[] {
  const groups = new Map<string, TaskMeta[]>();
  for (const label of GROUP_ORDER) groups.set(label, []);

  for (const task of tasks) {
    const group = getDateGroup(task.updated_at);
    if (!groups.has(group)) groups.set(group, []);
    groups.get(group)?.push(task);
  }

  const items: FlatItem[] = [];
  for (const [label, groupTasks] of groups) {
    if (groupTasks.length > 0) {
      items.push({ type: "header", label });
      for (const task of groupTasks) {
        items.push({ type: "task", task });
      }
    }
  }
  return items;
}

export const TasksSection: React.FC<TasksSectionProps> = ({
  breakpoint,
  collapsed,
  projectLoading,
  onToggleCollapsed,
}) => {
  const dispatch = useAppDispatch();
  const {
    data: tasks,
    isLoading,
    isFetching,
    isError,
  } = useListTasksQuery(undefined, {
    refetchOnMountOrArgChange: true,
    refetchOnFocus: true,
    refetchOnReconnect: true,
  });
  const [createTask, { isLoading: isCreatingTask }] = useCreateTaskMutation();

  const [searchQuery, setSearchQuery] = useState("");
  const deferredQuery = useDeferredValue(searchQuery);

  const sortedTasks = useMemo(() => {
    if (!tasks) return [];
    const priority = new Map([
      ["active", 0],
      ["planning", 1],
      ["paused", 2],
      ["completed", 3],
      ["abandoned", 4],
    ]);
    return [...tasks].sort((a, b) => {
      const pa = priority.get(a.status) ?? 999;
      const pb = priority.get(b.status) ?? 999;
      if (pa !== pb) return pa - pb;
      return (
        new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime()
      );
    });
  }, [tasks]);

  const filteredTasks = useMemo(() => {
    if (!deferredQuery.trim()) return sortedTasks;
    const q = deferredQuery.toLowerCase();
    return sortedTasks.filter(
      (t) =>
        t.name.toLowerCase().includes(q) || t.status.toLowerCase().includes(q),
    );
  }, [sortedTasks, deferredQuery]);

  const flatItems = useMemo(
    () => buildFlatList(filteredTasks),
    [filteredTasks],
  );

  const handleTaskClick = useCallback(
    (task: TaskMeta) => {
      dispatch(push({ name: "task workspace", taskId: task.id }));
    },
    [dispatch],
  );

  const handleNewTask = useCallback(() => {
    void createTask({ name: "New Task" })
      .unwrap()
      .then((task) => {
        dispatch(push({ name: "task workspace", taskId: task.id }));
      })
      .catch(() => {
        // Task creation failed
      });
  }, [createTask, dispatch]);

  const activeCount = filteredTasks.filter(
    (t) => t.status === "active" || t.status === "planning",
  ).length;
  const tasksLoaded = Boolean(tasks);
  const tasksLoading =
    projectLoading || isLoading || (isFetching && !tasksLoaded);
  const showTaskError = isError && !projectLoading && !tasksLoaded;

  const renderHeader = (children?: React.ReactNode) => (
    <div className={styles.header}>
      <button
        type="button"
        className={styles.headerToggle}
        onClick={onToggleCollapsed}
        aria-expanded={!collapsed}
      >
        <Text size="1" weight="bold" color="gray" className={styles.label}>
          TASKS
        </Text>
        <Flex align="center" gap="1">
          {activeCount > 0 && (
            <Text size="1" color="gray">
              {activeCount} active
            </Text>
          )}
          <Text size="1" color={showTaskError ? "red" : "gray"}>
            {tasksLoading
              ? "Loading"
              : showTaskError
                ? "Error"
                : `${filteredTasks.length} total`}
          </Text>
          {collapsed ? (
            <ChevronDownIcon width={12} height={12} color="var(--gray-9)" />
          ) : (
            <ChevronUpIcon width={12} height={12} color="var(--gray-9)" />
          )}
        </Flex>
      </button>
      {children}
    </div>
  );

  if (showTaskError) {
    return (
      <div className={styles.section} data-collapsed={collapsed || undefined}>
        {renderHeader()}
        <CollapsePanel collapsed={collapsed} className={styles.bodyPanel}>
          <Flex direction="column" align="center" gap="2" p="4">
            <Text size="2" color="red">
              Failed to load tasks
            </Text>
            <Text size="1" color="gray" align="center">
              Refact could not load the task list. Check the engine logs or try
              reconnecting.
            </Text>
          </Flex>
        </CollapsePanel>
      </div>
    );
  }

  if (tasksLoading) {
    return (
      <div className={styles.section} data-collapsed={collapsed || undefined}>
        {renderHeader()}
        <CollapsePanel collapsed={collapsed} className={styles.bodyPanel}>
          <Flex direction="column" gap="1" p="1">
            {Array.from({ length: 3 }, (_, i) => (
              <Flex key={i} align="center" gap="2" py="1" px="2">
                <Skeleton>
                  <div style={{ width: 8, height: 8, borderRadius: "50%" }} />
                </Skeleton>
                <Skeleton>
                  <Text size="2" style={{ width: `${120 + (i % 3) * 40}px` }}>
                    &nbsp;
                  </Text>
                </Skeleton>
                <div style={{ flex: 1 }} />
                <Skeleton>
                  <Text size="1" style={{ width: 40 }}>
                    &nbsp;
                  </Text>
                </Skeleton>
              </Flex>
            ))}
          </Flex>
        </CollapsePanel>
      </div>
    );
  }

  if (isError) {
    return (
      <div className={styles.section} data-collapsed={collapsed || undefined}>
        {renderHeader()}
        <CollapsePanel collapsed={collapsed} className={styles.bodyPanel}>
          <Text size="1" color="red">
            Failed to load tasks
          </Text>
        </CollapsePanel>
      </div>
    );
  }

  return (
    <div className={styles.section} data-collapsed={collapsed || undefined}>
      {renderHeader(
        <button
          type="button"
          className={styles.newTaskButton}
          onClick={handleNewTask}
          disabled={isCreatingTask}
        >
          <PlusIcon width={12} height={12} />
          <Text size="1">New Task</Text>
        </button>,
      )}
      <CollapsePanel collapsed={collapsed} className={styles.bodyPanel}>
        <div className={styles.controls}>
          <TextField.Root
            size="1"
            placeholder="Search..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
          >
            <TextField.Slot>
              <MagnifyingGlassIcon width={12} height={12} />
            </TextField.Slot>
          </TextField.Root>
        </div>

        <div className={styles.list}>
          <Virtuoso
            data={flatItems}
            overscan={200}
            className={styles.virtuosoList}
            itemContent={(_index, item) => {
              if (item.type === "header") {
                return (
                  <div className={styles.groupLabel}>
                    <Text
                      size="1"
                      color="gray"
                      className={styles.groupLabelText}
                    >
                      {item.label}
                    </Text>
                    <div className={styles.groupDivider} />
                  </div>
                );
              }
              const { task } = item;
              return (
                <div
                  role="button"
                  tabIndex={0}
                  className={styles.taskItem}
                  onClick={() => handleTaskClick(task)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                      e.preventDefault();
                      handleTaskClick(task);
                    }
                  }}
                >
                  <div className={styles.taskLeft}>
                    <span className={styles.indent} />
                    <StatusDot
                      state={getTaskStatusDotState(task)}
                      size="small"
                    />
                    <Text size="2" truncate className={styles.taskName}>
                      {task.name}
                    </Text>
                  </div>
                  <div className={styles.taskRight}>
                    {task.cards_total > 0 && (
                      <Text size="1" color="gray">
                        {task.cards_done}/{task.cards_total}
                      </Text>
                    )}
                    {breakpoint !== "narrow" && task.cards_failed > 0 && (
                      <Text size="1" color="red">
                        {task.cards_failed} failed
                      </Text>
                    )}
                    {breakpoint !== "narrow" && (
                      <Badge
                        size="1"
                        variant="soft"
                        color={getStatusColor(task.status)}
                      >
                        {task.status}
                      </Badge>
                    )}
                    <Text size="1" color="gray" className={styles.taskTime}>
                      {formatTaskTime(task.updated_at)}
                    </Text>
                  </div>
                </div>
              );
            }}
          />
          {filteredTasks.length === 0 && (
            <Text
              size="2"
              color="gray"
              style={{ padding: "var(--space-4)", textAlign: "center" }}
            >
              {searchQuery
                ? "No matching tasks"
                : "No tasks yet — start a new one!"}
            </Text>
          )}
        </div>
      </CollapsePanel>
    </div>
  );
};
