import { humanizeBuddyKey } from "./ui";
import type { BuddySettings, ChatReactionDebug } from "./types";

export const SPEECH_INTENT_KEYS: string[] = [
  "humor",
  "suggestion",
  "insight",
  "win",
  "error_alert",
  "greeting",
  "tour",
  "milestone",
  "memory_pulse_commentary",
  "quest_accept",
  "quest_complete",
  "chat_reaction",
];

const SPEECH_INTENT_LABELS: Partial<Record<string, string>> = {
  humor: "Humor",
  suggestion: "Suggestions",
  insight: "Insights",
  win: "Wins",
  error_alert: "Error alerts",
  greeting: "Greetings",
  tour: "Tours",
  milestone: "Milestones",
  memory_pulse_commentary: "Memory pulse commentary",
  quest_accept: "Quest acceptance",
  quest_complete: "Quest completion",
  chat_reaction: "Chat reactions",
};

const SPEECH_REASON_LABELS: Partial<Record<string, string>> = {
  allowed: "Spoke on its own",
  user_initiated: "You asked",
  chat_muted: "Held back: chat muted",
  intent_muted: "Held back: topic off",
  quiet_hours: "Held back: quiet hours",
  intent_budget: "Held back: topic limit",
};

export const OBSERVER_LABELS: Record<keyof BuddySettings["observers"], string> =
  {
    task_health: "Task health",
    trajectory_clutter: "Chat clutter",
    chat_pattern: "Chat patterns",
    customization_drift: "Customization drift",
    memory_garden: "Memory garden",
    mcp_auth: "MCP sign-in",
    git_pressure: "Uncommitted changes",
    diagnostic_cluster: "Repeated errors",
    provider_health: "Provider health",
  };

export function speechIntentLabel(intent: string | null | undefined): string {
  if (!intent) return "Speech";
  return SPEECH_INTENT_LABELS[intent] ?? "Other topic";
}

export function speechReasonLabel(reason: string, allowed: boolean): string {
  return SPEECH_REASON_LABELS[reason] ?? (allowed ? "Spoke" : "Held back");
}

export function isIntentAllowed(
  mutedIntents: readonly string[],
  intent: string,
): boolean {
  return !mutedIntents.includes(intent);
}

export function nextMutedIntents(
  mutedIntents: readonly string[],
  intent: string,
  allowed: boolean,
): string[] {
  const rest = mutedIntents.filter((key) => key !== intent);
  return allowed ? rest : [...rest, intent];
}

export function humorIntentBlocked(settings: BuddySettings): boolean {
  return !settings.humor_enabled || settings.humor_level === "off";
}

export function formatChatReactionStatus(
  debug: ChatReactionDebug | null,
): string {
  if (!debug) return "No chat reaction activity yet.";
  const counts = debug.counts_by_result as
    | Partial<Record<string, number>>
    | undefined;
  const emitted = counts?.emitted ?? 0;
  const skipped = counts?.skipped ?? 0;
  const last = debug.recent_attempts.at(-1);
  if (last?.result === "emitted") {
    return `Last sent: ${humanizeBuddyKey(
      last.signal_type,
      "chat reaction",
    ).toLowerCase()}.`;
  }
  if (debug.last_skip_reason) {
    return `Last skipped: ${humanizeBuddyKey(
      debug.last_skip_reason,
      "unknown reason",
    ).toLowerCase()}.`;
  }
  return `Sent ${emitted}, skipped ${skipped}.`;
}
