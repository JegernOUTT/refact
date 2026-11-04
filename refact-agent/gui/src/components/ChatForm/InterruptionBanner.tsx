import React, { useCallback, useState } from "react";
import { Text } from "@radix-ui/themes";
import { CircleAlert, Copy, ExternalLink } from "lucide-react";
import { useAppSelector } from "../../hooks/useAppSelector";
import { useCopyToClipboard } from "../../hooks/useCopyToClipboard";
import { useInternalNavigation } from "../../hooks/useInternalNavigation";
import {
  selectInterruptionById,
  selectIsInterruptedById,
  selectTaskMetaById,
} from "../../features/Chat/Thread/selectors";
import { Button, Icon } from "../ui";
import styles from "./InterruptionBanner.module.css";
import { restartAgentCommand } from "./restartCommand";

export const InterruptionBanner: React.FC<{ chatId: string }> = ({
  chatId,
}) => {
  const interrupted = useAppSelector((state) =>
    selectIsInterruptedById(state, chatId),
  );
  const interruption = useAppSelector((state) =>
    selectInterruptionById(state, chatId),
  );
  const taskMeta = useAppSelector((state) => selectTaskMetaById(state, chatId));
  const copy = useCopyToClipboard();
  const { navigate, pending, error } = useInternalNavigation();
  const [copied, setCopied] = useState(false);

  const ownerChatId =
    interruption?.owner_chat_id ?? taskMeta?.planner_chat_id ?? null;
  const cardId = taskMeta?.card_id ?? null;
  const command = cardId ? restartAgentCommand(cardId) : null;

  const openPlanner = useCallback(() => {
    if (ownerChatId)
      void navigate(`refact://chat/${ownerChatId}`, {
        chatId,
        taskId: taskMeta?.task_id,
      });
  }, [chatId, navigate, ownerChatId, taskMeta?.task_id]);

  const copyCommand = useCallback(() => {
    if (!command) return;
    copy(command);
    setCopied(true);
  }, [command, copy]);

  if (!interrupted) return null;

  return (
    <div
      role="status"
      className={styles.banner}
      data-composer-no-expand="true"
      data-testid="interruption-banner"
    >
      <div className={styles.header}>
        <Icon icon={CircleAlert} size="sm" tone="danger" />
        <Text size="2" weight="medium">
          Interrupted — recover in planner
        </Text>
      </div>
      <Text size="1" className={styles.detail}>
        This agent stopped because its task binding is no longer current
        {interruption?.condition ? ` (${interruption.condition})` : ""}. It will
        not resume from messages sent here; restart it from the owning planner.
      </Text>
      {(interruption?.expected ?? interruption?.actual) && (
        <Text size="1" className={styles.detail}>
          {interruption.expected ? `expected ${interruption.expected}` : ""}
          {interruption.expected && interruption.actual ? ", " : ""}
          {interruption.actual ? `actual ${interruption.actual}` : ""}
        </Text>
      )}
      {command && (
        <code className={styles.command} data-testid="restart-command">
          {command}
        </code>
      )}
      <div className={styles.actions}>
        <Button
          type="button"
          size="sm"
          variant="soft"
          leftIcon={ExternalLink}
          disabled={!ownerChatId || pending}
          onClick={openPlanner}
        >
          Open owning planner
        </Button>
        <Button
          type="button"
          size="sm"
          variant="ghost"
          leftIcon={Copy}
          disabled={!command}
          onClick={copyCommand}
        >
          {copied ? "Copied restart command" : "Copy restart command"}
        </Button>
      </div>
      {error && (
        <Text size="1" role="alert" className={styles.error}>
          {error}
        </Text>
      )}
    </div>
  );
};
