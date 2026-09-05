import type { LucideIcon } from "lucide-react";
import { MessageSquare, Send } from "lucide-react";
import type { PushMode, QueuedItem } from "../../features/Chat/Thread/types";
import { humanizeIdentifier } from "../../utils/displayNames";
import {
  eventSourceLabel,
  eventSubkindIcon,
  eventSubkindLabel,
  eventTone,
  type EventTone,
} from "./EventLog/eventSubkind";

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
};

export const PUSH_MODES: PushMode[] = ["preempt", "append", "when_idle"];

const PUSH_MODE_LABELS: Record<PushMode, string> = {
  preempt: "Interrupt now",
  append: "After current step",
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

export function pushModeLabel(mode: PushMode): string {
  return PUSH_MODE_LABELS[mode];
}

export function pushModeDescription(mode: PushMode): string {
  return PUSH_MODE_DESCRIPTIONS[mode];
}

export function pushModeTone(mode: PushMode): EventTone {
  return PUSH_MODE_TONES[mode];
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

export function describeQueuedItem(item: QueuedItem): QueuedItemPresentation {
  const isUserMessage = item.command_type === "user_message";
  const event = item.event;
  const rawSource = item.source ?? event?.source ?? null;

  const title =
    event?.subkind === "system_notice" && rawSource?.startsWith("agents.")
      ? eventTone(event.subkind, event.payload) === "success"
        ? "Agent completed"
        : "Agent update"
      : event
        ? eventSubkindLabel(event.subkind)
        : fallbackTitle(item.command_type);

  const icon = event
    ? eventSubkindIcon(event.subkind)
    : isUserMessage
      ? MessageSquare
      : Send;

  const tone: EventTone = event
    ? eventTone(event.subkind, event.payload)
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
  };
}
