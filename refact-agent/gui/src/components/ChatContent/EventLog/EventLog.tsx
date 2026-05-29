import React, { useEffect, useMemo, useState } from "react";
import { Box, Card, Flex, Text } from "@radix-ui/themes";
import { useAppDispatch } from "../../../hooks";
import { openScheduler } from "../../../features/Pages/pagesSlice";
import type {
  EventMessage,
  EventSubkind,
} from "../../../services/refact/types";
import { EventLogEntry } from "./EventLogEntry";
import { eventSubkindIcon } from "./eventSubkind";
import styles from "./EventLog.module.css";

export type EventLogProps = {
  events: EventMessage[];
  threadId: string;
  filterEvents?: EventMessage[];
  onProcessCompletedClick?: (processId: string) => void;
};

const EVENT_SUBKINDS: EventSubkind[] = [
  "mode_switch",
  "tool_decision",
  "ide_callback",
  "process_completed",
  "cron_fire",
  "tick",
  "summarization_marker",
  "cancellation_note",
  "verifier_report",
  "system_notice",
];

function collapsedStorageKey(threadId: string): string {
  return `event-log-collapsed-${threadId}`;
}

function filterStorageKey(threadId: string): string {
  return `event-log-filter-${threadId}`;
}

function isEventSubkind(value: unknown): value is EventSubkind {
  return (
    typeof value === "string" && EVENT_SUBKINDS.includes(value as EventSubkind)
  );
}

function readCollapsed(threadId: string): boolean {
  try {
    if (typeof localStorage === "undefined") return true;
    const stored = localStorage.getItem(collapsedStorageKey(threadId));
    if (stored === "false") return false;
    return true;
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

function readSelectedSubkinds(threadId: string): EventSubkind[] {
  try {
    if (typeof localStorage === "undefined") return EVENT_SUBKINDS;
    const stored = localStorage.getItem(filterStorageKey(threadId));
    if (!stored) return EVENT_SUBKINDS;
    const parsed = JSON.parse(stored) as unknown;
    if (!Array.isArray(parsed)) return EVENT_SUBKINDS;
    const selected = parsed.filter(isEventSubkind);
    return selected;
  } catch {
    return EVENT_SUBKINDS;
  }
}

function writeSelectedSubkinds(
  threadId: string,
  selectedSubkinds: EventSubkind[],
): void {
  try {
    if (typeof localStorage === "undefined") return;
    localStorage.setItem(
      filterStorageKey(threadId),
      JSON.stringify(selectedSubkinds),
    );
  } catch {
    return;
  }
}

function entryKey(event: EventMessage, index: number): string {
  return event.message_id ?? `${event.subkind}-${event.source}-${index}`;
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
  filterEvents = events,
  onProcessCompletedClick,
}) => {
  const dispatch = useAppDispatch();
  const [collapsed, setCollapsed] = useState(() => readCollapsed(threadId));
  const [selectedSubkinds, setSelectedSubkinds] = useState(() =>
    readSelectedSubkinds(threadId),
  );

  useEffect(() => {
    setCollapsed(readCollapsed(threadId));
    setSelectedSubkinds(readSelectedSubkinds(threadId));
  }, [threadId]);

  const presentSubkinds = useMemo(() => {
    return EVENT_SUBKINDS.filter((subkind) =>
      filterEvents.some((event) => event.subkind === subkind),
    );
  }, [filterEvents]);

  const selectedSet = useMemo(
    () => new Set<EventSubkind>(selectedSubkinds),
    [selectedSubkinds],
  );

  const filteredEvents = useMemo(
    () => events.filter((event) => selectedSet.has(event.subkind)),
    [events, selectedSet],
  );

  if (events.length === 0) return null;

  const handleSummaryClick = (event: React.MouseEvent<HTMLElement>) => {
    event.preventDefault();
    setCollapsed((current) => {
      const next = !current;
      writeCollapsed(threadId, next);
      return next;
    });
  };

  const toggleSubkind = (subkind: EventSubkind) => {
    setSelectedSubkinds((current) => {
      const currentSet = new Set(current);
      if (currentSet.has(subkind)) {
        currentSet.delete(subkind);
      } else {
        currentSet.add(subkind);
      }
      const next = EVENT_SUBKINDS.filter((candidate) =>
        currentSet.has(candidate),
      );
      writeSelectedSubkinds(threadId, next);
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
      <details open={!collapsed}>
        <summary className={styles.summary} onClick={handleSummaryClick}>
          <Flex align="center" gap="2" wrap="wrap">
            <Text as="span" size="1" weight="medium">
              Event log
            </Text>
            <Text as="span" size="1" className={styles.count}>
              {events.length} {events.length === 1 ? "event" : "events"}
            </Text>
          </Flex>
        </summary>
        <Box className={styles.body}>
          <Flex gap="1" wrap="wrap" className={styles.filters}>
            {presentSubkinds.map((subkind) => (
              <label key={subkind} className={styles.filterChip}>
                <input
                  type="checkbox"
                  checked={selectedSet.has(subkind)}
                  onChange={() => toggleSubkind(subkind)}
                />
                <Text as="span" size="1" aria-hidden="true">
                  {eventSubkindIcon(subkind)}
                </Text>
                <Text as="span" size="1">
                  {subkind}
                </Text>
              </label>
            ))}
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
                All event subkinds are hidden by filters.
              </Text>
            )}
          </Flex>
        </Box>
      </details>
    </Card>
  );
};
