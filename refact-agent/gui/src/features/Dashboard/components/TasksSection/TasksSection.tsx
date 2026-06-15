import React, { useCallback, useDeferredValue, useMemo, useState } from "react";
import {
  Badge,
  Button,
  EmptyState,
  ErrorState,
  Icon,
  LoadingState,
  StatusDot,
} from "../../../../components/ui";
import {
  DashboardText,
  DashboardTextField,
  dashboardToneFromTaskStatus,
} from "../DashboardPrimitives";
import { ChevronDown, ChevronUp, ListPlus, Search } from "lucide-react";
import { CollapsePanel } from "../../../../components/shared/CollapsePanel";
import { Virtuoso } from "react-virtuoso";
import { useAppDispatch, useAppSelector } from "../../../../hooks";
import { push } from "../../../Pages/pagesSlice";
import {
  tasksApi,
  useCreateTaskMutation,
} from "../../../../services/refact/tasks";
import { getTaskStatusDotState } from "../../../../utils/sessionStatus";
import type { TaskMeta } from "../../../../services/refact/tasks";
import type { DashboardBreakpoint } from "../../types";
import styles from "./TasksSection.module.css";

type TasksSectionProps = {
  breakpoint: DashboardBreakpoint;
  collapsed: boolean;
  projectLoading: boolean;
  loadError: string | null;
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

const GROUP_ORDER = ["Today", "Yesterday", "Earlier"] as const;
const EMPTY_TASKS: TaskMeta[] = [];

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
      if (label !== "Today") {
        items.push({ type: "header", label });
      }
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
  loadError,
  onToggleCollapsed,
}) => {
  const dispatch = useAppDispatch();
  const tasks = useAppSelector((state) => {
    const query = tasksApi.endpoints.listTasks.select(undefined)(state);
    if (query.data) return query.data;
    const seededQuery = Object.values(state.tasksApi.queries).find(
      (item) => item?.endpointName === "listTasks",
    );
    return (seededQuery?.data as TaskMeta[] | undefined) ?? EMPTY_TASKS;
  });
  const [createTask, { isLoading: isCreatingTask }] = useCreateTaskMutation();

  const [searchQuery, setSearchQuery] = useState("");
  const deferredQuery = useDeferredValue(searchQuery);

  const sortedTasks = useMemo(() => {
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
      .catch(() => undefined);
  }, [createTask, dispatch]);

  const activeCount = filteredTasks.filter(
    (t) => t.status === "active" || t.status === "planning",
  ).length;
  const showTaskError = Boolean(loadError);
  const tasksLoading = !showTaskError && projectLoading;

  const renderHeader = (children?: React.ReactNode, showSearch = false) => (
    <div className={styles.header}>
      <div className={styles.headerMain}>
        <Button
          variant="plain"
          size="sm"
          className={styles.headerToggle}
          onClick={onToggleCollapsed}
          aria-expanded={!collapsed}
          rightIcon={collapsed ? ChevronDown : ChevronUp}
        >
          <DashboardText
            size="1"
            weight="bold"
            tone="muted"
            className={styles.label}
          >
            TASKS
          </DashboardText>
        </Button>
        {showSearch && !collapsed && (
          <DashboardTextField.Root
            size="1"
            placeholder="Search..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            className={styles.searchField}
          >
            <DashboardTextField.Slot>
              <Icon icon={Search} size="sm" tone="muted" />
            </DashboardTextField.Slot>
          </DashboardTextField.Root>
        )}
      </div>
      <div className={styles.headerActions}>
        {activeCount > 0 && (
          <DashboardText size="1" tone="muted">
            {activeCount} active
          </DashboardText>
        )}
        <DashboardText size="1" tone={showTaskError ? "danger" : "muted"}>
          {tasksLoading
            ? "Loading"
            : showTaskError
              ? "Error"
              : `${filteredTasks.length} total`}
        </DashboardText>
        {children}
      </div>
    </div>
  );

  if (showTaskError) {
    return (
      <div className={styles.section} data-collapsed={collapsed || undefined}>
        {renderHeader()}
        <CollapsePanel collapsed={collapsed} className={styles.bodyPanel}>
          <ErrorState
            title="Failed to load tasks"
            error={loadError ?? "Refact could not load the task list."}
            className={styles.stateBlock}
          />
        </CollapsePanel>
      </div>
    );
  }

  if (tasksLoading) {
    return (
      <div className={styles.section} data-collapsed={collapsed || undefined}>
        {renderHeader()}
        <CollapsePanel collapsed={collapsed} className={styles.bodyPanel}>
          <LoadingState
            label="Loading tasks"
            kind="skeleton"
            className={styles.stateBlock}
          />
        </CollapsePanel>
      </div>
    );
  }

  return (
    <div className={styles.section} data-collapsed={collapsed || undefined}>
      {renderHeader(
        <Button
          variant="ghost"
          size="sm"
          className={styles.newTaskButton}
          onClick={handleNewTask}
          loading={isCreatingTask}
          leftIcon={ListPlus}
        >
          New Task
        </Button>,
        true,
      )}
      <CollapsePanel collapsed={collapsed} className={styles.bodyPanel}>
        <div className={styles.list}>
          <Virtuoso
            data={flatItems}
            overscan={200}
            className={styles.virtuosoList}
            itemContent={(_index, item) => {
              if (item.type === "header") {
                return (
                  <div className={styles.groupLabel}>
                    <DashboardText
                      size="1"
                      tone="muted"
                      className={styles.groupLabelText}
                    >
                      {item.label}
                    </DashboardText>
                    <div className={styles.groupDivider} />
                  </div>
                );
              }
              const { task } = item;
              const dotStatus = getTaskStatusDotState(task);
              return (
                <div
                  role="button"
                  tabIndex={0}
                  className={`${styles.taskItem} rf-enter-rise rf-pressable`}
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
                      status={dotStatus}
                      size="small"
                      pulse={dotStatus === "in_progress"}
                    />
                    <DashboardText
                      size="2"
                      truncate
                      className={styles.taskName}
                    >
                      {task.name}
                    </DashboardText>
                  </div>
                  <div className={styles.taskRight}>
                    {task.cards_total > 0 && (
                      <DashboardText size="1" tone="muted">
                        {task.cards_done}/{task.cards_total}
                      </DashboardText>
                    )}
                    {breakpoint !== "narrow" && task.cards_failed > 0 && (
                      <DashboardText size="1" tone="danger">
                        {task.cards_failed} failed
                      </DashboardText>
                    )}
                    {breakpoint !== "narrow" && (
                      <Badge tone={dashboardToneFromTaskStatus(task.status)}>
                        {task.status}
                      </Badge>
                    )}
                    <DashboardText
                      size="1"
                      tone="muted"
                      className={styles.taskTime}
                    >
                      {formatTaskTime(task.updated_at)}
                    </DashboardText>
                  </div>
                </div>
              );
            }}
          />
          {filteredTasks.length === 0 && (
            <EmptyState
              title={searchQuery ? "No matching tasks" : "No tasks yet"}
              description={
                searchQuery ? undefined : "Create a task when you are ready."
              }
              action={
                searchQuery ? undefined : (
                  <Button
                    variant="soft"
                    size="sm"
                    onClick={handleNewTask}
                    loading={isCreatingTask}
                    leftIcon={ListPlus}
                  >
                    New Task
                  </Button>
                )
              }
              className={styles.stateBlock}
            />
          )}
        </div>
      </CollapsePanel>
    </div>
  );
};
