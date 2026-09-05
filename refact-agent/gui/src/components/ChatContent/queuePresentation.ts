import type { LucideIcon } from "lucide-react";
import { MessageSquare, Pause, Send, SkipForward, Zap } from "lucide-react";
import type { PushMode, QueuedItem } from "../../features/Chat/Thread/types";
import { humanizeIdentifier } from "../../utils/displayNames";
import {
  eventSourceLabel,
  eventSubkindIcon,
  eventSubkindLabel,
  eventTone,
  type EventTone,
} from "./EventRow/eventSubkind";

export type QueuedItemPresentation = {
  title: string;
  preview: string;
  source: string | null;
  icon: LucideIcon;
  tone: EventTone;
  /** Legacy user messages keep inline edit and the two-state priority toggle. */
  isUserMessage: boolean;
  /** Anything the engine delivers on the unified A/B/C wire. */
  isDelivery: boolean;
  push: PushMode;
  /** Present when the item is a finished process the transcript can reveal. */
  processId: string | null;
};

export const PUSH_MODES: PushMode[] = ["preempt", "append", "when_idle"];

const PUSH_MODE_LABELS: Record<PushMode, string> = {
  preempt: "Interrupt now",
  append: "After current step",
  when_idle: "When idle",
};

/** Chip-sized labels: the row has one line, the dropdown shows the full label. */
const PUSH_MODE_SHORT_LABELS: Record<PushMode, string> = {
  preempt: "Interrupt now",
  append: "After step",
  when_idle: "When idle",
};

const PUSH_MODE_DESCRIPTIONS: Record<PushMode, string> = {
  preempt:
    "Cancels the active step and discards its partial output before delivery.",
  append:
    "Delivered after the assistant response and all its tool results finish.",
  when_idle: "Delivered only after the agent reaches final idle.",
};

const PUSH_MODE_TONES: Record<PushMode, EventTone> = {
  preempt: "danger",
  append: "accent",
  when_idle: "muted",
};

const PUSH_MODE_ICONS: Record<PushMode, LucideIcon> = {
  preempt: Zap,
  append: SkipForward,
  when_idle: Pause,
};

export function pushModeLabel(mode: PushMode): string {
  return PUSH_MODE_LABELS[mode];
}

export function pushModeShortLabel(mode: PushMode): string {
  return PUSH_MODE_SHORT_LABELS[mode];
}

export function pushModeDescription(mode: PushMode): string {
  return PUSH_MODE_DESCRIPTIONS[mode];
}

export function pushModeTone(mode: PushMode): EventTone {
  return PUSH_MODE_TONES[mode];
}

export function pushModeIcon(mode: PushMode): LucideIcon {
  return PUSH_MODE_ICONS[mode];
}

export type QueueModeOption = {
  value: PushMode;
  label: string;
  shortLabel: string;
  description: string;
  tone: EventTone;
  icon: LucideIcon;
};

/**
 * Legacy user messages only ever had a boolean priority, so their chip offers
 * the two states that boolean can express, on the same `PushMode` values the
 * row already reports through `data-push`.
 */
const USER_MESSAGE_MODES: Record<
  "preempt" | "append",
  { label: string; description: string }
> = {
  preempt: {
    label: "Send next",
    description: "Delivered before the rest of the queue.",
  },
  append: {
    label: "In order",
    description: "Delivered in the order it was queued.",
  },
};

export function queueModeOptions(isUserMessage: boolean): QueueModeOption[] {
  if (isUserMessage) {
    return (["preempt", "append"] as const).map((mode) => ({
      value: mode,
      label: USER_MESSAGE_MODES[mode].label,
      shortLabel: USER_MESSAGE_MODES[mode].label,
      description: USER_MESSAGE_MODES[mode].description,
      tone: pushModeTone(mode),
      icon: pushModeIcon(mode),
    }));
  }
  return PUSH_MODES.map((mode) => ({
    value: mode,
    label: pushModeLabel(mode),
    shortLabel: pushModeShortLabel(mode),
    description: pushModeDescription(mode),
    tone: pushModeTone(mode),
    icon: pushModeIcon(mode),
  }));
}

/** `append` is the default: the engine treats a missing push the same way. */
export function queuedItemPushMode(item: QueuedItem): PushMode {
  if (item.push) return item.push;
  return item.priority ? "preempt" : "append";
}

function fallbackTitle(commandType: string): string {
  if (commandType === "user_message") return "Your message";
  if (commandType === "delivery" || commandType.length === 0) {
    return "Agent continuation";
  }
  return humanizeIdentifier(commandType);
}

function queuedItemProcessId(item: QueuedItem): string | null {
  if (item.event?.subkind !== "process_completed") return null;
  const payload: unknown = item.event.payload;
  if (typeof payload !== "object" || payload === null) return null;
  const processId = (payload as Record<string, unknown>).process_id;
  return typeof processId === "string" && processId.length > 0
    ? processId
    : null;
}

export function describeQueuedItem(item: QueuedItem): QueuedItemPresentation {
  const isUserMessage = item.command_type === "user_message";
  const event = item.event;
  const subkind =
    typeof event?.subkind === "string" && event.subkind.length > 0
      ? event.subkind
      : null;
  const rawSource = item.source ?? event?.source ?? null;

  const title =
    subkind === "system_notice" && rawSource?.startsWith("agents.")
      ? eventTone(subkind, event?.payload) === "success"
        ? "Agent completed"
        : "Agent update"
      : subkind
        ? eventSubkindLabel(subkind)
        : fallbackTitle(item.command_type);

  const icon = subkind
    ? eventSubkindIcon(subkind)
    : isUserMessage
      ? MessageSquare
      : Send;

  const tone: EventTone = subkind
    ? eventTone(subkind, event?.payload)
    : isUserMessage
      ? "accent"
      : "default";

  const preview = item.preview.trim() || (item.content?.trim() ?? "") || title;

  return {
    title,
    preview,
    source: rawSource ? eventSourceLabel(rawSource) : null,
    icon,
    tone,
    isUserMessage,
    isDelivery: !isUserMessage,
    push: queuedItemPushMode(item),
    processId: queuedItemProcessId(item),
  };
}

/**
 * One-line summary of what the engine will deliver next. `append` reads as
 * "ready to deliver" when the agent is idle because nothing is holding it.
 */
export function queueStatusText(
  queuedItems: QueuedItem[],
  isBusy: boolean,
  waitingInterruptible = false,
): string {
  const count = (push: PushMode) =>
    queuedItems.filter((item) => queuedItemPushMode(item) === push).length;
  if (isBusy && waitingInterruptible) {
    const delivering = count("append") + count("when_idle");
    return [
      delivering ? `${delivering} delivering now` : "",
      count("preempt") ? `${count("preempt")} interrupting` : "",
    ]
      .filter(Boolean)
      .join(" · ");
  }
  return [
    count("preempt") ? `${count("preempt")} interrupting` : "",
    count("append")
      ? `${count("append")} ${
          isBusy && waitingInterruptible
            ? "delivering now"
            : isBusy
              ? "after step"
              : "ready to deliver"
        }`
      : "",
    count("when_idle") ? `${count("when_idle")} when idle` : "",
  ]
    .filter(Boolean)
    .join(" · ");
}
