import React, { useCallback, useState } from "react";
import { Flex, Text } from "@radix-ui/themes";
import { Clock, Send, X } from "lucide-react";
import type { QueuedItem } from "../../features/Chat";
import { useChatActions } from "../../hooks";
import { useAppSelector } from "../../hooks";
import { selectConfig, selectApiKey } from "../../features/Config/configSlice";
import { selectChatId } from "../../features/Chat/Thread/selectors";
import { sendUserMessage } from "../../services/refact/chatCommands";
import { setInputValue } from "../ChatForm/actions";
import { Badge, Icon, IconButton, Tooltip } from "../ui";
import styles from "./ChatContent.module.css";
import classNames from "classnames";

type QueuedMessageProps = {
  queuedItem: QueuedItem;
  position: number;
};

function postInputValue(text: string, sendImmediately: boolean) {
  window.postMessage(
    setInputValue({ value: text, send_immediately: sendImmediately }),
    window.location.origin || "*",
  );
}

export const QueuedMessage: React.FC<QueuedMessageProps> = ({
  queuedItem,
  position,
}) => {
  const { cancelQueued } = useChatActions();
  const config = useAppSelector(selectConfig);
  const apiKey = useAppSelector(selectApiKey);
  const chatId = useAppSelector(selectChatId);
  const [isWorking, setIsWorking] = useState(false);

  const content = queuedItem.content ?? "";
  const isEditable =
    queuedItem.command_type === "user_message" && content.length > 0;

  const handleCancel = useCallback(async () => {
    if (isWorking) return;
    setIsWorking(true);
    try {
      await cancelQueued(queuedItem.client_request_id);
    } catch {
      return;
    } finally {
      setIsWorking(false);
    }
  }, [isWorking, cancelQueued, queuedItem.client_request_id]);

  const handleEdit = useCallback(async () => {
    if (isWorking || !isEditable) return;
    setIsWorking(true);
    try {
      const ok = await cancelQueued(queuedItem.client_request_id);
      if (!ok) return;
      postInputValue(content, queuedItem.priority);
    } catch {
      return;
    } finally {
      setIsWorking(false);
    }
  }, [
    isWorking,
    isEditable,
    cancelQueued,
    queuedItem.client_request_id,
    queuedItem.priority,
    content,
  ]);

  const handleEditKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        void handleEdit();
      }
    },
    [handleEdit],
  );

  const handleTogglePriority = useCallback(async () => {
    if (isWorking || !isEditable || !chatId) return;
    setIsWorking(true);
    try {
      const ok = await cancelQueued(queuedItem.client_request_id);
      if (!ok) return;
      try {
        await sendUserMessage(
          chatId,
          content,
          config,
          apiKey ?? undefined,
          !queuedItem.priority,
        );
      } catch {
        postInputValue(content, queuedItem.priority);
      }
    } catch {
      return;
    } finally {
      setIsWorking(false);
    }
  }, [
    isWorking,
    isEditable,
    chatId,
    config,
    apiKey,
    cancelQueued,
    queuedItem.client_request_id,
    queuedItem.priority,
    content,
  ]);

  const tooltipContent = content || queuedItem.preview;
  const PriorityIcon = queuedItem.priority ? Send : Clock;

  return (
    <Tooltip delayDuration={400}>
      <Tooltip.Trigger asChild>
        <div
          className={classNames(styles.queuedMessage, "rf-enter-rise", {
            [styles.queuedMessagePriority]: queuedItem.priority,
          })}
        >
          <Flex gap="2" align="center" justify="between">
            <Flex gap="2" align="center" className={styles.queuedMessageMain}>
              <Badge tone={queuedItem.priority ? "accent" : "warning"}>
                <Icon icon={PriorityIcon} size="sm" />
                {position}
              </Badge>
              <Text
                size="2"
                className={classNames(styles.queuedMessageText, {
                  [styles.queuedMessageEditable]: isEditable && !isWorking,
                })}
                role={isEditable ? "button" : undefined}
                tabIndex={isEditable ? 0 : undefined}
                aria-label={
                  isEditable ? "Click to edit queued message" : undefined
                }
                aria-disabled={isWorking || undefined}
                onClick={isEditable ? () => void handleEdit() : undefined}
                onKeyDown={isEditable ? handleEditKeyDown : undefined}
              >
                {queuedItem.preview || `[${queuedItem.command_type}]`}
              </Text>
            </Flex>
            <Flex gap="1" align="center" flexShrink="0">
              {isEditable && (
                <IconButton
                  aria-label={
                    queuedItem.priority
                      ? "Change to normal queue"
                      : "Change to send next"
                  }
                  disabled={isWorking}
                  icon={queuedItem.priority ? Clock : Send}
                  onClick={() => void handleTogglePriority()}
                  size="sm"
                  variant="plain"
                />
              )}
              <IconButton
                aria-label="Cancel queued message"
                disabled={isWorking}
                icon={X}
                onClick={() => void handleCancel()}
                size="sm"
                variant="plain"
              />
            </Flex>
          </Flex>
        </div>
      </Tooltip.Trigger>
      <Tooltip.Content side="left">{tooltipContent}</Tooltip.Content>
    </Tooltip>
  );
};

export default QueuedMessage;
