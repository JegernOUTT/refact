import { useCallback, useEffect, useMemo, useState } from "react";

import { Badge, Button, LoadingState, StatusDot } from "../../../components/ui";
import { useAppDispatch } from "../../../hooks";
import {
  type TaskBoard,
  useGetBoardQuery,
  useListTasksQuery,
} from "../../../services/refact/tasks";
import { push } from "../../Pages/pagesSlice";
import { openTask } from "../../Tasks/tasksSlice";
import styles from "./TasksSection.module.css";
import {
  buildTaskDockEntries,
  sortTaskDockEntries,
  type TaskDockAgentStatus,
  type TaskDockEntry,
} from "./tasksSectionModel";

type TasksSectionViewProps = {
  entries: TaskDockEntry[];
  isLoading: boolean;
  onOpenBoard: () => void;
  onOpenTask: (entry: TaskDockEntry) => void;
};

type BoardState = {
  board?: TaskBoard;
  loading: boolean;
};

const statusDot: Record<
  TaskDockAgentStatus,
  { label: string; status: "running" | "warning" | "error" | "success" }
> = {
  running: { label: "Running agent", status: "running" },
  stuck: { label: "Stuck agent", status: "warning" },
  failed: { label: "Failed agent", status: "error" },
  done: { label: "Done agent", status: "success" },
};

const statusBadgeTone: Record<
  TaskDockAgentStatus,
  "accent" | "warning" | "danger" | "success"
> = {
  running: "accent",
  stuck: "warning",
  failed: "danger",
  done: "success",
};

export function TasksSectionView({
  entries,
  isLoading,
  onOpenBoard,
  onOpenTask,
}: TasksSectionViewProps) {
  const orderedEntries = useMemo(() => sortTaskDockEntries(entries), [entries]);
  const hasAttention = orderedEntries.some(
    (entry) => entry.agentStatus === "stuck" || entry.agentStatus === "failed",
  );
  const hasRunning = orderedEntries.some(
    (entry) => entry.agentStatus === "running",
  );

  return (
    <section className={styles.section} aria-label="Tasks dock section">
      <header className={styles.header}>
        <span className={styles.heading}>In-flight work</span>
        {hasAttention ? (
          <StatusDot
            aria-label="Tasks need attention"
            size="small"
            status="needs_attention"
          />
        ) : hasRunning ? (
          <StatusDot
            aria-label="Tasks running"
            pulse
            size="small"
            status="running"
          />
        ) : null}
      </header>
      {isLoading ? (
        <LoadingState className={styles.state} label="Loading active tasks" />
      ) : orderedEntries.length === 0 ? (
        <div className={styles.empty}>
          <span>No active tasks</span>
          <Button size="sm" variant="soft" onClick={onOpenBoard}>
            Open board
          </Button>
        </div>
      ) : (
        <div className={styles.list}>
          {orderedEntries.map((entry) => {
            const dot = statusDot[entry.agentStatus];
            return (
              <button
                key={`${entry.taskId}:${entry.cardId}`}
                className={`${styles.card} rf-pressable`}
                type="button"
                onClick={() => onOpenTask(entry)}
              >
                <span className={styles.cardLead}>
                  <StatusDot
                    aria-label={dot.label}
                    pulse={entry.agentStatus === "running"}
                    size="small"
                    status={dot.status}
                  />
                  <span className={styles.cardCopy}>
                    <span className={styles.cardTitle}>{entry.title}</span>
                    <span className={styles.taskName}>{entry.taskName}</span>
                  </span>
                </span>
                <Badge
                  size="xs"
                  tone={statusBadgeTone[entry.agentStatus]}
                  variant="outline"
                >
                  {entry.columnLabel}
                </Badge>
              </button>
            );
          })}
        </div>
      )}
    </section>
  );
}

function TaskBoardLoader({
  taskId,
  onBoardState,
}: {
  taskId: string;
  onBoardState: (taskId: string, state?: BoardState) => void;
}) {
  const { data, isLoading } = useGetBoardQuery(taskId, {
    pollingInterval: 0,
  });

  useEffect(() => {
    onBoardState(taskId, { board: data, loading: isLoading });
  }, [data, isLoading, onBoardState, taskId]);

  useEffect(
    () => () => {
      onBoardState(taskId, undefined);
    },
    [onBoardState, taskId],
  );

  return null;
}

export function TasksSection() {
  const dispatch = useAppDispatch();
  const { data: tasks = [], isLoading: tasksLoading } = useListTasksQuery(
    undefined,
    { pollingInterval: 0 },
  );
  const [boardStates, setBoardStates] = useState<
    Record<string, BoardState | undefined>
  >({});
  const activeTasks = useMemo(
    () =>
      tasks.filter(
        (task) =>
          task.status === "planning" ||
          task.status === "active" ||
          task.status === "paused",
      ),
    [tasks],
  );
  const handleBoardState = useCallback((taskId: string, state?: BoardState) => {
    setBoardStates((current) => {
      if (current[taskId] === state) return current;
      if (!state) {
        const { [taskId]: _removed, ...remaining } = current;
        return remaining;
      }
      return { ...current, [taskId]: state };
    });
  }, []);
  const boardsByTask = useMemo(
    () =>
      Object.fromEntries(
        Object.entries(boardStates).map(([taskId, state]) => [
          taskId,
          state?.board,
        ]),
      ),
    [boardStates],
  );
  const entries = useMemo(
    () => buildTaskDockEntries(activeTasks, boardsByTask, Date.now()),
    [activeTasks, boardsByTask],
  );
  const boardsLoading = activeTasks.some(
    (task) => boardStates[task.id]?.loading !== false,
  );
  const handleOpenBoard = useCallback(() => {
    dispatch(push({ name: "tasks list" }));
  }, [dispatch]);
  const handleOpenTask = useCallback(
    (entry: TaskDockEntry) => {
      dispatch(openTask({ id: entry.taskId, name: entry.taskName }));
      dispatch(push({ name: "task workspace", taskId: entry.taskId }));
    },
    [dispatch],
  );

  return (
    <>
      {activeTasks.map((task) => (
        <TaskBoardLoader
          key={task.id}
          taskId={task.id}
          onBoardState={handleBoardState}
        />
      ))}
      <TasksSectionView
        entries={entries}
        isLoading={tasksLoading || boardsLoading}
        onOpenBoard={handleOpenBoard}
        onOpenTask={handleOpenTask}
      />
    </>
  );
}
