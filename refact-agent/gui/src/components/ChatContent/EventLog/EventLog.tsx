import React, { useEffect, useMemo, useState } from "react";
import { Box, Card, Flex, Text } from "@radix-ui/themes";
import { useAppDispatch } from "../../../hooks/useAppDispatch";
import { openScheduler } from "../../../features/Pages/pagesSlice";
import type { EventMessage } from "../../../services/refact/types";
import {
  getEventMetadata,
  normalizeEventMessageMetadata,
} from "../../../services/refact/types";
import { EventLogEntry } from "./EventLogEntry";
import {
  eventSubkindIconElement,
  eventSubkindLabel,
  eventMessageTone,
} from "./eventSubkind";
import styles from "./EventLog.module.css";

export type EventLogProps = {
  events: EventMessage[];
  threadId: string;
  filterEvents?: EventMessage[];
  onProcessCompletedClick?: (processId: string) => void;
};

function collapsedStorageKey(threadId: string): string {
  return `event-log-collapsed-${threadId}`;
}

function filterStorageKey(threadId: string): string {
  return `event-log-hidden-${threadId}`;
}

function isEventLogMessage(event: EventMessage): boolean {
  return getEventMetadata(event) !== null;
}

function readCollapsed(threadId: string): boolean {
  try {
    if (typeof localStorage === "undefined") return true;
    return localStorage.getItem(collapsedStorageKey(threadId)) !== "false";
  } catch {
    return true;
  }
}

function writeCollapsed(threadId: string, collapsed: boolean): void {
  try {
    if (typeof localStorage === "undefined") return;
    localStorage.setItem(collapsedStorageKey(threadId), String(collapsed));
  } catch {
    return;
  }
}

/**
 * Hidden subkinds are persisted instead of visible ones so subkinds the engine
 * adds later stay visible by default rather than silently disappearing.
 */
function readHiddenSubkinds(threadId: string): string[] {
  try {
    if (typeof localStorage === "undefined") return [];
    const stored = localStorage.getItem(filterStorageKey(threadId));
    if (!stored) return [];
    const parsed = JSON.parse(stored) as unknown;
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((value): value is string => typeof value === "string");
  } catch {
    return [];
  }
}

function writeHiddenSubkinds(threadId: string, hidden: string[]): void {
  try {
    if (typeof localStorage === "undefined") return;
    localStorage.setItem(filterStorageKey(threadId), JSON.stringify(hidden));
  } catch {
    return;
  }
}

function entryKey(event: EventMessage, index: number): string {
  const metadata = getEventMetadata(event);
  return (
    event.message_id ??
    `${metadata?.subkind ?? "event"}-${metadata?.source ?? ""}-${index}`
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function payloadString(event: EventMessage, field: string): string | null {
  if (!isRecord(event.payload)) return null;
  const value = event.payload[field];
  return typeof value === "string" && value.length > 0 ? value : null;
}

export const EventLog: React.FC<EventLogProps> = ({
  events,
  threadId,
  filterEvents: rawFilterEvents = events,
  onProcessCompletedClick,
}) => {
  const dispatch = useAppDispatch();
  const [collapsed, setCollapsed] = useState(() => readCollapsed(threadId));
  const [hiddenSubkinds, setHiddenSubkinds] = useState(() =>
    readHiddenSubkinds(threadId),
  );

  useEffect(() => {
    setCollapsed(readCollapsed(threadId));
    setHiddenSubkinds(readHiddenSubkinds(threadId));
  }, [threadId]);

  const visibleEvents = useMemo(
    () => events.filter(isEventLogMessage).map(normalizeEventMessageMetadata),
    [events],
  );

  const filterEvents = useMemo(
    () =>
      rawFilterEvents
        .filter(isEventLogMessage)
        .map(normalizeEventMessageMetadata),
    [rawFilterEvents],
  );

  const presentSubkinds = useMemo(() => {
    const seen: string[] = [];
    for (const event of filterEvents) {
      if (!seen.includes(event.subkind)) seen.push(event.subkind);
    }
    return seen;
  }, [filterEvents]);

  const hiddenSet = useMemo(
    () => new Set<string>(hiddenSubkinds),
    [hiddenSubkinds],
  );

  const filteredEvents = useMemo(
    () => visibleEvents.filter((event) => !hiddenSet.has(event.subkind)),
    [visibleEvents, hiddenSet],
  );

  if (visibleEvents.length === 0) return null;

  const handleSummaryClick = (event: React.MouseEvent<HTMLElement>) => {
    event.preventDefault();
    setCollapsed((current) => {
      const next = !current;
      writeCollapsed(threadId, next);
      return next;
    });
  };

  const toggleSubkind = (subkind: string) => {
    setHiddenSubkinds((current) => {
      const next = current.includes(subkind)
        ? current.filter((candidate) => candidate !== subkind)
        : [...current, subkind];
      writeHiddenSubkinds(threadId, next);
      return next;
    });
  };

  const handleEventClick = (event: EventMessage): boolean => {
    if (event.subkind === "process_completed") {
      const processId = payloadString(event, "process_id");
      if (processId && onProcessCompletedClick) {
        onProcessCompletedClick(processId);
        return true;
      }
      return false;
    }

    if (event.subkind === "cron_fire") {
      const taskId = payloadString(event, "task_id");
      dispatch(openScheduler(taskId ? { taskId } : undefined));
      return true;
    }

    return false;
  };

  return (
    <Card className={styles.card} data-testid="event-log">
      <details className={styles.details} open={!collapsed}>
        <summary
          className={`${styles.summary} rf-pressable`}
          onClick={handleSummaryClick}
        >
          <Flex
            align="center"
            gap="2"
            wrap="wrap"
            className={styles.summaryRow}
          >
            <Text as="span" size="1" weight="medium" className={styles.title}>
              Event history
            </Text>
            <Text as="span" size="1" className={styles.count}>
              {visibleEvents.length}{" "}
              {visibleEvents.length === 1 ? "event" : "events"}
            </Text>
          </Flex>
        </summary>
        <Box className={`${styles.body} rf-enter-rise`}>
          <Flex gap="1" wrap="wrap" className={styles.filters}>
            {presentSubkinds.map((subkind) => {
              const selected = !hiddenSet.has(subkind);
              const label = eventSubkindLabel(subkind);
              const tone = eventMessageTone(
                filterEvents.find((event) => event.subkind === subkind) ??
                  ({ subkind } as EventMessage),
              );
              return (
                <label
                  key={subkind}
                  className={styles.filterChip}
                  data-selected={selected}
                  data-subkind={subkind}
                >
                  <input
                    type="checkbox"
                    checked={selected}
                    onChange={() => toggleSubkind(subkind)}
                  />
                  <Text as="span" size="1" aria-hidden="true">
                    {eventSubkindIconElement(subkind, tone)}
                  </Text>
                  <Text as="span" size="1">
                    {label}
                  </Text>
                </label>
              );
            })}
          </Flex>
          <Flex direction="column" gap="1">
            {filteredEvents.length > 0 ? (
              filteredEvents.map((event, index) => {
                const key = entryKey(event, index);
                return (
                  <EventLogEntry
                    key={key}
                    event={event}
                    entryId={key}
                    onEventClick={handleEventClick}
                  />
                );
              })
            ) : (
              <Text size="1" color="gray">
                All event types are hidden by filters.
              </Text>
            )}
          </Flex>
        </Box>
      </details>
    </Card>
  );
};
