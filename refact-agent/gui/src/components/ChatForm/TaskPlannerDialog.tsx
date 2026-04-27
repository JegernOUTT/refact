import React, { useCallback, useState } from "react";
import {
  Dialog,
  Flex,
  Text,
  Button,
  Callout,
  Badge,
  Spinner,
} from "@radix-ui/themes";
import { ExclamationTriangleIcon } from "@radix-ui/react-icons";
import {
  createChatWithId,
  requestSseRefresh,
} from "../../features/Chat/Thread/actions";
import {
  openTask,
  addPlannerChat,
  setTaskActiveChat,
} from "../../features/Tasks/tasksSlice";
import { push } from "../../features/Pages/pagesSlice";
import { useAppDispatch } from "../../hooks";
import {
  useCreateTaskMutation,
  useCreatePlannerChatMutation,
} from "../../services/refact/tasks";
import styles from "./ModeTransitionDialog.module.css";

function extractErrorMessage(err: unknown): string {
  if (err && typeof err === "object") {
    const obj = err as Record<string, unknown>;
    if (obj.data && typeof obj.data === "object") {
      const data = obj.data as Record<string, unknown>;
      if (typeof data.detail === "string") return data.detail;
    }
    if (typeof obj.data === "string") return obj.data;
    if (typeof obj.message === "string") return obj.message;
  }
  if (err instanceof Error) return err.message;
  return "Failed to create task planner";
}

type TaskPlannerDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Present when opened from inside a task workspace — adds planner to that task */
  taskId?: string;
};

export const TaskPlannerDialog: React.FC<TaskPlannerDialogProps> = ({
  open,
  onOpenChange,
  taskId,
}) => {
  const dispatch = useAppDispatch();
  const [error, setError] = useState<string | null>(null);

  const [createTask, { isLoading: isCreatingTask }] = useCreateTaskMutation();
  const [createPlannerChat, { isLoading: isCreatingPlanner }] =
    useCreatePlannerChatMutation();

  const isInTaskWorkspace = taskId !== undefined;
  const isLoading = isCreatingTask || isCreatingPlanner;

  const handleApply = useCallback(async () => {
    setError(null);
    const now = new Date().toISOString();
    try {
      let resolvedTaskId: string;

      if (isInTaskWorkspace && taskId) {
        resolvedTaskId = taskId;
      } else {
        // Create new task first
        const task = await createTask({ name: "New Task" }).unwrap();
        resolvedTaskId = task.id;
        dispatch(openTask({ id: resolvedTaskId, name: task.name }));
      }

      // Always use the task-owned planner endpoint — this is the only correct way
      // to create a backend trajectory that belongs to the task
      const result = await createPlannerChat(resolvedTaskId).unwrap();
      const newChatId = result.chat_id;

      // Wire up the Redux thread with full task metadata — same as TaskWorkspace.handleNewPlanner
      dispatch(
        createChatWithId({
          id: newChatId,
          title: "",
          isTaskChat: true,
          mode: "TASK_PLANNER",
          taskMeta: { task_id: resolvedTaskId, role: "planner" },
        }),
      );
      dispatch(requestSseRefresh({ chatId: newChatId }));
      dispatch(
        addPlannerChat({
          taskId: resolvedTaskId,
          planner: { id: newChatId, title: "", createdAt: now, updatedAt: now },
        }),
      );
      dispatch(
        setTaskActiveChat({
          taskId: resolvedTaskId,
          activeChat: { type: "planner", chatId: newChatId },
        }),
      );

      if (!isInTaskWorkspace) {
        dispatch(push({ name: "task workspace", taskId: resolvedTaskId }));
      }

      onOpenChange(false);
    } catch (err) {
      setError(extractErrorMessage(err));
    }
  }, [
    isInTaskWorkspace,
    taskId,
    createTask,
    createPlannerChat,
    dispatch,
    onOpenChange,
  ]);

  const handleOpenChange = useCallback(
    (newOpen: boolean) => {
      if (!newOpen) setError(null);
      onOpenChange(newOpen);
    },
    [onOpenChange],
  );

  const title = isInTaskWorkspace ? "New Planner" : "Switch to Task Planner";
  const description = isInTaskWorkspace
    ? "Create a new planner chat in this task."
    : "Create a new task and open the Task Planner.";
  const buttonLabel = isInTaskWorkspace ? "Create Planner" : "Create Task";
  const loadingLabel = isCreatingTask
    ? "Creating task..."
    : "Creating planner...";

  return (
    <Dialog.Root open={open} onOpenChange={handleOpenChange}>
      <Dialog.Content maxWidth="500px" className={styles.dialogContent}>
        <Dialog.Title>
          <Flex align="center" gap="2">
            <Text>{title}</Text>
            <Badge color="blue">task_planner</Badge>
          </Flex>
        </Dialog.Title>

        <Dialog.Description size="2" color="gray">
          {description}
        </Dialog.Description>

        {error && (
          <Callout.Root color="red" className={styles.callout}>
            <Callout.Icon>
              <ExclamationTriangleIcon />
            </Callout.Icon>
            <Callout.Text>{error}</Callout.Text>
          </Callout.Root>
        )}

        {isLoading && (
          <Flex
            align="center"
            justify="center"
            gap="2"
            className={styles.loadingContainer}
          >
            <Spinner />
            <Text color="gray">{loadingLabel}</Text>
          </Flex>
        )}

        <Flex gap="3" mt="4" justify="end">
          <Dialog.Close>
            <Button variant="soft" color="gray" disabled={isLoading}>
              Cancel
            </Button>
          </Dialog.Close>
          <Button onClick={() => void handleApply()} disabled={isLoading}>
            {isLoading ? (
              <>
                <Spinner size="1" />
                {loadingLabel}
              </>
            ) : (
              buttonLabel
            )}
          </Button>
        </Flex>
      </Dialog.Content>
    </Dialog.Root>
  );
};

TaskPlannerDialog.displayName = "TaskPlannerDialog";
