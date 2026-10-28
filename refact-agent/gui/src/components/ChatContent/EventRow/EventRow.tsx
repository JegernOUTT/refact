import React, { useCallback, useMemo } from "react";
import { useAppDispatch } from "../../../hooks/useAppDispatch";
import { openScheduler } from "../../../features/Pages/pagesSlice";
import type { EventMessage } from "../../../services/refact/types";
import { Button, Icon } from "../../ui";
import type { EventRunPosition } from "../ChatContentDisplayItems";
import {
  TranscriptItem,
  fromOutcomeWord,
  type TranscriptFact,
  type TranscriptRawBlock,
  type TranscriptStatus,
} from "../TranscriptItem";
import { eventMessageTone, eventSubkindIcon } from "./eventSubkind";
import { eventDetailFields } from "./eventDetails";
import {
  collapseWhitespace,
  eventPayloadRecord,
  isRecord,
  payloadNumber,
  payloadString,
} from "./eventPayload";
import { eventSummary } from "./eventSummary";
import { eventTimestamp } from "./eventTimestamp";
import styles from "./EventRow.module.css";

export type EventRowProps = {
  event: EventMessage;
  run?: EventRunPosition;
  onOpenProcessOutput?: (processId: string) => void;
};

const SUBKIND_VERBS: Partial<Record<string, string>> = {
  tool_decision: "Approval",
  process_completed: "Process finished",
  cron_fire: "Scheduler",
  plan_delta: "Plan",
  goal_delta: "Goal",
  goal_pursuit: "Goal",
  verifier_report: "Verifier",
  mode_switch: "Mode",
  summarization_marker: "Compressed",
  cancellation_note: "Cancelled",
  ide_callback: "IDE",
  tick: "Tick",
};

const SOURCE_VERBS: Partial<Record<string, string>> = {
  "agents.approval": "Approval",
  "agents.message": "Subagent note",
  "agents.inbox": "Subagent note",
  "agents.native_note": "Subagent note",
  "agents.sibling": "Subagent note",
  "agents.spawn": "Subagent finished",
  "agents.cap_stop": "Subagent stopped",
  "chat.command.set_goal": "Goal",
  "chat.command.update_goal": "Goal",
  "chat.goal_monitor": "Goal",
  "tool.set_goal": "Goal",
  "goal.plan_install": "Plan",
  "scheduler.cron": "Scheduler",
};

const QUESTION_PREFIX = "[subagent question]";
const LEADING_TAG = /^\s*\[[^\]\n]{1,60}\]\s*/;
const CLOCK_LENGTH = 5;

function eventVerb(event: EventMessage): string {
  if (event.content.trimStart().startsWith(QUESTION_PREFIX)) return "Question";
  return SOURCE_VERBS[event.source] ?? SUBKIND_VERBS[event.subkind] ?? "Notice";
}

function eventTarget(event: EventMessage): string {
  return collapseWhitespace(eventSummary(event).replace(LEADING_TAG, ""));
}

function outcomeWord(event: EventMessage): string | null {
  const payload = eventPayloadRecord(event);
  const exitCode = payloadNumber(payload, "exit_code");
  if (exitCode !== null) return exitCode === 0 ? "completed" : "failed";
  if (payload?.ok === false || payload?.tool_failed === true) return "failed";
  const decision = payloadString(payload, "decision");
  if (decision === "reject") return "rejected";
  if (decision === "approve") return "accepted";
  for (const field of ["status", "verdict", "result"]) {
    const word = payloadString(payload, field);
    if (word) return word;
  }
  return null;
}

function eventStatus(event: EventMessage): TranscriptStatus {
  const status = fromOutcomeWord(outcomeWord(event));
  return status === "unknown" || status === "running" ? "done" : status;
}

function eventFacts(event: EventMessage): TranscriptFact[] {
  return eventDetailFields(event)
    .filter(
      (field) => field.label !== "Message" && field.value.trim().length > 0,
    )
    .map((field) => ({
      label: field.label,
      value: field.mono ? <code>{field.value}</code> : field.value,
    }));
}

function eventRaw(event: EventMessage): TranscriptRawBlock[] {
  const payload = event.payload;
  if (payload === undefined || payload === null) return [];
  if (isRecord(payload) && Object.keys(payload).length === 0) return [];
  return [{ label: "Payload", value: payload }];
}

const EventRowComponent: React.FC<EventRowProps> = ({
  event,
  onOpenProcessOutput,
}) => {
  const dispatch = useAppDispatch();
  const payload = useMemo(() => eventPayloadRecord(event), [event]);
  const facts = useMemo(() => eventFacts(event), [event]);
  const raw = useMemo(() => eventRaw(event), [event]);
  const clock = useMemo(
    () => eventTimestamp(event)?.slice(0, CLOCK_LENGTH) ?? null,
    [event],
  );
  const processId =
    event.subkind === "process_completed"
      ? payloadString(payload, "process_id")
      : null;
  const taskId =
    event.subkind === "cron_fire" ? payloadString(payload, "task_id") : null;

  const handleOpenProcessOutput = useCallback(() => {
    if (processId) onOpenProcessOutput?.(processId);
  }, [onOpenProcessOutput, processId]);

  const handleOpenScheduler = useCallback(() => {
    dispatch(openScheduler(taskId ? { taskId } : undefined));
  }, [dispatch, taskId]);

  const showProcessAction =
    processId !== null && onOpenProcessOutput !== undefined;
  const showSchedulerAction = event.subkind === "cron_fire";
  const actions =
    showProcessAction || showSchedulerAction ? (
      <>
        {showProcessAction && (
          <Button size="sm" variant="soft" onClick={handleOpenProcessOutput}>
            Open output
          </Button>
        )}
        {showSchedulerAction && (
          <Button size="sm" variant="soft" onClick={handleOpenScheduler}>
            Open scheduler
          </Button>
        )}
      </>
    ) : null;

  return (
    <TranscriptItem
      data-testid="event-row"
      data-subkind={event.subkind}
      data-tone={eventMessageTone(event)}
      icon={<Icon icon={eventSubkindIcon(event.subkind)} size="sm" />}
      verb={eventVerb(event)}
      target={eventTarget(event)}
      meta={clock}
      status={eventStatus(event)}
      summary={facts}
      sectionLabels={{ output: "Message" }}
      output={
        event.content.trim() ? (
          <div className={styles.message}>{event.content.trim()}</div>
        ) : null
      }
      actions={actions}
      raw={raw}
      openKey={event.message_id ? `event:${event.message_id}` : undefined}
    />
  );
};

EventRowComponent.displayName = "EventRow";

export const EventRow = React.memo(EventRowComponent);
