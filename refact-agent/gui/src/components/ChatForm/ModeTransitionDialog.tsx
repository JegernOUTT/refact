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
import { useApplyModeTransitionMutation } from "../../services/refact/trajectory";
import { trajectoriesApi } from "../../services/refact/trajectories";
import {
  createChatWithId,
  requestSseRefresh,
  closeThread,
} from "../../features/Chat/Thread/actions";
import { selectThreadById } from "../../features/Chat/Thread/selectors";
import { push } from "../../features/Pages/pagesSlice";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { selectConfig, selectApiKey } from "../../features/Config/configSlice";
import { regenerate } from "../../services/refact/chatCommands";
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

type ModeTransitionDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  chatId: string;
  currentMode: string;
  targetMode: string;
  targetModeTitle: string;
  targetModeDescription: string;
};

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

  const [applyMutation, { isLoading: isApplying }] =
    useApplyModeTransitionMutation();

  const handleApply = useCallback(async () => {
    setError(null);
    try {
      const result = await applyMutation({
        chatId,
        targetMode,
        targetModeDescription,
      }).unwrap();

      onOpenChange(false);

      await dispatch(
        trajectoriesApi.endpoints.listAllTrajectories.initiate(undefined, {
          forceRefetch: true,
        }),
      ).unwrap();

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

      await regenerate(result.new_chat_id, config, apiKey ?? undefined);
    } catch (err) {
      const errorMessage = extractErrorMessage(err);
      setError(errorMessage);
    }
  }, [
    chatId,
    targetMode,
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
      if (!newOpen) {
        setError(null);
      }
      onOpenChange(newOpen);
    },
    [onOpenChange],
  );

  const isSelf = isSelfSwitch(currentMode, targetMode);

  return (
    <Dialog.Root open={open} onOpenChange={handleOpenChange}>
      <Dialog.Content maxWidth="500px" className={styles.dialogContent}>
        <Dialog.Title>
          <Flex align="center" gap="2">
            <Text>{isSelf ? "Restart Mode" : "Switch Mode"}</Text>
            {isSelf ? (
              <Badge color="green">{targetModeTitle || targetMode}</Badge>
            ) : (
              <>
                <Badge color="gray">{currentMode}</Badge>
                <Text color="gray">→</Text>
                <Badge color="blue">{targetModeTitle || targetMode}</Badge>
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

        {isApplying && (
          <Flex
            align="center"
            justify="center"
            gap="2"
            className={styles.loadingContainer}
          >
            <Spinner />
            <Text color="gray">Analyzing conversation...</Text>
          </Flex>
        )}

        <Flex gap="3" mt="4" justify="end">
          <Dialog.Close>
            <Button variant="soft" color="gray" disabled={isApplying}>
              Cancel
            </Button>
          </Dialog.Close>
          <Button onClick={() => void handleApply()} disabled={isApplying}>
            {isApplying ? (
              <>
                <Spinner size="1" />
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
