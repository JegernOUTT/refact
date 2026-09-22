import type { EventMessage } from "../../../services/refact/types";
import { humanizeIdentifier } from "../../../utils/displayNames";
import {
  collapseWhitespace,
  eventPayloadRecord,
  payloadNumber,
  payloadString,
} from "./eventPayload";

function fallbackSummary(event: EventMessage): string {
  return collapseWhitespace(event.content);
}

function modeSwitchSummary(event: EventMessage): string | null {
  const payload = eventPayloadRecord(event);
  const from = payloadString(payload, "from");
  const to = payloadString(payload, "to");
  if (!from || !to) return null;
  const reason = payloadString(payload, "reason");
  const transition = `${humanizeIdentifier(from)} → ${humanizeIdentifier(to)}`;
  return reason ? `${transition} · ${collapseWhitespace(reason)}` : transition;
}

function processCompletedSummary(event: EventMessage): string | null {
  const payload = eventPayloadRecord(event);
  const description =
    payloadString(payload, "short_description") ??
    payloadString(payload, "process_id");
  if (!description) return null;
  const status = payloadString(payload, "status");
  const exitCode = payloadNumber(payload, "exit_code");
  const outcome =
    exitCode !== null
      ? `exit ${exitCode}`
      : status
        ? humanizeIdentifier(status).toLowerCase()
        : null;
  return collapseWhitespace(
    outcome ? `${description} · ${outcome}` : description,
  );
}

function cronFireSummary(event: EventMessage): string | null {
  const description = collapseWhitespace(event.content);
  if (description.length > 0) return description;
  const payload = eventPayloadRecord(event);
  const taskId = payloadString(payload, "task_id");
  return taskId ? `Task ${taskId}` : null;
}

function cancellationNoteSummary(event: EventMessage): string | null {
  const description = collapseWhitespace(event.content);
  if (description.length > 0) return description;
  const payload = eventPayloadRecord(event);
  const source = payloadString(payload, "source");
  const reason = payloadString(payload, "reason");
  if (source && reason) return `${humanizeIdentifier(reason)} by ${source}`;
  if (source) return `Interrupted by ${source}`;
  return reason ? humanizeIdentifier(reason) : null;
}

export function eventSummary(event: EventMessage): string {
  switch (event.subkind) {
    case "mode_switch":
      return modeSwitchSummary(event) ?? fallbackSummary(event);
    case "process_completed":
      return processCompletedSummary(event) ?? fallbackSummary(event);
    case "cron_fire":
      return cronFireSummary(event) ?? fallbackSummary(event);
    case "cancellation_note":
      return cancellationNoteSummary(event) ?? fallbackSummary(event);
    default:
      return fallbackSummary(event);
  }
}
