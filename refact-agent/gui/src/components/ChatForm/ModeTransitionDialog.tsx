import React, { useCallback, useRef, useState } from "react";
import { LoaderCircle } from "lucide-react";
import {
  KitButton,
  KitDialog,
  KitDialogCancel,
  KitKeyValue,
  KitStaticRow,
} from "../ComposerKit";
import { useApplyModeTransitionMutation } from "../../services/refact/trajectory";
import { trajectoriesApi } from "../../services/refact/trajectories";
import {
  createChatWithId,
  requestSseRefresh,
  closeThread,
  updateChatRuntimeFromSessionState,
} from "../../features/Chat/Thread/actions";
import {
  selectIsStreamingById,
  selectIsWaitingById,
  selectPendingQuestionSetsById,
  selectThreadById,
  selectThreadPauseById,
} from "../../features/Chat/Thread/selectors";
import { push } from "../../features/Pages/pagesSlice";
import { useAppDispatch } from "../../hooks/useAppDispatch";
import { useAppSelector } from "../../hooks/useAppSelector";
import { selectConfig, selectApiKey } from "../../features/Config/configSlice";
import { regenerate } from "../../services/refact/chatCommands";
import { dialogNonInteractiveCloseHandlers } from "../../utils/dialogPointerClose";
import { humanizeIdentifier } from "../../utils/displayNames";
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
  currentModeTitle?: string;
  targetMode: string;
  targetModeTitle: string;
  targetModeDescription: string;
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
  currentModeTitle,
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
  const isStreaming = useAppSelector((state) =>
    selectIsStreamingById(state, chatId),
  );
  const isWaiting = useAppSelector((state) =>
    selectIsWaitingById(state, chatId),
  );
  const isPaused = useAppSelector((state) =>
    selectThreadPauseById(state, chatId),
  );
  const hasPendingQuestions = useAppSelector((state) =>
    selectPendingQuestionSetsById(state, chatId).some(
      (set) => set.status === "pending",
    ),
  );
  const keepSourceThreadRef = useRef(false);
  keepSourceThreadRef.current = isWaiting || isPaused || hasPendingQuestions;
  const [error, setError] = useState<string | null>(null);
  const [phase, setPhase] = useState<TransitionPhase | null>(null);
  const [activeTransition, setActiveTransition] = useState<{
    currentMode: string;
    currentModeTitle?: string;
    targetMode: string;
    targetModeTitle: string;
    isSelf: boolean;
  } | null>(null);

  const [applyMutation, { isLoading: isApplying }] =
    useApplyModeTransitionMutation();
  const isBusy = isApplying || phase !== null;

  const handleApply = useCallback(async () => {
    if (isBusy || isStreaming) return;
    const keepSourceAtStart = keepSourceThreadRef.current;

    const transition = {
      currentMode,
      currentModeTitle,
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
      const newChatId = result.new_chat_id;

      setPhase("refreshing");
      await dispatch(
        trajectoriesApi.endpoints.listAllTrajectories.initiate(undefined, {
          forceRefetch: true,
        }),
      ).unwrap();

      setPhase("opening");
      if (!keepSourceAtStart && !keepSourceThreadRef.current) {
        dispatch(closeThread({ id: chatId, force: true }));
      }
      dispatch(
        createChatWithId({
          id: newChatId,
          mode: targetMode,
          parentId: chatId,
          linkType: "mode_transition",
          rootChatId: result.root_chat_id ?? undefined,
          worktree: sourceWorktree,
        }),
      );
      dispatch(requestSseRefresh({ chatId: newChatId }));
      dispatch(push({ name: "chat" }));

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
    isStreaming,
    currentMode,
    currentModeTitle,
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
  const displayCurrentModeTitle =
    (activeTransition ? activeTransition.currentModeTitle : currentModeTitle) ??
    humanizeIdentifier(displayCurrentMode);
  const displayTargetMode = activeTransition?.targetMode ?? targetMode;
  const displayTargetModeTitle =
    activeTransition?.targetModeTitle ?? targetModeTitle;
  const phaseInfo = phase ? TRANSITION_PHASES[phase] : null;

  const actionLabel = isSelf ? "Restart Mode" : "Switch Mode";
  const targetLabel = displayTargetModeTitle || displayTargetMode;

  return (
    <KitDialog
      open={open}
      onOpenChange={handleOpenChange}
      title={actionLabel}
      description={
        isSelf
          ? "The assistant will analyze your conversation and create a fresh start with preserved context."
          : "The assistant will analyze your conversation and preserve relevant context for the new mode."
      }
      width="480px"
      bodyProps={dialogNonInteractiveCloseHandlers(() =>
        handleOpenChange(false),
      )}
      footer={
        <>
          <KitDialogCancel disabled={isBusy} />
          <KitButton
            variant="primary"
            onClick={() => void handleApply()}
            disabled={isBusy || isStreaming}
          >
            {isBusy ? (isSelf ? "Restarting..." : "Switching...") : actionLabel}
          </KitButton>
        </>
      }
    >
      {isSelf ? (
        <KitKeyValue label="Mode" value={targetLabel} />
      ) : (
        <>
          <KitKeyValue label="From" value={displayCurrentModeTitle} />
          <KitKeyValue label="To" value={targetLabel} />
        </>
      )}
      {error ? (
        <p className={styles.error} role="alert">
          {error}
        </p>
      ) : null}
      {phaseInfo ? (
        <KitStaticRow
          tone="muted"
          icon={<LoaderCircle aria-hidden="true" className={styles.spinner} />}
          label={
            <span role="status" aria-live="polite">
              {phaseInfo.label}
            </span>
          }
        />
      ) : null}
    </KitDialog>
  );
};

ModeTransitionDialog.displayName = "ModeTransitionDialog";
