import React, { useCallback, useState } from "react";
import { Flex, Text, IconButton, Card, Badge, Tooltip } from "@radix-ui/themes";
import {
  Cross1Icon,
  ClockIcon,
  LightningBoltIcon,
} from "@radix-ui/react-icons";
import type { QueuedItem } from "../../features/Chat";
import { useChatActions } from "../../hooks";
import { useAppSelector } from "../../hooks";
import { selectConfig, selectApiKey } from "../../features/Config/configSlice";
import { selectChatId } from "../../features/Chat/Thread/selectors";
import { sendUserMessage } from "../../services/refact/chatCommands";
import { setInputValue } from "../ChatForm/actions";
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
      // ignore cancel errors
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
      // ignore edit errors
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
      // ignore toggle errors
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

  return (
    <Tooltip content={tooltipContent} side="left" delayDuration={400}>
      <Card
        className={classNames(styles.queuedMessage, {
          [styles.queuedMessagePriority]: queuedItem.priority,
        })}
      >
        <Flex gap="2" align="center" justify="between">
          <Flex gap="2" align="center" style={{ flex: 1, minWidth: 0 }}>
            <Badge
              color={queuedItem.priority ? "blue" : "amber"}
              variant="soft"
              size="1"
            >
              {queuedItem.priority ? (
                <LightningBoltIcon width={12} height={12} />
              ) : (
                <ClockIcon width={12} height={12} />
              )}
              {position}
            </Badge>
            <Text
              size="2"
              color="gray"
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
                size="1"
                variant="ghost"
                color={queuedItem.priority ? "amber" : "blue"}
                disabled={isWorking}
                onClick={() => void handleTogglePriority()}
                title={
                  queuedItem.priority
                    ? "Change to normal queue"
                    : "Change to send next"
                }
              >
                {queuedItem.priority ? (
                  <ClockIcon width={14} height={14} />
                ) : (
                  <LightningBoltIcon width={14} height={14} />
                )}
              </IconButton>
            )}
            <IconButton
              size="1"
              variant="ghost"
              color="gray"
              disabled={isWorking}
              onClick={() => void handleCancel()}
              title="Cancel queued message"
            >
              <Cross1Icon width={14} height={14} />
            </IconButton>
          </Flex>
        </Flex>
      </Card>
    </Tooltip>
  );
};

export default QueuedMessage;
