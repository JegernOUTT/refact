import React, { useMemo, useState } from "react";
import { Box, Flex, Text } from "@radix-ui/themes";
import type { EventMessage } from "../../../services/refact/types";
import { eventSubkindIcon } from "./eventSubkind";
import styles from "./EventLog.module.css";

type EventLogEntryProps = {
  event: EventMessage;
  entryId: string;
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function timestampFromValue(value: unknown): string | null {
  let date: Date | null = null;

  if (typeof value === "number" && Number.isFinite(value)) {
    const millis = value > 10_000_000_000 ? value : value * 1000;
    date = new Date(millis);
  }

  if (typeof value === "string" && value.trim().length > 0) {
    const parsed = Date.parse(value);
    if (Number.isFinite(parsed)) {
      date = new Date(parsed);
    }
  }

  if (!date || Number.isNaN(date.getTime())) return null;

  const hours = date.getHours().toString().padStart(2, "0");
  const minutes = date.getMinutes().toString().padStart(2, "0");
  const seconds = date.getSeconds().toString().padStart(2, "0");
  return `${hours}:${minutes}:${seconds}`;
}

function eventTimestamp(event: EventMessage): string {
  const payload = isRecord(event.payload) ? event.payload : null;
  const candidates = [
    payload?.timestamp,
    payload?.created_at_ms,
    payload?.created_at,
    payload?.ts,
    event.extra?.timestamp,
    event.extra?.created_at_ms,
    event.extra?.created_at,
  ];

  for (const candidate of candidates) {
    const formatted = timestampFromValue(candidate);
    if (formatted) return formatted;
  }

  return "--:--:--";
}

function payloadJson(payload: unknown): string {
  const value = payload === undefined ? {} : payload;
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

function summaryText(content: string): string {
  return content.replace(/\s+/g, " ").trim() || "No summary";
}

export const EventLogEntry: React.FC<EventLogEntryProps> = ({
  event,
  entryId,
}) => {
  const [expanded, setExpanded] = useState(false);
  const jsonId = `${entryId}-json`;
  const timestamp = useMemo(() => eventTimestamp(event), [event]);
  const formattedPayload = useMemo(() => payloadJson(event.payload), [event]);

  return (
    <Box className={styles.entry} data-testid="event-log-entry">
      <button
        type="button"
        className={styles.entryButton}
        aria-expanded={expanded}
        aria-controls={jsonId}
        onClick={() => setExpanded((current) => !current)}
      >
        <Flex align="center" gap="2" className={styles.entryRow}>
          <Text as="span" className={styles.icon} aria-hidden="true">
            {eventSubkindIcon(event.subkind)}
          </Text>
          <Text as="span" size="1" className={styles.timestamp}>
            {timestamp}
          </Text>
          <Text as="span" size="1" className={styles.subkindChip}>
            {event.subkind}
          </Text>
          <Text as="span" size="1" className={styles.source}>
            {event.source}
          </Text>
          <Text as="span" size="1" className={styles.summaryText}>
            {summaryText(event.content)}
          </Text>
        </Flex>
      </button>
      {expanded && (
        <Box
          id={jsonId}
          className={styles.jsonPanel}
          data-testid={`event-log-json-${entryId}`}
        >
          <pre className={styles.jsonPre}>{formattedPayload}</pre>
        </Box>
      )}
    </Box>
  );
};
