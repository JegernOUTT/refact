import React, { useEffect, useState } from "react";
import { useThreadId } from "../../features/Chat/Thread";
import { Inbox } from "lucide-react";
import type { QueuedItem } from "../../features/Chat/Thread/types";
import { Icon } from "../ui";
import { QueueRow } from "./QueueRow";
import { queueStatusText } from "./queuePresentation";
import { useAppSelector } from "../../hooks/useAppSelector";
import {
  selectRecoveryById,
  selectWaitingInterruptibleById,
} from "../../features/Chat/Thread/selectors";
import { TranscriptItem } from "./TranscriptItem/TranscriptItem";
import styles from "./QueuePanel.module.css";

type QueuePanelProps = {
  queuedItems: QueuedItem[];
  isBusy: boolean;
};

/** Rows visible before the list scrolls internally. */
const VISIBLE_ROWS = 4;
const QUEUE_SECTION_LABELS = { output: "" };

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
  useEffect(() => {
    try {
      sessionStorage.setItem(storageKey, String(isOpen));
    } catch {
      /* Storage may be unavailable in embedded hosts. */
    }
  }, [isOpen, storageKey]);
  const [list, setList] = useState<HTMLDivElement | null>(null);
  const [hasOverflow, setHasOverflow] = useState(false);
  useEffect(() => {
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
  }, [list, queuedItems]);

  if (queuedItems.length === 0) return null;

  const status = queueStatusText(queuedItems, isBusy, waitingInterruptible);
  const count = queuedItems.length;

  return (
    <div className={styles.panel}>
      <TranscriptItem
        icon={<Icon icon={Inbox} size="sm" tone="muted" />}
        verb="Queued"
        target={`${count} ${count === 1 ? "message" : "messages"}`}
        meta={<span data-testid="queue-header">{status}</span>}
        expandable
        open={isOpen}
        onOpenChange={setIsOpen}
        sectionLabels={QUEUE_SECTION_LABELS}
        panelClassName={styles.disclosure}
        output={
          <div
            className={styles.list}
            data-testid="queue-list"
            ref={setList}
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
        }
      />
      <span className={styles.liveStatus} role="status" aria-live="polite">
        {status}
      </span>
    </div>
  );
};

// The transcript store resets on thread switches; session storage also survives
// switching away and back, while keeping queue preferences scoped to a thread.
export const QueuePanel: React.FC<QueuePanelProps> = (props) => {
  const chatId = useThreadId();
  const recovery = useAppSelector((state) => selectRecoveryById(state, chatId));
  const queuedItems =
    recovery && recovery.state !== "succeeded"
      ? props.queuedItems.filter(
          (item) =>
            item.command_type !== "recovery_retry" &&
            item.command_type !== "recovery_retry_now",
        )
      : props.queuedItems;
  return (
    <ThreadQueuePanel
      key={chatId}
      chatId={chatId}
      {...props}
      queuedItems={queuedItems}
    />
  );
};
