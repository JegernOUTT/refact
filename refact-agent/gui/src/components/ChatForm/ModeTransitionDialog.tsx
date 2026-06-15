import React, { useCallback, useState } from "react";
import { Flex, Text, Button, Badge } from "@radix-ui/themes";
import { LoaderCircle } from "lucide-react";
import { Dialog, Icon } from "../ui";
import { Callout } from "../Callout";
import { useApplyModeTransitionMutation } from "../../services/refact/trajectory";
import { trajectoriesApi } from "../../services/refact/trajectories";
import { useCreatePlannerChatFromTransitionMutation } from "../../services/refact/tasks";
import {
  addPlannerChat,
  setTaskActiveChat,
} from "../../features/Tasks/tasksSlice";
import {
  createChatWithId,
  requestSseRefresh,
  closeThread,
  updateChatRuntimeFromSessionState,
} from "../../features/Chat/Thread/actions";
import { selectThreadById } from "../../features/Chat/Thread/selectors";
import { push } from "../../features/Pages/pagesSlice";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { selectConfig, selectApiKey } from "../../features/Config/configSlice";
import { regenerate } from "../../services/refact/chatCommands";
import { dialogNonInteractiveCloseHandlers } from "../../utils/dialogPointerClose";
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
  return "Failed to apply transition";
}

function formatRegenerateError(err: unknown): string {
  return `Failed to start assistant after mode switch: ${extractErrorMessage(
    err,
  )}`;
}

type ModeTransitionDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  chatId: string;
  currentMode: string;
  targetMode: string;
  targetModeTitle: string;
  targetModeDescription: string;
  taskId?: string;
};

type TransitionPhase = "analyzing" | "refreshing" | "opening" | "starting";

const TRANSITION_PHASES: Record<TransitionPhase, { label: string }> = {
  analyzing: { label: "Analyzing conversation..." },
  refreshing: { label: "Updating chat list..." },
  opening: { label: "Opening new chat..." },
  starting: { label: "Starting assistant..." },
};

function waitForNextFrame(): Promise<void> {
  if (
    typeof window === "undefined" ||
    typeof window.requestAnimationFrame !== "function"
  ) {
    return Promise.resolve();
  }
  return new Promise((resolve) => {
    window.requestAnimationFrame(() => resolve());
  });
}

function isSelfSwitch(currentMode: string, targetMode: string): boolean {
  return currentMode === targetMode;
}

export const ModeTransitionDialog: React.FC<ModeTransitionDialogProps> = ({
  open,
  onOpenChange,
  chatId,
  currentMode,
  targetMode,
  targetModeTitle,
  targetModeDescription,
  taskId,
}) => {
  const dispatch = useAppDispatch();
  const config = useAppSelector(selectConfig);
  const apiKey = useAppSelector(selectApiKey);
  const sourceThread = useAppSelector((state) =>
    selectThreadById(state, chatId),
  );
  const sourceWorktree = sourceThread?.worktree;
  const [error, setError] = useState<string | null>(null);
  const [phase, setPhase] = useState<TransitionPhase | null>(null);
  const [activeTransition, setActiveTransition] = useState<{
    currentMode: string;
    targetMode: string;
    targetModeTitle: string;
    isSelf: boolean;
  } | null>(null);

  const [applyMutation, { isLoading: isApplying }] =
    useApplyModeTransitionMutation();
  const [createFromTransition, { isLoading: isTransitioning }] =
    useCreatePlannerChatFromTransitionMutation();

  const isBusy = isApplying || isTransitioning || phase !== null;

  const handleApply = useCallback(async () => {
    if (isBusy) return;

    const transition = {
      currentMode,
      targetMode,
      targetModeTitle,
      isSelf: isSelfSwitch(currentMode, targetMode),
    };
    setActiveTransition(transition);
    setError(null);
    setPhase("analyzing");
    try {
      let newChatId: string;
      if (taskId) {
        const result = await createFromTransition({
          taskId,
          sourceChatId: chatId,
          targetMode,
          targetModeDescription,
        }).unwrap();
        newChatId = result.new_chat_id;
        const now = new Date().toISOString();
        setPhase("opening");
        dispatch(
          createChatWithId({
            id: newChatId,
            mode: targetMode,
            isTaskChat: true,
            taskMeta: {
              task_id: taskId,
              role: "planner",
              planner_chat_id: newChatId,
            },
            parentId: chatId,
            linkType: "mode_transition",
            worktree: sourceWorktree,
          }),
        );
        dispatch(
          addPlannerChat({
            taskId,
            planner: {
              id: newChatId,
              title: "",
              createdAt: now,
              updatedAt: now,
              mode: targetMode,
            },
          }),
        );
        dispatch(
          setTaskActiveChat({
            taskId,
            activeChat: { type: "planner", chatId: newChatId },
          }),
        );
        dispatch(requestSseRefresh({ chatId: newChatId }));
      } else {
        const result = await applyMutation({
          chatId,
          targetMode,
          targetModeDescription,
        }).unwrap();
        newChatId = result.new_chat_id;

        setPhase("refreshing");
        await dispatch(
          trajectoriesApi.endpoints.listAllTrajectories.initiate(undefined, {
            forceRefetch: true,
          }),
        ).unwrap();

        setPhase("opening");
        dispatch(closeThread({ id: chatId, force: true }));
        dispatch(
          createChatWithId({
            id: newChatId,
            mode: targetMode,
            parentId: chatId,
            linkType: "mode_transition",
            worktree: sourceWorktree,
          }),
        );
        dispatch(requestSseRefresh({ chatId: newChatId }));
        dispatch(push({ name: "chat" }));
      }

      await waitForNextFrame();
      setPhase("starting");
      try {
        await regenerate(newChatId, config, apiKey ?? undefined);
      } catch (regenerateError) {
        dispatch(
          updateChatRuntimeFromSessionState({
            id: newChatId,
            session_state: "error",
            error: formatRegenerateError(regenerateError),
          }),
        );
      }
      await waitForNextFrame();
      onOpenChange(false);
    } catch (err) {
      const errorMessage = extractErrorMessage(err);
      setError(errorMessage);
    } finally {
      setPhase(null);
      setActiveTransition(null);
    }
  }, [
    isBusy,
    currentMode,
    chatId,
    targetMode,
    targetModeTitle,
    targetModeDescription,
    taskId,
    applyMutation,
    createFromTransition,
    dispatch,
    onOpenChange,
    config,
    apiKey,
    sourceWorktree,
  ]);

  const handleOpenChange = useCallback(
    (newOpen: boolean) => {
      if (!newOpen && isBusy) {
        return;
      }
      if (!newOpen) {
        setError(null);
        setPhase(null);
        setActiveTransition(null);
      }
      onOpenChange(newOpen);
    },
    [onOpenChange, isBusy],
  );

  const isSelf =
    activeTransition?.isSelf ?? isSelfSwitch(currentMode, targetMode);
  const displayCurrentMode = activeTransition?.currentMode ?? currentMode;
  const displayTargetMode = activeTransition?.targetMode ?? targetMode;
  const displayTargetModeTitle =
    activeTransition?.targetModeTitle ?? targetModeTitle;
  const phaseInfo = phase ? TRANSITION_PHASES[phase] : null;

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
              <Text>{isSelf ? "Restart Mode" : "Switch Mode"}</Text>
              {isSelf ? (
                <Badge color="green">
                  {displayTargetModeTitle || displayTargetMode}
                </Badge>
              ) : (
                <>
                  <Badge color="gray">{displayCurrentMode}</Badge>
                  <Text color="gray">→</Text>
                  <Badge color="blue">
                    {displayTargetModeTitle || displayTargetMode}
                  </Badge>
                </>
              )}
            </Flex>
          </Dialog.Title>

          <Dialog.Description>
            {isSelf
              ? "The assistant will analyze your conversation and create a fresh start with preserved context."
              : "The assistant will analyze your conversation and preserve relevant context for the new mode."}
          </Dialog.Description>

          {error && (
            <Callout type="error" preventClose className={styles.callout}>
              {error}
            </Callout>
          )}

          {phaseInfo && (
            <Flex
              direction="column"
              gap="3"
              className={styles.loadingContainer}
            >
              <Flex align="center" justify="center" gap="2">
                <Icon
                  icon={LoaderCircle}
                  size="md"
                  tone="accent"
                  className={styles.spinnerIcon}
                />
                <Text color="gray" role="status" aria-live="polite">
                  {phaseInfo.label}
                </Text>
              </Flex>
            </Flex>
          )}

          <Flex gap="3" mt="4" justify="end">
            <Dialog.Close asChild>
              <Button variant="soft" color="gray" disabled={isBusy}>
                Cancel
              </Button>
            </Dialog.Close>
            <Button onClick={() => void handleApply()} disabled={isBusy}>
              {isBusy
                ? isSelf
                  ? "Restarting..."
                  : "Switching..."
                : isSelf
                  ? "Restart Mode"
                  : "Switch Mode"}
            </Button>
          </Flex>
        </Flex>
      </Dialog.Content>
    </Dialog>
  );
};

ModeTransitionDialog.displayName = "ModeTransitionDialog";
