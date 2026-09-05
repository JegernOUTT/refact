import React, { useCallback, useState } from "react";
import { Flex, Text } from "@radix-ui/themes";
import { Clock, Info, Send, X } from "lucide-react";
import type { PushMode, QueuedItem } from "../../features/Chat";
import { useChatActions } from "../../hooks/useChatActions";
import { useThreadId } from "../../features/Chat/Thread";
import { setInputValue } from "../ChatForm/actions";
import {
  Badge,
  Icon,
  IconButton,
  SegmentedControl,
  Select,
  Tooltip,
} from "../ui";
import {
  PUSH_MODES,
  describeQueuedItem,
  pushModeDescription,
  pushModeLabel,
} from "./queuePresentation";
import styles from "./ChatContent.module.css";
import classNames from "classnames";

type QueuedMessageProps = {
  queuedItem: QueuedItem;
  position: number;
};

function postInputValue(
  chatId: string,
  text: string,
  sendImmediately: boolean,
) {
  window.postMessage(
    setInputValue({ chatId, value: text, send_immediately: sendImmediately }),
    window.location.origin || "*",
  );
}

export const QueuedMessage: React.FC<QueuedMessageProps> = ({
  queuedItem,
  position,
}) => {
  const chatId = useThreadId();
  const { cancelQueued, setQueuedPriority, updatePendingDelivery } =
    useChatActions(chatId);
  const [isWorking, setIsWorking] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);

  const presentation = describeQueuedItem(queuedItem);
  const content = queuedItem.content ?? "";
  const isEditable = presentation.isUserMessage && content.length > 0;

  const handleCancel = useCallback(async () => {
    if (isWorking) return;
    setIsWorking(true);
    setActionError(null);
    try {
      if (presentation.isDelivery) {
        await updatePendingDelivery(queuedItem.client_request_id, {
          cancel: true,
        });
        return;
      }
      const ok = await cancelQueued(queuedItem.client_request_id);
      if (!ok) throw new Error("Cancel failed");
    } catch {
      setActionError(
        "Could not update this queued item. Try the control again.",
      );
      return;
    } finally {
      setIsWorking(false);
    }
  }, [
    isWorking,
    presentation.isDelivery,
    updatePendingDelivery,
    cancelQueued,
    queuedItem.client_request_id,
  ]);

  const handleEdit = useCallback(async () => {
    if (isWorking || !isEditable) return;
    setIsWorking(true);
    setActionError(null);
    try {
      const ok = await cancelQueued(queuedItem.client_request_id);
      if (!ok) throw new Error("Cancel failed");
      postInputValue(chatId, content, queuedItem.priority);
    } catch {
      setActionError(
        "Could not update this queued item. Try the control again.",
      );
      return;
    } finally {
      setIsWorking(false);
    }
  }, [
    isWorking,
    isEditable,
    cancelQueued,
    chatId,
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
    if (isWorking || !presentation.isUserMessage || !chatId) return;
    setIsWorking(true);
    setActionError(null);
    try {
      const ok = await setQueuedPriority(
        queuedItem.client_request_id,
        !queuedItem.priority,
      );
      if (!ok) throw new Error("Priority failed");
    } catch {
      setActionError(
        "Could not update this queued item. Try the control again.",
      );
      return;
    } finally {
      setIsWorking(false);
    }
  }, [
    isWorking,
    chatId,
    presentation.isUserMessage,
    setQueuedPriority,
    queuedItem.client_request_id,
    queuedItem.priority,
  ]);

  const handlePushChange = useCallback(
    async (next: PushMode) => {
      if (isWorking || next === presentation.push) return;
      setIsWorking(true);
      setActionError(null);
      try {
        await updatePendingDelivery(queuedItem.client_request_id, {
          push: next,
        });
      } catch {
        setActionError(
          "Could not change delivery timing. Try choosing it again.",
        );
        return;
      } finally {
        setIsWorking(false);
      }
    },
    [
      isWorking,
      presentation.push,
      updatePendingDelivery,
      queuedItem.client_request_id,
    ],
  );

  const tooltipContent = content || presentation.preview;

  return (
    <div
      className={classNames(styles.queuedMessage, "rf-enter-rise", {
        [styles.queuedMessagePriority]: presentation.push === "preempt",
        [styles.queuedMessageDeferred]: presentation.push === "when_idle",
      })}
      data-testid="queued-item"
      data-delivery-id={queuedItem.client_request_id}
      data-push={presentation.push}
      aria-busy={isWorking}
    >
      <Flex direction="column" gap="2">
        <Flex
          gap="2"
          align="center"
          justify="between"
          className={styles.queueCardHeader}
        >
          <Flex gap="2" align="center" className={styles.queuedMessageMain}>
            <Badge tone={presentation.tone} size="xs">
              <Icon icon={presentation.icon} size="sm" />
              {position}
            </Badge>
            <Flex direction="column" className={styles.queuedMessageMain}>
              <Flex gap="2" align="center" wrap="wrap">
                <Text size="1" weight="medium">
                  {presentation.title}
                </Text>
                {presentation.source && (
                  <Text size="1" className={styles.queuedMessageSource}>
                    {presentation.source}
                  </Text>
                )}
              </Flex>
            </Flex>
          </Flex>
          <Flex
            gap="1"
            align="center"
            flexShrink="0"
            className={styles.queueCardActions}
          >
            {presentation.isUserMessage && (
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
              aria-label={
                presentation.isDelivery
                  ? `Cancel pending delivery: ${presentation.title}`
                  : "Cancel queued message"
              }
              disabled={isWorking}
              icon={X}
              onClick={() => void handleCancel()}
              size="sm"
              variant="plain"
            />
          </Flex>
        </Flex>
        <Tooltip delayDuration={400}>
          <Tooltip.Trigger asChild>
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
              {presentation.preview}
            </Text>
          </Tooltip.Trigger>
          <Tooltip.Content side="left">{tooltipContent}</Tooltip.Content>
        </Tooltip>
        {actionError && (
          <Text role="alert" size="1" className={styles.queueError}>
            {actionError}
          </Text>
        )}
        {presentation.isDelivery && (
          <Flex gap="2" align="center" wrap="wrap">
            <SegmentedControl
              aria-label={`Delivery timing for ${presentation.title}`}
              aria-busy={isWorking}
              className={styles.queuedMessagePushControl}
              size="sm"
              options={PUSH_MODES.map((mode) => ({
                value: mode,
                label: pushModeLabel(mode),
                ariaLabel: pushModeLabel(mode),
              }))}
              value={presentation.push}
              onValueChange={(next) => void handlePushChange(next as PushMode)}
            />
            <div className={styles.queuedMessagePushSelect}>
              <Select
                value={presentation.push}
                onValueChange={(next) =>
                  void handlePushChange(next as PushMode)
                }
              >
                <Select.Trigger
                  aria-label={`Delivery timing for ${presentation.title}`}
                  aria-busy={isWorking}
                >
                  {presentation.push === "append"
                    ? "After step"
                    : pushModeLabel(presentation.push)}
                </Select.Trigger>
                <Select.Content>
                  {PUSH_MODES.map((mode) => (
                    <Select.Item key={mode} value={mode}>
                      {pushModeLabel(mode)}
                    </Select.Item>
                  ))}
                </Select.Content>
              </Select>
            </div>
            <Tooltip>
              <Tooltip.Trigger asChild>
                <IconButton
                  icon={Info}
                  size="sm"
                  variant="plain"
                  aria-label={pushModeDescription(presentation.push)}
                />
              </Tooltip.Trigger>
              <Tooltip.Content>
                {pushModeDescription(presentation.push)}
              </Tooltip.Content>
            </Tooltip>
          </Flex>
        )}
      </Flex>
    </div>
  );
};

export default QueuedMessage;
