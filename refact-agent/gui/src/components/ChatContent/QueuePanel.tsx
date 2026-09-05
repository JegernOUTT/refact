import React, { useId, useState } from "react";
import { Container, Flex, Text } from "@radix-ui/themes";
import type { QueuedItem } from "../../features/Chat/Thread/types";
import { QueuedMessage } from "./QueuedMessage";
import { IconButton } from "../ui";
import { ChevronDown, ChevronUp } from "lucide-react";
import { queuedItemPushMode } from "./queuePresentation";
import styles from "./ChatContent.module.css";

type QueuePanelProps = {
  queuedItems: QueuedItem[];
  isBusy: boolean;
};

function statusText(queuedItems: QueuedItem[], isBusy: boolean): string {
  const count = (push: string) =>
    queuedItems.filter((item) => queuedItemPushMode(item) === push).length;
  return [
    count("preempt") ? `${count("preempt")} interrupting now` : "",
    count("append")
      ? `${count("append")} ${
          isBusy ? "after current step" : "ready to deliver"
        }`
      : "",
    count("when_idle") ? `${count("when_idle")} waiting for idle` : "",
  ]
    .filter(Boolean)
    .join(" · ");
}

/**
 * The pending delivery queue: what the agent will receive next, in the order
 * the engine will deliver it. The visual order always mirrors the wire order;
 * when-idle items stay in place and are labeled instead of being moved.
 */
export const QueuePanel: React.FC<QueuePanelProps> = ({
  queuedItems,
  isBusy,
}) => {
  const [expanded, setExpanded] = useState(false);
  const listId = useId();
  const displayedItems = expanded ? queuedItems : queuedItems.slice(0, 1);
  if (queuedItems.length === 0) return null;

  return (
    <Container className={styles.queuedMessagesContent}>
      <Flex direction="column" gap="2" align="stretch">
        <Flex
          align="center"
          gap="2"
          justify="end"
          className={styles.queueHeader}
          role="status"
          aria-live="polite"
          data-testid="queue-header"
        >
          <Text size="1" weight="medium">
            Delivery queue
          </Text>
          <Text size="1" className={styles.queuedMessageSource}>
            {statusText(queuedItems, isBusy)}
          </Text>
          {queuedItems.length > 1 && (
            <IconButton
              icon={expanded ? ChevronUp : ChevronDown}
              size="sm"
              variant="plain"
              aria-expanded={expanded}
              aria-controls={listId}
              aria-label={
                expanded
                  ? "Show fewer queued items"
                  : `Show all ${queuedItems.length} queued items`
              }
              onClick={() => setExpanded(!expanded)}
            />
          )}
        </Flex>
        <div className={styles.queueList} id={listId}>
          {displayedItems.map((item, index) => (
            <QueuedMessage
              key={item.client_request_id}
              queuedItem={item}
              position={index + 1}
            />
          ))}
        </div>
      </Flex>
    </Container>
  );
};
