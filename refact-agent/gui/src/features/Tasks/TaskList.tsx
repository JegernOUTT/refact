import React, { useCallback, useEffect, useRef, useState } from "react";
import {
  ArrowLeft,
  CheckCircle,
  Circle,
  CircleX,
  Layers,
  LoaderCircle,
  Pause,
  Plus,
  Trash2,
} from "lucide-react";
import { ChatLoading } from "../../components/ChatContent/ChatLoading";
import {
  Button,
  Card,
  FieldTextarea,
  FieldText,
  Icon,
  IconButton,
  Badge,
  Dialog,
} from "../../components/ui";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { pop, push } from "../Pages/pagesSlice";
import {
  useListTasksQuery,
  useCreateTaskMutation,
  useDeleteTaskMutation,
  TaskMeta,
  taskErrorMessage,
} from "../../services/refact/tasks";
import { getEngineEndpointIdentity } from "../../services/refact/apiUrl";
import { selectConfig } from "../Config/configSlice";
import { openTask } from "./tasksSlice";
import { humanizeIdentifier } from "../../utils/displayNames";
import styles from "./Tasks.module.css";

// Task list keeps the product wording "Done" for `completed`; the shared
// vocabulary says "Completed", so it stays a local override. Every other
// status is plain identifier humanization.
const statusLabel = (status: TaskMeta["status"]): string =>
  status === "completed" ? "Done" : humanizeIdentifier(status);

const statusTones: Record<
  TaskMeta["status"],
  React.ComponentProps<typeof Badge>["tone"]
> = {
  planning: "muted",
  active: "accent",
  paused: "warning",
  completed: "success",
  abandoned: "danger",
};

interface TaskItemProps {
  task: TaskMeta;
  onClick: () => void;
  onDelete: () => void;
  deleteDisabled: boolean;
}

function taskStatusIcon(
  task: TaskMeta,
  plannerState: TaskMeta["planner_session_state"],
) {
  const isPlannerWorking =
    plannerState === "generating" || plannerState === "executing_tools";
  const isPlannerPaused =
    plannerState === "paused" || plannerState === "waiting_ide";

  if (isPlannerWorking)
    return { icon: LoaderCircle, tone: "accent" as const, spin: true };
  if (isPlannerPaused) return { icon: Pause, tone: "warning" as const };
  if (plannerState === "error")
    return { icon: CircleX, tone: "danger" as const };
  if (task.status === "completed")
    return { icon: CheckCircle, tone: "success" as const };
  if (task.status === "abandoned")
    return { icon: CircleX, tone: "danger" as const };
  return { icon: Circle, tone: "muted" as const };
}

const TaskItem: React.FC<TaskItemProps> = ({
  task,
  onClick,
  onDelete,
  deleteDisabled,
}) => {
  const dateUpdated = new Date(task.updated_at);
  const dateTimeString = dateUpdated.toLocaleString();
  const statusIcon = taskStatusIcon(task, task.planner_session_state);

  return (
    <Card animated="rise" className={styles.taskItem} interactive>
      <div
        className={`${styles.taskItemButton} rf-pressable`}
        role="button"
        tabIndex={0}
        onKeyDown={(event) => {
          if (event.target !== event.currentTarget) return;
          if (event.key === "Enter" || event.key === " ") {
            event.preventDefault();
            onClick();
          }
        }}
        onClick={(event) => {
          event.preventDefault();
          event.stopPropagation();
          onClick();
        }}
      >
        <span className={styles.taskItemHeader}>
          <span className={styles.taskItemTitleGroup}>
            <span className={statusIcon.spin ? styles.taskSpinner : undefined}>
              <Icon icon={statusIcon.icon} size="md" tone={statusIcon.tone} />
            </span>
            <span className={styles.taskItemTitle}>{task.name}</span>
            <Badge tone={statusTones[task.status]}>
              {statusLabel(task.status)}
            </Badge>
          </span>
          <IconButton
            aria-label="delete task"
            icon={Trash2}
            size="sm"
            variant="ghost"
            disabled={deleteDisabled}
            onClick={(event) => {
              event.preventDefault();
              event.stopPropagation();
              onDelete();
            }}
          />
        </span>

        <span className={styles.taskItemMetaRow}>
          <span className={styles.taskItemMetaGroup}>
            <span className={styles.taskItemMeta}>
              <Icon icon={Layers} size="sm" tone="muted" />
              {task.cards_done}/{task.cards_total}
              {task.cards_failed > 0 && (
                <span className={styles.taskItemDanger}>
                  ({task.cards_failed} failed)
                </span>
              )}
            </span>
            {task.agents_active > 0 && (
              <span className={styles.taskItemMetaAccent}>
                <span className={styles.taskSpinner}>
                  <Icon icon={LoaderCircle} size="sm" tone="accent" />
                </span>
                {task.agents_active} agent{task.agents_active > 1 ? "s" : ""}
              </span>
            )}
          </span>
          <span className={styles.taskItemDate}>{dateTimeString}</span>
        </span>
      </div>
    </Card>
  );
};

interface TaskListProps {
  backFromTasks?: () => void;
}

export const TaskList: React.FC<TaskListProps> = ({ backFromTasks }) => {
  const config = useAppSelector(selectConfig);
  return (
    <TaskListContent
      key={`${getEngineEndpointIdentity(config)}:${config.apiKey ?? ""}`}
      backFromTasks={backFromTasks}
    />
  );
};

const TaskListContent: React.FC<TaskListProps> = ({ backFromTasks }) => {
  const dispatch = useAppDispatch();
  const {
    data: tasks = [],
    isLoading,
    isFetching,
    error: listError,
    refetch,
  } = useListTasksQuery(undefined, {
    pollingInterval: 0,
  });
  const [createTask, { isLoading: createPending }] = useCreateTaskMutation();
  const [deleteTask, { isLoading: deletePending }] = useDeleteTaskMutation();
  const [newTaskName, setNewTaskName] = useState("");
  const [newTaskTargetFiles, setNewTaskTargetFiles] = useState("");
  const [isCreating, setIsCreating] = useState(false);
  const [createError, setCreateError] = useState<string | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<TaskMeta | null>(null);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const createInFlight = useRef(false);
  const deleteInFlight = useRef(false);
  const mounted = useRef(false);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);

  const handleBack = useCallback(() => {
    if (backFromTasks) {
      backFromTasks();
      return;
    }
    dispatch(pop());
  }, [backFromTasks, dispatch]);

  const handleCreateTask = useCallback(() => {
    if (!newTaskName.trim() || createInFlight.current) return;
    createInFlight.current = true;
    setCreateError(null);
    const targetFiles = newTaskTargetFiles
      .split(/[\n,]/)
      .map((file) => file.trim())
      .filter(Boolean);
    createTask({ name: newTaskName.trim(), target_files: targetFiles })
      .unwrap()
      .then((task) => {
        if (!mounted.current) return;
        setNewTaskName("");
        setNewTaskTargetFiles("");
        setIsCreating(false);
        dispatch(openTask({ id: task.id, name: task.name }));
        dispatch(push({ name: "task workspace", taskId: task.id }));
      })
      .catch((error: unknown) => {
        if (mounted.current) setCreateError(taskErrorMessage(error));
      })
      .finally(() => {
        createInFlight.current = false;
      });
  }, [createTask, dispatch, newTaskName, newTaskTargetFiles]);

  const handleTaskClick = useCallback(
    (task: TaskMeta) => {
      dispatch(openTask({ id: task.id, name: task.name }));
      dispatch(push({ name: "task workspace", taskId: task.id }));
    },
    [dispatch],
  );

  const handleDeleteTask = useCallback(() => {
    if (!deleteTarget || deleteInFlight.current) return;
    deleteInFlight.current = true;
    setDeleteError(null);
    void deleteTask(deleteTarget.id)
      .unwrap()
      .then(() => {
        if (mounted.current) setDeleteTarget(null);
      })
      .catch((error: unknown) => {
        if (mounted.current) setDeleteError(taskErrorMessage(error));
      })
      .finally(() => {
        deleteInFlight.current = false;
      });
  }, [deleteTarget, deleteTask]);

  const handleKeyDown = useCallback(
    (event: React.KeyboardEvent) => {
      if (event.key === "Enter") {
        event.preventDefault();
        handleCreateTask();
      } else if (event.key === "Escape") {
        if (createInFlight.current) return;
        setIsCreating(false);
      }
    },
    [handleCreateTask],
  );

  if (isLoading) {
    return <ChatLoading />;
  }

  return (
    <section className={styles.taskListRoot}>
      <header className={styles.taskListHeader}>
        <div className={styles.taskListTitleGroup}>
          <IconButton
            aria-label="Back to previous page"
            icon={ArrowLeft}
            size="sm"
            variant="ghost"
            title="Back"
            onClick={handleBack}
          />
          <h2 className={styles.taskListTitle}>Tasks</h2>
        </div>
        {!isCreating && (
          <Button
            leftIcon={Plus}
            size="sm"
            variant="soft"
            onClick={() => setIsCreating(true)}
          >
            New Task
          </Button>
        )}
      </header>

      {listError && (
        <div role="alert">
          Failed to load tasks: {taskErrorMessage(listError)}
          <Button
            size="sm"
            variant="soft"
            disabled={isFetching}
            onClick={() => void refetch()}
          >
            Retry tasks
          </Button>
        </div>
      )}
      {isFetching && <p role="status">Refreshing tasks...</p>}
      {isCreating && (
        <Card animated="rise" className={styles.taskCreateCard}>
          <div className={styles.taskCreateRow}>
            <FieldText
              className={styles.taskCreateName}
              placeholder="Task name..."
              value={newTaskName}
              onChange={setNewTaskName}
              onKeyDown={handleKeyDown}
              disabled={createPending}
              autoFocus
            />
            <Button
              onClick={handleCreateTask}
              disabled={!newTaskName.trim() || createPending}
              loading={createPending}
              variant="primary"
            >
              {createPending ? "Creating..." : "Create"}
            </Button>
            <Button
              variant="ghost"
              disabled={createPending}
              onClick={() => {
                setIsCreating(false);
              }}
            >
              Cancel
            </Button>
          </div>
          <FieldTextarea
            aria-label="Target files"
            placeholder="Target files (comma or newline separated)"
            value={newTaskTargetFiles}
            onChange={setNewTaskTargetFiles}
            disabled={createPending}
          />
          {createError && <p role="alert">Create failed: {createError}</p>}
        </Card>
      )}

      <div className={styles.taskListScroller}>
        <div className={`${styles.taskListItems} rf-stagger`}>
          {tasks.length === 0 && !listError ? (
            <p className={styles.taskListEmpty}>
              No tasks yet. Create one to start planning.
            </p>
          ) : (
            tasks.map((task) => (
              <TaskItem
                key={task.id}
                task={task}
                onClick={() => handleTaskClick(task)}
                deleteDisabled={deletePending}
                onDelete={() => {
                  setDeleteError(null);
                  setDeleteTarget(task);
                }}
              />
            ))
          )}
        </div>
      </div>
      <Dialog
        open={Boolean(deleteTarget)}
        onOpenChange={(open) => {
          if (!open && !deleteInFlight.current) setDeleteTarget(null);
        }}
      >
        <Dialog.Content maxWidth="420px">
          <Dialog.Title>Delete task</Dialog.Title>
          <Dialog.Description>
            Delete “{deleteTarget?.name}”? This cannot be undone. Tasks with
            active agents or worktree references cannot be deleted here.
          </Dialog.Description>
          {deleteError && <p role="alert">Delete failed: {deleteError}</p>}
          <Dialog.Footer>
            <Dialog.Close asChild>
              <Button variant="soft" disabled={deletePending}>
                Cancel
              </Button>
            </Dialog.Close>
            <Button
              variant="danger"
              disabled={deletePending}
              loading={deletePending}
              onClick={handleDeleteTask}
            >
              {deletePending ? "Deleting..." : "Delete task"}
            </Button>
          </Dialog.Footer>
        </Dialog.Content>
      </Dialog>
    </section>
  );
};
