import React, {
  useCallback,
  useId,
  useState,
  useRef,
  useLayoutEffect,
} from "react";
import classNames from "classnames";
import { ExternalLink, Pencil, X } from "lucide-react";
import type { PushMode, QueuedItem } from "../../features/Chat";
import { useChatActions } from "../../hooks/useChatActions";
import { useThreadId } from "../../features/Chat/Thread";
import { setInputValue } from "../ChatForm/actions";
import { Button, Icon, IconButton, SegmentedControl, Select } from "../ui";
import { describeQueuedItem, queueModeOptions } from "./queuePresentation";
import { revealProcessOutput } from "./revealProcessOutput";
import styles from "./QueuePanel.module.css";

export type QueueRowProps = {
  queuedItem: QueuedItem;
  position: number;
  expanded?: boolean;
  onToggleExpanded?: () => void;
};

const UPDATE_ERROR =
  "Could not update this queued item. Try the control again.";
const TIMING_ERROR = "Could not change delivery timing. Try choosing it again.";

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

export const QueueRow: React.FC<QueueRowProps> = ({
  queuedItem,
  position,
  expanded,
  onToggleExpanded,
}) => {
  const chatId = useThreadId();
  const { cancelQueued, setQueuedPriority, updatePendingDelivery } =
    useChatActions(chatId);
  const [isWorking, setIsWorking] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [localExpanded, setIsExpanded] = useState(false);
  const isExpanded = expanded ?? localExpanded;
  const rowRef = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const row = rowRef.current;
    if (isExpanded && row && typeof row.scrollIntoView === "function") {
      row.scrollIntoView({ block: "nearest" });
    }
  }, [isExpanded]);
  const detailId = useId();

  const presentation = describeQueuedItem(queuedItem);
  const content = queuedItem.content ?? "";
  const isEditable = presentation.isUserMessage && content.length > 0;
  const options = queueModeOptions(presentation.isUserMessage);
  const active =
    options.find((option) => option.value === presentation.push) ?? options[0];

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
      setActionError(UPDATE_ERROR);
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
      setActionError(UPDATE_ERROR);
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

  const handleModeChange = useCallback(
    async (next: PushMode) => {
      if (isWorking || next === presentation.push) return;
      setIsWorking(true);
      setActionError(null);
      try {
        if (presentation.isUserMessage) {
          const ok = await setQueuedPriority(
            queuedItem.client_request_id,
            next === "preempt",
          );
          if (!ok) throw new Error("Priority failed");
          return;
        }
        await updatePendingDelivery(queuedItem.client_request_id, {
          push: next,
        });
      } catch {
        setActionError(TIMING_ERROR);
        return;
      } finally {
        setIsWorking(false);
      }
    },
    [
      isWorking,
      presentation.push,
      presentation.isUserMessage,
      setQueuedPriority,
      updatePendingDelivery,
      queuedItem.client_request_id,
    ],
  );

  const cancelLabel = presentation.isDelivery
    ? `Cancel pending delivery: ${presentation.title}`
    : "Cancel queued message";
  const modeLabel = `Delivery timing for ${presentation.title}`;

  return (
    <div
      className={classNames(styles.row, "rf-enter-rise")}
      ref={rowRef}
      data-testid="queued-item"
      data-delivery-id={queuedItem.client_request_id}
      data-push={presentation.push}
      data-expanded={isExpanded}
      aria-busy={isWorking}
    >
      <div className={styles.rowHeader}>
        <button
          type="button"
          className={classNames(styles.rowSurface, "rf-pressable")}
          disabled={isWorking}
          aria-controls={detailId}
          aria-expanded={isExpanded}
          onClick={() =>
            onToggleExpanded
              ? onToggleExpanded()
              : setIsExpanded((prev) => !prev)
          }
        >
          <span className={styles.position}>{position}</span>
          <Icon
            className={styles.toneIcon}
            icon={presentation.icon}
            size="sm"
            tone={presentation.tone}
          />
          <span className={styles.title}>{presentation.title}</span>
          {presentation.source && (
            <span className={styles.source}>{presentation.source}</span>
          )}
          <span className={styles.preview}>{presentation.preview}</span>
        </button>
        <div className={styles.rowActions}>
          {isEditable && (
            <IconButton
              aria-label="Click to edit queued message"
              disabled={isWorking}
              icon={Pencil}
              size="sm"
              variant="plain"
              onClick={() => void handleEdit()}
            />
          )}
          <Select
            disabled={isWorking}
            value={presentation.push}
            onValueChange={(next) => void handleModeChange(next as PushMode)}
          >
            <Select.Trigger
              aria-busy={isWorking}
              aria-label={`${modeLabel}: ${active.label}`}
              className={styles.chip}
              data-tone={active.tone}
              title={active.label}
            >
              <Icon icon={active.icon} size="sm" />
              <span className={styles.chipLabel}>{active.shortLabel}</span>
            </Select.Trigger>
            <Select.Content>
              {options.map((option) => (
                <Select.Item key={option.value} value={option.value}>
                  <span className={styles.optionLabel}>
                    <span>{option.label}</span>
                    <span className={styles.optionDescription}>
                      {option.description}
                    </span>
                  </span>
                </Select.Item>
              ))}
            </Select.Content>
          </Select>
          <IconButton
            aria-label={cancelLabel}
            disabled={isWorking}
            icon={X}
            size="sm"
            variant="plain"
            onClick={() => void handleCancel()}
          />
        </div>
      </div>
      {isExpanded && (
        <div
          className={classNames(styles.detail, "rf-enter-rise")}
          id={detailId}
        >
          {isEditable ? (
            <button
              type="button"
              aria-label="Edit queued message text"
              className={classNames(styles.detailText, styles.detailEditable)}
              disabled={isWorking}
              onClick={() => void handleEdit()}
            >
              {content}
            </button>
          ) : (
            <p className={styles.detailText}>
              {content || presentation.preview}
            </p>
          )}
          <div className={styles.detailLine}>
            <SegmentedControl
              aria-busy={isWorking}
              aria-label={modeLabel}
              className={styles.detailSegments}
              data-push={presentation.push}
              size="sm"
              options={options.map((option) => ({
                value: option.value,
                label:
                  option.value === "preempt" && !presentation.isUserMessage
                    ? "Interrupt"
                    : option.shortLabel,
                ariaLabel: option.label,
                disabled: isWorking,
              }))}
              value={presentation.push}
              onValueChange={(next) => void handleModeChange(next as PushMode)}
            />
            <p className={styles.detailHint}>{active.description}</p>
            <div className={styles.detailActions}>
              {presentation.processId !== null && (
                <Button
                  disabled={isWorking}
                  leftIcon={ExternalLink}
                  size="sm"
                  variant="plain"
                  onClick={() =>
                    revealProcessOutput(presentation.processId ?? "")
                  }
                >
                  Open output
                </Button>
              )}
              <Button
                className={styles.detailCancel}
                disabled={isWorking}
                leftIcon={X}
                size="sm"
                variant="plain"
                onClick={() => void handleCancel()}
              >
                Cancel delivery
              </Button>
            </div>
          </div>
        </div>
      )}
      {actionError && (
        <p role="alert" className={styles.error}>
          {actionError}
        </p>
      )}
    </div>
  );
};

export default QueueRow;
