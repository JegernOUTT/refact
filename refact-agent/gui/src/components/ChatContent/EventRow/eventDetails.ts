import type { EventMessage } from "../../../services/refact/types";
import { humanizeIdentifier } from "../../../utils/displayNames";
import { eventSourceLabel, eventSubkindLabel } from "./eventSubkind";
import {
  collapseWhitespace,
  eventPayloadRecord,
  formatDurationMs,
  isRecord,
  payloadBoolean,
  payloadNumber,
  payloadString,
} from "./eventPayload";

export type EventDetailField = {
  label: string;
  value: string;
  mono?: boolean;
};

type Payload = Record<string, unknown> | null;

function pushString(
  fields: EventDetailField[],
  label: string,
  value: string | null,
  mono?: boolean,
): void {
  if (value === null) return;
  fields.push(mono ? { label, value, mono } : { label, value });
}

function pushNumber(
  fields: EventDetailField[],
  label: string,
  value: number | null,
  suffix = "",
): void {
  if (value === null) return;
  fields.push({ label, value: `${value}${suffix}` });
}

function primitiveText(value: unknown): string | null {
  if (typeof value === "string") return value;
  if (typeof value === "number" && Number.isFinite(value)) return String(value);
  if (typeof value === "boolean") return value ? "yes" : "no";
  if (Array.isArray(value)) {
    const parts = value.map(primitiveText).filter((part) => part !== null);
    return parts.length > 0 ? parts.join(", ") : null;
  }
  return null;
}

// `mode_switch.diff` (chat/queue.rs `mode_switch_diff`) mixes three node shapes:
// `{from,to,changed}` deltas, `{count,names,truncated}` tool deltas and bare
// `{changed}` flags. Only changed nodes produce a line.
function flattenModeDiff(diff: unknown, path: string[] = []): string[] {
  if (!isRecord(diff)) {
    const text = primitiveText(diff);
    return text === null || path.length === 0
      ? []
      : [`${path.join(".")}: ${text}`];
  }

  const label = path.join(".");

  if ("from" in diff && "to" in diff) {
    const changed = diff.changed;
    const from = primitiveText(diff.from);
    const to = primitiveText(diff.to);
    if (changed === false || from === null || to === null || from === to) {
      return [];
    }
    return [`${label}: ${from} → ${to}`];
  }

  if ("count" in diff && "names" in diff) {
    const count = typeof diff.count === "number" ? diff.count : 0;
    if (count === 0) return [];
    const names = primitiveText(diff.names);
    const suffix = diff.truncated === true ? ", …" : "";
    return [
      names ? `${label}: ${count} (${names}${suffix})` : `${label}: ${count}`,
    ];
  }

  const changedOnly = Object.keys(diff).every(
    (key) => key === "changed" || key.endsWith("_count"),
  );
  if (changedOnly && path.length > 0) {
    return diff.changed === true ? [`${label}: changed`] : [];
  }

  const lines: string[] = [];
  for (const [key, value] of Object.entries(diff)) {
    if (key === "resolved" || key === "changed") continue;
    lines.push(...flattenModeDiff(value, [...path, key]));
  }
  return lines;
}

function modeSwitchFields(payload: Payload): EventDetailField[] {
  const fields: EventDetailField[] = [];
  const from = payloadString(payload, "from");
  const to = payloadString(payload, "to");
  if (from && to) {
    fields.push({
      label: "Transition",
      value: `${humanizeIdentifier(from)} → ${humanizeIdentifier(to)}`,
    });
  }
  pushString(fields, "Reason", payloadString(payload, "reason"));
  const diffLines = flattenModeDiff(payload?.diff);
  if (diffLines.length > 0) {
    fields.push({
      label: "Settings",
      value: diffLines.join("\n"),
      mono: true,
    });
  }
  return fields;
}

function processCompletedFields(payload: Payload): EventDetailField[] {
  const fields: EventDetailField[] = [];
  pushString(fields, "Process", payloadString(payload, "process_id"), true);
  pushString(fields, "Command", payloadString(payload, "short_description"));
  const status = payloadString(payload, "status");
  pushString(fields, "Status", status ? humanizeIdentifier(status) : null);
  pushNumber(fields, "Exit code", payloadNumber(payload, "exit_code"));
  const durationMs = payloadNumber(payload, "duration_ms");
  pushString(
    fields,
    "Duration",
    durationMs === null ? null : formatDurationMs(durationMs),
  );
  const mode = payloadString(payload, "mode");
  pushString(fields, "Mode", mode ? humanizeIdentifier(mode) : null);
  return fields;
}

function cronFireFields(payload: Payload): EventDetailField[] {
  const fields: EventDetailField[] = [];
  pushString(fields, "Task", payloadString(payload, "task_id"), true);
  pushString(fields, "Schedule", payloadString(payload, "cron"), true);
  const actionKind = payloadString(payload, "action_kind");
  pushString(
    fields,
    "Action",
    actionKind ? humanizeIdentifier(actionKind) : null,
  );
  pushNumber(fields, "Fire count", payloadNumber(payload, "fire_count"));
  const recurring = payloadBoolean(payload, "recurring");
  pushString(
    fields,
    "Recurring",
    recurring === null ? null : recurring ? "yes" : "no",
  );
  if (payloadBoolean(payload, "missed") === true) {
    fields.push({ label: "Missed", value: "yes" });
  }
  if (payloadBoolean(payload, "final") === true) {
    fields.push({ label: "Final run", value: "yes" });
  }
  return fields;
}

function cancellationNoteFields(
  event: EventMessage,
  payload: Payload,
): EventDetailField[] {
  const fields: EventDetailField[] = [];
  const source = payloadString(payload, "source");
  pushString(fields, "Requested by", source ?? eventSourceLabel(event.source));
  const reason = payloadString(payload, "reason");
  pushString(fields, "Reason", reason ? humanizeIdentifier(reason) : null);
  const push = payloadString(payload, "push");
  pushString(fields, "Push mode", push ? humanizeIdentifier(push) : null);
  pushString(fields, "Delivery", payloadString(payload, "delivery_id"), true);
  pushString(fields, "Discarded", collapseWhitespace(event.content) || null);
  return fields;
}

function deltaFields(payload: Payload): EventDetailField[] {
  const fields: EventDetailField[] = [];
  pushNumber(fields, "Sequence", payloadNumber(payload, "seq"));
  pushString(fields, "Summary", payloadString(payload, "summary"));
  if (payloadBoolean(payload, "truncated") !== true) return fields;
  const kept = payloadNumber(payload, "kept_chars");
  const original = payloadNumber(payload, "original_chars");
  fields.push({
    label: "Truncated",
    value:
      kept !== null && original !== null
        ? `kept ${kept} of ${original} chars`
        : "yes",
  });
  return fields;
}

function goalPursuitFields(payload: Payload): EventDetailField[] {
  const fields: EventDetailField[] = [];
  const kind = payloadString(payload, "kind");
  pushString(fields, "Event kind", kind ? humanizeIdentifier(kind) : null);
  const trigger = payloadString(payload, "trigger");
  pushString(fields, "Trigger", trigger ? humanizeIdentifier(trigger) : null);
  const reason = payloadString(payload, "reason");
  pushString(fields, "Reason", reason ? humanizeIdentifier(reason) : null);
  return fields;
}

function verifierReportFields(payload: Payload): EventDetailField[] {
  const fields: EventDetailField[] = [];
  const kind = payloadString(payload, "kind");
  pushString(fields, "Event kind", kind ? humanizeIdentifier(kind) : null);
  return fields;
}

function summarizationMarkerFields(payload: Payload): EventDetailField[] {
  const fields: EventDetailField[] = [];
  pushNumber(fields, "Tokens before", payloadNumber(payload, "tokens_before"));
  pushNumber(fields, "Tokens after", payloadNumber(payload, "tokens_after"));
  pushNumber(fields, "Messages", payloadNumber(payload, "messages_compacted"));
  return fields;
}

function toolDecisionFields(payload: Payload): EventDetailField[] {
  const fields: EventDetailField[] = [];
  const decision = payloadString(payload, "decision");
  pushString(
    fields,
    "Decision",
    decision ? humanizeIdentifier(decision) : null,
  );
  const scope = payloadString(payload, "scope");
  pushString(fields, "Scope", scope ? humanizeIdentifier(scope) : null);
  const ids = payload?.tool_call_ids;
  if (Array.isArray(ids)) {
    fields.push({ label: "Tool calls", value: String(ids.length) });
  }
  return fields;
}

function ideCallbackFields(payload: Payload): EventDetailField[] {
  const fields: EventDetailField[] = [];
  pushString(fields, "Tool call", payloadString(payload, "tool_call_id"), true);
  const ok = payloadBoolean(payload, "ok");
  pushString(fields, "Result", ok === null ? null : ok ? "ok" : "failed");
  pushString(fields, "Summary", payloadString(payload, "summary"));
  return fields;
}

function tickFields(payload: Payload): EventDetailField[] {
  const fields: EventDetailField[] = [];
  const elapsed = payloadNumber(payload, "elapsed_ms");
  pushString(
    fields,
    "Elapsed",
    elapsed === null ? null : formatDurationMs(elapsed),
  );
  const remaining = payloadNumber(payload, "remaining_ms");
  pushString(
    fields,
    "Remaining",
    remaining === null ? null : formatDurationMs(remaining),
  );
  return fields;
}

function systemNoticeFields(payload: Payload): EventDetailField[] {
  const fields: EventDetailField[] = [];
  pushString(fields, "Message", payloadString(payload, "message"));
  pushString(fields, "Error", payloadString(payload, "error"));
  pushString(fields, "From", payloadString(payload, "from"));
  pushString(fields, "Subagent", payloadString(payload, "config_name"), true);
  pushString(fields, "Summary", payloadString(payload, "summary"));
  pushNumber(
    fields,
    "Processes cleared",
    payloadNumber(payload, "killed_count"),
  );
  return fields;
}

function typedFields(
  event: EventMessage,
  payload: Payload,
): EventDetailField[] {
  switch (event.subkind) {
    case "mode_switch":
      return modeSwitchFields(payload);
    case "process_completed":
      return processCompletedFields(payload);
    case "cron_fire":
      return cronFireFields(payload);
    case "cancellation_note":
      return cancellationNoteFields(event, payload);
    case "plan_delta":
    case "goal_delta":
      return deltaFields(payload);
    case "goal_pursuit":
      return goalPursuitFields(payload);
    case "verifier_report":
      return verifierReportFields(payload);
    case "summarization_marker":
      return summarizationMarkerFields(payload);
    case "tool_decision":
      return toolDecisionFields(payload);
    case "ide_callback":
      return ideCallbackFields(payload);
    case "tick":
      return tickFields(payload);
    case "system_notice":
      return systemNoticeFields(payload);
    default:
      return [];
  }
}

export function eventDetailFields(event: EventMessage): EventDetailField[] {
  return [
    { label: "Source", value: eventSourceLabel(event.source) },
    { label: "Kind", value: eventSubkindLabel(event.subkind) },
    { label: "Message", value: event.content },
    ...typedFields(event, eventPayloadRecord(event)),
  ];
}

export function eventPayloadJson(event: EventMessage): string | null {
  const payload = event.payload;
  if (payload === undefined || payload === null) return null;
  if (isRecord(payload) && Object.keys(payload).length === 0) return null;
  try {
    return JSON.stringify(payload, null, 2);
  } catch {
    return null;
  }
}
