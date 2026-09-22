import React, { useEffect, useId, useRef, useState } from "react";
import { useThreadId } from "../../features/Chat/Thread";
import { ChevronDown, ChevronUp, Inbox } from "lucide-react";
import type { QueuedItem } from "../../features/Chat/Thread/types";
import { Badge, Icon, IconButton } from "../ui";
import { QueueRow } from "./QueueRow";
import { queueStatusText } from "./queuePresentation";
import { useAppSelector } from "../../hooks/useAppSelector";
import { selectWaitingInterruptibleById } from "../../features/Chat/Thread/selectors";
import styles from "./QueuePanel.module.css";

type QueuePanelProps = {
  queuedItems: QueuedItem[];
  isBusy: boolean;
};

/** Rows visible before the list scrolls internally. */
const VISIBLE_ROWS = 4;

/**
 * The pending delivery queue: what the agent will receive next, in the order
 * the engine will deliver it. The visual order always mirrors the wire order;
 * when-idle items stay in place and are labeled instead of being moved.
 */
const ThreadQueuePanel: React.FC<QueuePanelProps & { chatId: string }> = ({
  queuedItems,
  isBusy,
  chatId,
}) => {
  const waitingInterruptible = useAppSelector((state) =>
    selectWaitingInterruptibleById(state, chatId),
  );
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const hasExpandedRow = queuedItems.some(
    (item) => item.client_request_id === expandedId,
  );
  const storageKey = `refact:queue-open:${chatId}`;
  const [isOpen, setIsOpen] = useState(() => {
    try {
      return sessionStorage.getItem(storageKey) !== "false";
    } catch {
      return true;
    }
  });
  const toggleOpen = () => setIsOpen((open) => !open);
  useEffect(() => {
    try {
      sessionStorage.setItem(storageKey, String(isOpen));
    } catch {
      /* Storage may be unavailable in embedded hosts. */
    }
  }, [isOpen, storageKey]);
  const listRef = useRef<HTMLDivElement>(null);
  const [hasOverflow, setHasOverflow] = useState(false);
  useEffect(() => {
    const list = listRef.current;
    if (!list) return;
    const measure = () =>
      setHasOverflow(
        list.scrollHeight > list.clientHeight + list.scrollTop + 1,
      );
    const observer = new ResizeObserver(measure);
    observer.observe(list);
    Array.from(list.children).forEach((row) => observer.observe(row));
    list.addEventListener("scroll", measure);
    measure();
    return () => {
      observer.disconnect();
      list.removeEventListener("scroll", measure);
    };
  }, [isOpen, queuedItems]);
  const listId = useId();

  if (queuedItems.length === 0) return null;

  return (
    <div className={styles.panel}>
      <div className={styles.header}>
        <Icon icon={Inbox} size="sm" tone="muted" />
        <span className={styles.headerTitle}>Delivery queue</span>
        <Badge className={styles.headerCount} size="xs" tone="accent">
          {queuedItems.length}
        </Badge>
        <span
          className={styles.headerStatus}
          role="status"
          aria-live="polite"
          data-testid="queue-header"
        >
          {queueStatusText(queuedItems, isBusy, waitingInterruptible)}
        </span>
        <IconButton
          aria-controls={listId}
          aria-expanded={isOpen}
          aria-label={
            isOpen
              ? "Collapse the delivery queue"
              : `Expand the delivery queue, ${queuedItems.length} items`
          }
          icon={isOpen ? ChevronUp : ChevronDown}
          size="sm"
          variant="plain"
          onClick={toggleOpen}
        />
      </div>
      {isOpen && (
        <div
          className={styles.list}
          id={listId}
          data-testid="queue-list"
          ref={listRef}
          data-overflow={hasOverflow}
          data-visible-rows={VISIBLE_ROWS}
          data-expanded-row={hasExpandedRow}
        >
          {queuedItems.map((item, index) => (
            <QueueRow
              key={item.client_request_id}
              queuedItem={item}
              position={index + 1}
              expanded={expandedId === item.client_request_id}
              onToggleExpanded={() =>
                setExpandedId((current) =>
                  current === item.client_request_id
                    ? null
                    : item.client_request_id,
                )
              }
            />
          ))}
        </div>
      )}
    </div>
  );
};

// The transcript store resets on thread switches; session storage also survives
// switching away and back, while keeping queue preferences scoped to a thread.
export const QueuePanel: React.FC<QueuePanelProps> = (props) => {
  const chatId = useThreadId();
  return <ThreadQueuePanel key={chatId} chatId={chatId} {...props} />;
};
