import React, { useCallback, useState } from "react";
import { Dialog, Flex, Text, Button, Callout, Badge } from "@radix-ui/themes";
import { Spinner } from "../ui";
import { ExclamationTriangleIcon } from "@radix-ui/react-icons";
import { useApplyModeTransitionMutation } from "../../services/refact/trajectory";
import { trajectoriesApi } from "../../services/refact/trajectories";
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
};

type TransitionPhase = "analyzing" | "refreshing" | "opening" | "starting";

const TRANSITION_PHASES: Record<
  TransitionPhase,
  { label: string; progress: number }
> = {
  analyzing: { label: "Analyzing conversation...", progress: 30 },
  refreshing: { label: "Updating chat list...", progress: 55 },
  opening: { label: "Opening new chat...", progress: 75 },
  starting: { label: "Starting assistant...", progress: 92 },
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

  const isBusy = isApplying || phase !== null;

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
      const result = await applyMutation({
        chatId,
        targetMode,
        targetModeDescription,
      }).unwrap();

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
          id: result.new_chat_id,
          mode: targetMode,
          parentId: chatId,
          linkType: "mode_transition",
          worktree: sourceWorktree,
        }),
      );
      dispatch(requestSseRefresh({ chatId: result.new_chat_id }));
      dispatch(push({ name: "chat" }));

      await waitForNextFrame();
      setPhase("starting");
      try {
        await regenerate(result.new_chat_id, config, apiKey ?? undefined);
      } catch (regenerateError) {
        dispatch(
          updateChatRuntimeFromSessionState({
            id: result.new_chat_id,
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
    applyMutation,
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
    <Dialog.Root open={open} onOpenChange={handleOpenChange}>
      <Dialog.Content
        maxWidth="500px"
        className={styles.dialogContent}
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

        <Dialog.Description size="2" color="gray">
          {isSelf
            ? "The assistant will analyze your conversation and create a fresh start with preserved context."
            : "The assistant will analyze your conversation and preserve relevant context for the new mode."}
        </Dialog.Description>

        {error && (
          <Callout.Root color="red" className={styles.callout}>
            <Callout.Icon>
              <ExclamationTriangleIcon />
            </Callout.Icon>
            <Callout.Text>{error}</Callout.Text>
          </Callout.Root>
        )}

        {phaseInfo && (
          <Flex direction="column" gap="3" className={styles.loadingContainer}>
            <Flex align="center" justify="center" gap="2">
              <Spinner label="Processing" />
              <Text color="gray" role="status" aria-live="polite">
                {phaseInfo.label}
              </Text>
            </Flex>
            <div
              className={styles.progressTrack}
              role="progressbar"
              aria-label={isSelf ? "Restart progress" : "Switch progress"}
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={phaseInfo.progress}
            >
              <div
                className={styles.progressFill}
                style={{ width: `${phaseInfo.progress}%` }}
              />
            </div>
          </Flex>
        )}

        <Flex gap="3" mt="4" justify="end">
          <Dialog.Close>
            <Button variant="soft" color="gray" disabled={isBusy}>
              Cancel
            </Button>
          </Dialog.Close>
          <Button onClick={() => void handleApply()} disabled={isBusy}>
            {isBusy ? (
              <>
                <Spinner size="sm" />
                {isSelf ? "Restarting..." : "Switching..."}
              </>
            ) : isSelf ? (
              "Restart Mode"
            ) : (
              "Switch Mode"
            )}
          </Button>
        </Flex>
      </Dialog.Content>
    </Dialog.Root>
  );
};

ModeTransitionDialog.displayName = "ModeTransitionDialog";
