import React, { useCallback, useState } from "react";
import { Flex, Text, Button, Badge } from "@radix-ui/themes";
import { LoaderCircle } from "lucide-react";
import { Dialog, Icon } from "../ui";
import { Callout } from "../Callout";
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
import { useAppDispatch, useAppSelector } from "../../hooks";
import { selectConfig, selectApiKey } from "../../features/Config/configSlice";
import {
  selectMessagesById,
  selectThreadWorktreeById,
} from "../../features/Chat/Thread";
import { regenerate } from "../../services/refact/chatCommands";
import { dialogNonInteractiveCloseHandlers } from "../../utils/dialogPointerClose";
import {
  useCreateTaskMutation,
  useDeleteTaskMutation,
  useCreatePlannerChatMutation,
  useCreatePlannerChatFromTransitionMutation,
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
  sourceChatId: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** Present when opened from inside a task workspace; otherwise a new task is created */
  taskId?: string;
  /** Description of the task_planner mode, used for context-transfer analysis */
  targetModeDescription?: string;
};

type PendingTask = { id: string; name: string };

export const TaskPlannerDialog: React.FC<TaskPlannerDialogProps> = ({
  sourceChatId,
  open,
  onOpenChange,
  taskId,
  targetModeDescription,
}) => {
  const dispatch = useAppDispatch();
  const config = useAppSelector(selectConfig);
  const apiKey = useAppSelector(selectApiKey);
  const messages = useAppSelector((state) =>
    selectMessagesById(state, sourceChatId),
  );
  const sourceWorktree = useAppSelector((state) =>
    selectThreadWorktreeById(state, sourceChatId),
  );

  const [error, setError] = useState<string | null>(null);
  // Cache the created task so retries after a planner-creation failure don't
  // create more tasks. Persists across cancel/reopen so we don't leak tasks
  // that the user gave up on; cleared only on success or explicit rollback.
  const [pendingTask, setPendingTask] = useState<PendingTask | null>(null);

  const [createTask, { isLoading: isCreatingTask }] = useCreateTaskMutation();
  const [deleteTask] = useDeleteTaskMutation();
  const [createPlannerChat, { isLoading: isCreatingPlanner }] =
    useCreatePlannerChatMutation();
  const [createFromTransition, { isLoading: isTransitioning }] =
    useCreatePlannerChatFromTransitionMutation();

  const isInTaskWorkspace = taskId !== undefined;
  const hasMessages = messages.length > 0 && Boolean(sourceChatId);
  const isLoading = isCreatingTask || isCreatingPlanner || isTransitioning;

  const handleApply = useCallback(async () => {
    if (isLoading) return;
    setError(null);
    const now = new Date().toISOString();
    try {
      // Resolve / lazily create the target task. We track whether we just
      // created it here so the success path can register it with the UI exactly
      // once — including on retries after a previous planner failure.
      let resolved: PendingTask;
      let createdHere = false;
      if (isInTaskWorkspace && taskId) {
        resolved = { id: taskId, name: "" };
      } else if (pendingTask) {
        resolved = pendingTask;
        createdHere = true;
      } else {
        const task = await createTask({ name: "New Task" }).unwrap();
        resolved = { id: task.id, name: task.name };
        setPendingTask(resolved);
        createdHere = true;
      }

      // Create the planner chat — task-owned, with context if available
      let newChatId: string;
      let rootChatId: string | undefined;
      if (hasMessages && sourceChatId) {
        const result = await createFromTransition({
          taskId: resolved.id,
          sourceChatId,
          targetModeDescription: targetModeDescription ?? "",
        }).unwrap();
        newChatId = result.new_chat_id;
        rootChatId = result.root_chat_id ?? undefined;
      } else {
        const result = await createPlannerChat({
          taskId: resolved.id,
        }).unwrap();
        newChatId = result.chat_id;
      }

      // Wire up Redux thread with full task metadata — same as TaskWorkspace.handleNewPlanner
      dispatch(
        createChatWithId({
          id: newChatId,
          title: "",
          isTaskChat: true,
          mode: "TASK_PLANNER",
          taskMeta: {
            task_id: resolved.id,
            role: "planner",
            planner_chat_id: newChatId,
          },
          rootChatId,
          worktree: sourceWorktree,
        }),
      );
      dispatch(requestSseRefresh({ chatId: newChatId }));

      // Always openTask before addPlannerChat so the planner registration
      // hits an existing entry in the slice — necessary on retry too.
      if (createdHere) {
        dispatch(openTask({ id: resolved.id, name: resolved.name }));
      }
      dispatch(
        addPlannerChat({
          taskId: resolved.id,
          planner: { id: newChatId, title: "", createdAt: now, updatedAt: now },
        }),
      );
      dispatch(
        setTaskActiveChat({
          taskId: resolved.id,
          activeChat: { type: "planner", chatId: newChatId },
        }),
      );

      if (!isInTaskWorkspace) {
        dispatch(push({ name: "task workspace", taskId: resolved.id }));
      }

      // Successful — clear the pending task so a subsequent open creates fresh
      setPendingTask(null);
      onOpenChange(false);

      // Kick off generation if context was transferred
      if (hasMessages && sourceChatId) {
        void regenerate(newChatId, config, apiKey ?? undefined);
      }
    } catch (err) {
      setError(extractErrorMessage(err));
    }
  }, [
    isLoading,
    isInTaskWorkspace,
    taskId,
    pendingTask,
    hasMessages,
    sourceChatId,
    targetModeDescription,
    createTask,
    createPlannerChat,
    createFromTransition,
    dispatch,
    onOpenChange,
    config,
    apiKey,
    sourceWorktree,
  ]);

  const handleOpenChange = useCallback(
    (newOpen: boolean) => {
      if (!newOpen && isLoading) {
        return;
      }
      if (!newOpen) {
        // If the user is closing after a failed attempt with a half-created
        // task, roll it back so we don't leak orphan tasks.
        if (pendingTask) {
          void deleteTask(pendingTask.id);
        }
        setError(null);
        setPendingTask(null);
      }
      onOpenChange(newOpen);
    },
    [onOpenChange, pendingTask, deleteTask, isLoading],
  );

  const title = isInTaskWorkspace ? "New Planner" : "Switch to Task Planner";
  const description = isInTaskWorkspace
    ? hasMessages
      ? "The assistant will analyze the current chat and create a new planner chat with the relevant context."
      : "Create a new planner chat in this task."
    : hasMessages
      ? "The assistant will analyze your conversation, create a new task, and start a planner chat with the relevant context."
      : "Create a new task and open the Task Planner.";
  const buttonLabel = isInTaskWorkspace ? "Create Planner" : "Create Task";
  const loadingLabel = isCreatingTask
    ? "Creating task..."
    : isTransitioning
      ? "Analyzing conversation..."
      : "Creating planner...";

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <Dialog.Content maxWidth="500px" className={styles.dialogContent}>
        <Flex
          direction="column"
          gap="3"
          {...dialogNonInteractiveCloseHandlers(() => handleOpenChange(false))}
        >
          <Dialog.Title>
            <Flex align="center" gap="2">
              <Text>{title}</Text>
              <Badge color="blue">task_planner</Badge>
            </Flex>
          </Dialog.Title>

          <Dialog.Description>{description}</Dialog.Description>

          {error && (
            <Callout type="error" preventClose className={styles.callout}>
              {error}
            </Callout>
          )}

          {isLoading && (
            <Flex
              align="center"
              justify="center"
              gap="2"
              className={styles.loadingContainer}
            >
              <Icon
                icon={LoaderCircle}
                size="md"
                tone="accent"
                className={styles.spinnerIcon}
              />
              <Text color="gray" role="status" aria-live="polite">
                {loadingLabel}
              </Text>
            </Flex>
          )}

          <Flex gap="3" mt="4" justify="end">
            <Dialog.Close asChild>
              <Button variant="soft" color="gray" disabled={isLoading}>
                Cancel
              </Button>
            </Dialog.Close>
            <Button onClick={() => void handleApply()} disabled={isLoading}>
              {isLoading ? loadingLabel : buttonLabel}
            </Button>
          </Flex>
        </Flex>
      </Dialog.Content>
    </Dialog>
  );
};

TaskPlannerDialog.displayName = "TaskPlannerDialog";
