import React, { useCallback, useId, useMemo, useState } from "react";
import { Box, Flex, Text } from "@radix-ui/themes";
import classNames from "classnames";
import { ChevronRight } from "lucide-react";
import { useAppDispatch } from "../../../hooks/useAppDispatch";
import { openScheduler } from "../../../features/Pages/pagesSlice";
import type { EventMessage } from "../../../services/refact/types";
import { Badge, Button, Icon } from "../../ui";
import type { EventRunPosition } from "../ChatContentDisplayItems";
import {
  eventMessageTone,
  eventSourceLabel,
  eventSubkindIcon,
  eventSubkindLabel,
} from "./eventSubkind";
import { eventDetailFields, eventPayloadJson } from "./eventDetails";
import { eventPayloadRecord, payloadString } from "./eventPayload";
import { eventSummary } from "./eventSummary";
import { eventTimestamp } from "./eventTimestamp";
import styles from "./EventRow.module.css";

export type EventRowProps = {
  event: EventMessage;
  run: EventRunPosition;
  onOpenProcessOutput?: (processId: string) => void;
};

const EventRowComponent: React.FC<EventRowProps> = ({
  event,
  run,
  onOpenProcessOutput,
}) => {
  const dispatch = useAppDispatch();
  const [expanded, setExpanded] = useState(false);
  const detailId = useId();

  const tone = useMemo(() => eventMessageTone(event), [event]);
  const summary = useMemo(() => eventSummary(event), [event]);
  const timestamp = useMemo(() => eventTimestamp(event), [event]);
  const fields = useMemo(
    () => (expanded ? eventDetailFields(event) : []),
    [event, expanded],
  );
  const payloadJson = useMemo(
    () => (expanded ? eventPayloadJson(event) : null),
    [event, expanded],
  );
  const processId = useMemo(
    () =>
      event.subkind === "process_completed"
        ? payloadString(eventPayloadRecord(event), "process_id")
        : null,
    [event],
  );
  const taskId = useMemo(
    () =>
      event.subkind === "cron_fire"
        ? payloadString(eventPayloadRecord(event), "task_id")
        : null,
    [event],
  );

  const handleToggle = useCallback(() => {
    setExpanded((current) => !current);
  }, []);

  const handleOpenProcessOutput = useCallback(() => {
    if (processId) onOpenProcessOutput?.(processId);
  }, [onOpenProcessOutput, processId]);

  const handleOpenScheduler = useCallback(() => {
    dispatch(openScheduler(taskId ? { taskId } : undefined));
  }, [dispatch, taskId]);

  const showProcessAction =
    processId !== null && onOpenProcessOutput !== undefined;
  const showSchedulerAction = event.subkind === "cron_fire";

  return (
    <Box
      className={styles.row}
      data-expanded={expanded}
      data-run={run}
      data-tone={tone}
      data-subkind={event.subkind}
      data-testid="event-row"
    >
      <button
        type="button"
        className={`${styles.header} rf-pressable`}
        aria-expanded={expanded}
        aria-controls={detailId}
        onClick={handleToggle}
      >
        <span
          className={classNames(styles.rail, styles[`tone-${tone}`])}
          aria-hidden="true"
        >
          <span className={styles.dot} />
        </span>
        <span className={styles.icon} aria-hidden="true">
          <Icon icon={eventSubkindIcon(event.subkind)} size="sm" tone={tone} />
        </span>
        <Badge tone={tone} size="xs" className={styles.chip}>
          {eventSubkindLabel(event.subkind)}
        </Badge>
        <Text as="span" size="1" className={styles.source}>
          {eventSourceLabel(event.source)}
        </Text>
        <Text as="span" size="1" className={styles.summary}>
          {summary}
        </Text>
        {timestamp !== null && (
          <Text as="span" size="1" className={styles.timestamp}>
            {timestamp}
          </Text>
        )}
        <span className={styles.chevron} aria-hidden="true">
          <Icon icon={ChevronRight} size="sm" tone="faint" />
        </span>
      </button>
      {expanded && (
        <Box
          id={detailId}
          className={`${styles.detail} rf-enter-rise`}
          data-testid="event-row-detail"
        >
          <div className={styles.grid}>
            {fields.map((field) => (
              <React.Fragment key={field.label}>
                <Text as="span" size="1" className={styles.fieldLabel}>
                  {field.label}
                </Text>
                <Text
                  as="span"
                  size="1"
                  className={classNames(
                    styles.fieldValue,
                    field.mono && styles.fieldValueMono,
                  )}
                >
                  {field.value}
                </Text>
              </React.Fragment>
            ))}
          </div>
          {(showProcessAction || showSchedulerAction) && (
            <Flex gap="2" wrap="wrap" className={styles.actions}>
              {showProcessAction && (
                <Button
                  size="sm"
                  variant="soft"
                  onClick={handleOpenProcessOutput}
                >
                  Open output
                </Button>
              )}
              {showSchedulerAction && (
                <Button size="sm" variant="soft" onClick={handleOpenScheduler}>
                  Open scheduler
                </Button>
              )}
            </Flex>
          )}
          {payloadJson !== null && (
            <details className={styles.jsonPanel}>
              <summary className={styles.jsonSummary}>Payload</summary>
              <pre className={styles.jsonPre} data-testid="event-row-payload">
                {payloadJson}
              </pre>
            </details>
          )}
        </Box>
      )}
    </Box>
  );
};

EventRowComponent.displayName = "EventRow";

export const EventRow = React.memo(EventRowComponent);
