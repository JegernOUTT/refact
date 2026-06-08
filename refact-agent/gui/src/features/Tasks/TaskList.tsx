import React, { useCallback, useState } from "react";
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
} from "../../components/ui";
import { useAppDispatch } from "../../hooks";
import { pop, push } from "../Pages/pagesSlice";
import {
  useListTasksQuery,
  useCreateTaskMutation,
  useDeleteTaskMutation,
  TaskMeta,
} from "../../services/refact/tasks";
import { openTask } from "./tasksSlice";
import styles from "./Tasks.module.css";

const statusLabels: Record<TaskMeta["status"], string> = {
  planning: "Planning",
  active: "Active",
  paused: "Paused",
  completed: "Done",
  abandoned: "Abandoned",
};

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

const TaskItem: React.FC<TaskItemProps> = ({ task, onClick, onDelete }) => {
  const dateUpdated = new Date(task.updated_at);
  const dateTimeString = dateUpdated.toLocaleString();
  const statusIcon = taskStatusIcon(task, task.planner_session_state);

  return (
    <Card animated="rise" className={styles.taskItem} interactive>
      <button
        className={styles.taskItemButton}
        type="button"
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
              {statusLabels[task.status]}
            </Badge>
          </span>
          <IconButton
            aria-label="delete task"
            icon={Trash2}
            size="sm"
            variant="ghost"
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
      </button>
    </Card>
  );
};

interface TaskListProps {
  backFromTasks?: () => void;
}

export const TaskList: React.FC<TaskListProps> = ({ backFromTasks }) => {
  const dispatch = useAppDispatch();
  const { data: tasks = [], isLoading } = useListTasksQuery(undefined, {
    pollingInterval: 0,
  });
  const [createTask] = useCreateTaskMutation();
  const [deleteTask] = useDeleteTaskMutation();
  const [newTaskName, setNewTaskName] = useState("");
  const [newTaskTargetFiles, setNewTaskTargetFiles] = useState("");
  const [isCreating, setIsCreating] = useState(false);

  const handleBack = useCallback(() => {
    if (backFromTasks) {
      backFromTasks();
      return;
    }
    dispatch(pop());
  }, [backFromTasks, dispatch]);

  const handleCreateTask = useCallback(() => {
    if (!newTaskName.trim()) return;
    const targetFiles = newTaskTargetFiles
      .split(/[\n,]/)
      .map((file) => file.trim())
      .filter(Boolean);
    createTask({ name: newTaskName.trim(), target_files: targetFiles })
      .unwrap()
      .then((task) => {
        setNewTaskName("");
        setNewTaskTargetFiles("");
        setIsCreating(false);
        dispatch(openTask({ id: task.id, name: task.name }));
        dispatch(push({ name: "task workspace", taskId: task.id }));
      })
      .catch(() => undefined);
  }, [createTask, dispatch, newTaskName, newTaskTargetFiles]);

  const handleTaskClick = useCallback(
    (task: TaskMeta) => {
      dispatch(openTask({ id: task.id, name: task.name }));
      dispatch(push({ name: "task workspace", taskId: task.id }));
    },
    [dispatch],
  );

  const handleDeleteTask = useCallback(
    (taskId: string) => {
      void deleteTask(taskId);
    },
    [deleteTask],
  );

  const handleKeyDown = useCallback(
    (event: React.KeyboardEvent) => {
      if (event.key === "Enter") {
        handleCreateTask();
      } else if (event.key === "Escape") {
        setIsCreating(false);
        setNewTaskName("");
        setNewTaskTargetFiles("");
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

      {isCreating && (
        <Card animated="rise" className={styles.taskCreateCard}>
          <div className={styles.taskCreateRow}>
            <FieldText
              className={styles.taskCreateName}
              placeholder="Task name..."
              value={newTaskName}
              onChange={setNewTaskName}
              onKeyDown={handleKeyDown}
              autoFocus
            />
            <Button
              onClick={handleCreateTask}
              disabled={!newTaskName.trim()}
              variant="primary"
            >
              Create
            </Button>
            <Button
              variant="ghost"
              onClick={() => {
                setIsCreating(false);
                setNewTaskName("");
                setNewTaskTargetFiles("");
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
          />
        </Card>
      )}

      <div className={styles.taskListScroller}>
        <div className={styles.taskListItems}>
          {tasks.length === 0 ? (
            <p className={styles.taskListEmpty}>
              No tasks yet. Create one to start planning.
            </p>
          ) : (
            tasks.map((task) => (
              <TaskItem
                key={task.id}
                task={task}
                onClick={() => handleTaskClick(task)}
                onDelete={() => handleDeleteTask(task.id)}
              />
            ))
          )}
        </div>
      </div>
    </section>
  );
};
