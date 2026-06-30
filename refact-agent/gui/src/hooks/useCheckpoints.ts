import { useCallback, useMemo } from "react";
import type { RestoreMode } from "../features/Checkpoints/Checkpoints";
import { useAppSelector } from "./useAppSelector";
import {
  selectCheckpointsMessageIndex,
  selectIsCheckpointsPopupIsVisible,
  selectIsUndoingCheckpoints,
  selectLatestCheckpointResult,
  selectShouldNewChatBeStarted,
  setCheckpointsErrorLog,
  setIsCheckpointsPopupIsVisible,
  setIsUndoingCheckpoints,
  setLatestCheckpointResult,
  setShouldNewChatBeStarted,
} from "../features/Checkpoints/checkpointsSlice";
import { useAppDispatch } from "./useAppDispatch";
import { useRestoreCheckpoints } from "./useRestoreCheckpoints";
import { Checkpoint, FileChanged } from "../features/Checkpoints/types";
import {
  backUpMessages,
  newChatAction,
  selectMessagesById,
  selectThreadModeById,
  useThreadId,
} from "../features/Chat/Thread";
import { isUserMessage } from "../services/refact";
import { deleteChatById } from "../features/History/historySlice";
import { usePreviewCheckpoints } from "./usePreviewCheckpoints";
import { useEventsBusForIDE } from "./useEventBusForIDE";
import { selectConfig } from "../features/Config/configSlice";

export const useCheckpoints = () => {
  const dispatch = useAppDispatch();
  const chatId = useThreadId();
  const messages = useAppSelector((state) => selectMessagesById(state, chatId));
  const chatMode = useAppSelector((state) =>
    selectThreadModeById(state, chatId),
  );
  const configIdeHost = useAppSelector(selectConfig).host;

  const { setForceReloadFileByPath } = useEventsBusForIDE();

  const { restoreChangesFromCheckpoints, isLoading: isRestoring } =
    useRestoreCheckpoints();
  const { previewChangesFromCheckpoints, isLoading: isPreviewing } =
    usePreviewCheckpoints();
  const isCheckpointsPopupVisible = useAppSelector(
    selectIsCheckpointsPopupIsVisible,
  );
  const isUndoingCheckpoints = useAppSelector(selectIsUndoingCheckpoints);

  const latestRestoredCheckpointsResult = useAppSelector(
    selectLatestCheckpointResult,
  );

  const { reverted_changes, reverted_to, error_log } =
    latestRestoredCheckpointsResult;

  const shouldNewChatBeStarted = useAppSelector(selectShouldNewChatBeStarted);
  const maybeMessageIndex = useAppSelector(selectCheckpointsMessageIndex);

  const allChangedFiles = reverted_changes.reduce<
    (FileChanged & { workspace_folder: string })[]
  >((acc, change) => {
    const filesWithWorkspace = change.files_changed.map((file) => ({
      ...file,
      workspace_folder: change.workspace_folder,
    }));
    return [...acc, ...filesWithWorkspace];
  }, []);

  const wereFilesChanged = useMemo(() => {
    return allChangedFiles.length > 0;
  }, [allChangedFiles]);

  const shouldCheckpointsPopupBeShown = useMemo(() => {
    const targetChatId = latestRestoredCheckpointsResult.chat_id;
    return (
      isCheckpointsPopupVisible &&
      !isUndoingCheckpoints &&
      (!targetChatId || targetChatId === chatId)
    );
  }, [
    chatId,
    isCheckpointsPopupVisible,
    isUndoingCheckpoints,
    latestRestoredCheckpointsResult.chat_id,
  ]);

  const handleUndo = useCallback(() => {
    dispatch(setIsUndoingCheckpoints(true));
  }, [dispatch]);

  const handlePreview = useCallback(
    async (checkpoints: Checkpoint[] | null, messageIndex: number) => {
      if (!checkpoints) return;
      const amountOfUserMessages = messages.filter(isUserMessage);
      const firstUserMessage = amountOfUserMessages[0];
      // Capture chat_id and mode at click time to avoid race conditions
      const currentChatId = chatId;
      const currentChatMode = chatMode;
      try {
        const previewedChanges = await previewChangesFromCheckpoints(
          checkpoints,
          currentChatId,
          currentChatMode,
        ).unwrap();
        const actions = [
          dispatch(setIsUndoingCheckpoints(false)),
          setLatestCheckpointResult({
            ...previewedChanges,
            current_checkpoints: checkpoints,
            messageIndex,
            chat_id: currentChatId,
            chat_mode: currentChatMode,
          }),
          setIsCheckpointsPopupIsVisible(true),
          setShouldNewChatBeStarted(
            messageIndex === messages.indexOf(firstUserMessage),
          ),
        ];
        actions.forEach((action) => dispatch(action));
      } catch {
        dispatch(
          setCheckpointsErrorLog(["Failed to preview checkpoint changes"]),
        );
      }
    },
    [dispatch, previewChangesFromCheckpoints, messages, chatId, chatMode],
  );

  const handleFix = useCallback(
    async (restoreMode: RestoreMode = "files_and_messages") => {
      try {
        // Use chat_id and mode stored at preview time, not current state
        const response = await restoreChangesFromCheckpoints(
          latestRestoredCheckpointsResult.current_checkpoints,
          latestRestoredCheckpointsResult.chat_id,
          latestRestoredCheckpointsResult.chat_mode,
        ).unwrap();
        if (response.success) {
          if (configIdeHost === "jetbrains") {
            const files =
              latestRestoredCheckpointsResult.reverted_changes.flatMap(
                (change) => change.files_changed,
              );
            files.forEach((file) => {
              setForceReloadFileByPath(file.absolute_path);
            });
          }

          dispatch(setIsCheckpointsPopupIsVisible(false));
        } else {
          dispatch(setCheckpointsErrorLog(response.error_log));
          return;
        }

        // Only undo messages if restoreMode is "files_and_messages"
        if (restoreMode === "files_and_messages") {
          if (shouldNewChatBeStarted || !maybeMessageIndex) {
            const actions = [newChatAction(), deleteChatById(chatId)];
            actions.forEach((action) => dispatch(action));
          } else {
            const usefulMessages = messages.slice(0, maybeMessageIndex);
            dispatch(
              backUpMessages({
                id: chatId,
                messages: usefulMessages,
              }),
            );
          }
        }
        // If restoreMode is "files_only", we don't touch the messages
      } catch {
        dispatch(
          setCheckpointsErrorLog(["Failed to restore checkpoint changes"]),
        );
      }
    },
    [
      dispatch,
      setForceReloadFileByPath,
      restoreChangesFromCheckpoints,
      configIdeHost,
      shouldNewChatBeStarted,
      maybeMessageIndex,
      chatId,
      messages,
      latestRestoredCheckpointsResult.current_checkpoints,
      latestRestoredCheckpointsResult.reverted_changes,
      latestRestoredCheckpointsResult.chat_id,
      latestRestoredCheckpointsResult.chat_mode,
    ],
  );

  return {
    shouldCheckpointsPopupBeShown,
    handleUndo,
    handlePreview,
    handleFix,
    isRestoring,
    isPreviewing,
    reverted_changes,
    reverted_to,
    wereFilesChanged,
    allChangedFiles,
    errorLog: error_log,
  };
};
