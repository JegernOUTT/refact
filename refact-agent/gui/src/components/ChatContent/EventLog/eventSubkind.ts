import React from "react";
import type {
  EventMessage,
  EventSubkind,
} from "../../../services/refact/types";
import { isEventSubkind } from "../../../services/refact/types";
import type { LucideIcon } from "lucide-react";
import {
  AlarmClock,
  CheckCircle2,
  ClipboardList,
  Clock3,
  Compass,
  CircleDashed,
  FileText,
  Flag,
  Info,
  Microscope,
  Monitor,
  OctagonX,
  RefreshCw,
  Target,
} from "lucide-react";
import { Icon } from "../../ui";
import { humanizeIdentifier } from "../../../utils/displayNames";

/** Semantic tone shared by Badge (`tone`) and Icon (`tone`). */
export type EventTone =
  | "default"
  | "accent"
  | "success"
  | "warning"
  | "danger"
  | "muted";

export const EVENT_SUBKINDS: EventSubkind[] = [
  "mode_switch",
  "tool_decision",
  "ide_callback",
  "process_completed",
  "cron_fire",
  "tick",
  "summarization_marker",
  "verifier_report",
  "cancellation_note",
  "plan_delta",
  "goal_delta",
  "goal_pursuit",
  "system_notice",
];

const EVENT_SUBKIND_ICONS: Record<EventSubkind, LucideIcon> = {
  mode_switch: RefreshCw,
  tool_decision: CheckCircle2,
  ide_callback: Monitor,
  process_completed: Flag,
  cron_fire: AlarmClock,
  tick: Clock3,
  summarization_marker: FileText,
  cancellation_note: OctagonX,
  verifier_report: Microscope,
  plan_delta: ClipboardList,
  goal_delta: Target,
  goal_pursuit: Compass,
  system_notice: Info,
};

const EVENT_SUBKIND_TONES: Record<EventSubkind, EventTone> = {
  mode_switch: "accent",
  tool_decision: "success",
  ide_callback: "default",
  process_completed: "success",
  cron_fire: "accent",
  tick: "muted",
  summarization_marker: "muted",
  cancellation_note: "warning",
  verifier_report: "accent",
  plan_delta: "accent",
  goal_delta: "accent",
  goal_pursuit: "accent",
  system_notice: "default",
};

const EVENT_SUBKIND_LABELS: Record<EventSubkind, string> = {
  mode_switch: "Mode switch",
  tool_decision: "Tool decision",
  ide_callback: "IDE callback",
  process_completed: "Process finished",
  cron_fire: "Scheduled run",
  tick: "Tick",
  summarization_marker: "Context compacted",
  cancellation_note: "Cancellation",
  verifier_report: "Verifier report",
  plan_delta: "Plan update",
  goal_delta: "Goal update",
  goal_pursuit: "Goal pursuit",
  system_notice: "System notice",
};

const FAILURE_STATUSES = new Set([
  "failed",
  "failure",
  "error",
  "errored",
  "rejected",
  "denied",
  "crashed",
]);

const CAUTION_STATUSES = new Set([
  "killed",
  "cancelled",
  "canceled",
  "timeout",
  "timed_out",
  "aborted",
  "needs_work",
  "warning",
  "skipped",
]);

const SUCCESS_STATUSES = new Set([
  "ok",
  "success",
  "succeeded",
  "completed",
  "exited",
  "passed",
  "met",
  "fired",
  "accepted",
  "applied",
]);

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Icon for a subkind; unknown backend subkinds fall back to a neutral glyph. */
export function eventSubkindIcon(subkind: string): LucideIcon {
  return isEventSubkind(subkind) ? EVENT_SUBKIND_ICONS[subkind] : CircleDashed;
}

/** Base semantic tone for a subkind, before payload status refinement. */
export function eventSubkindTone(subkind: string): EventTone {
  return isEventSubkind(subkind) ? EVENT_SUBKIND_TONES[subkind] : "muted";
}

/** Human label for a subkind; never renders the raw identifier. */
export function eventSubkindLabel(subkind: string): string {
  if (isEventSubkind(subkind)) return EVENT_SUBKIND_LABELS[subkind];
  return subkind.length > 0 ? humanizeIdentifier(subkind) : "Event";
}

/**
 * Refine the subkind tone with the payload outcome so a failed process or a
 * rejected tool decision reads as failure rather than as its neutral kind.
 */
export function eventPayloadTone(payload: unknown): EventTone | null {
  if (!isRecord(payload)) return null;

  const exitCode = payload.exit_code ?? payload.exitCode;
  if (typeof exitCode === "number") {
    return exitCode === 0 ? "success" : "danger";
  }

  if (payload.tool_failed === true || payload.ok === false) return "danger";
  if (payload.accepted === false) return "warning";
  if (payload.ok === true || payload.accepted === true) return "success";

  for (const field of ["status", "verdict", "result", "level", "severity"]) {
    const value = payload[field];
    if (typeof value !== "string") continue;
    const normalized = value.toLowerCase();
    if (FAILURE_STATUSES.has(normalized)) return "danger";
    if (CAUTION_STATUSES.has(normalized)) return "warning";
    if (SUCCESS_STATUSES.has(normalized)) return "success";
  }

  return null;
}

/** Final tone for one event: payload outcome wins over the subkind default. */
export function eventTone(subkind: string, payload: unknown): EventTone {
  return eventPayloadTone(payload) ?? eventSubkindTone(subkind);
}

export function eventMessageTone(event: EventMessage): EventTone {
  return eventTone(event.subkind, event.payload);
}

/** Human label for an event source such as `chat.summarizer` or `agents.push`. */
export function eventSourceLabel(source: string): string {
  const trimmed = source.trim();
  if (trimmed.length === 0) return "Agent";
  return trimmed
    .split(".")
    .filter(Boolean)
    .map((part) => humanizeIdentifier(part))
    .join(" · ");
}

export function eventSubkindIconElement(
  subkind: string,
  tone: EventTone = "default",
): React.ReactElement {
  return React.createElement(Icon, {
    icon: eventSubkindIcon(subkind),
    size: "sm",
    tone,
  });
}
