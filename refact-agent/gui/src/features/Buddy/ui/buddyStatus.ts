import type { BadgeTone } from "../../../components/ui";

export type BuddyStatusTone = Exclude<BadgeTone, "accent" | "muted">;

export interface BuddyStatusEntry {
  label: string;
  tone: BuddyStatusTone;
}

export type BuddyWorkState =
  | "needs_you"
  | "working"
  | "delivered"
  | "blocked"
  | "cancelled"
  | "snoozed"
  | "watching";

export type BuddySeverity = "critical" | "high" | "medium" | "low";
export type BuddyReadState = "unread" | "read";
export type BuddyHealth = "ok" | "checking" | "degraded" | "failed";
export type BuddyEnabledState = "enabled" | "disabled";

export const BUDDY_WORK_STATUS: Record<BuddyWorkState, BuddyStatusEntry> = {
  needs_you: { label: "Needs you", tone: "warning" },
  working: { label: "Working", tone: "default" },
  delivered: { label: "Delivered", tone: "success" },
  blocked: { label: "Blocked", tone: "danger" },
  cancelled: { label: "Cancelled", tone: "default" },
  snoozed: { label: "Snoozed", tone: "default" },
  watching: { label: "Watching", tone: "default" },
};

export const BUDDY_SEVERITY_STATUS: Record<BuddySeverity, BuddyStatusEntry> = {
  critical: { label: "Critical", tone: "danger" },
  high: { label: "High", tone: "warning" },
  medium: { label: "Medium", tone: "default" },
  low: { label: "Low", tone: "default" },
};

export const BUDDY_READ_STATUS: Record<BuddyReadState, BuddyStatusEntry> = {
  unread: { label: "Unread", tone: "default" },
  read: { label: "Read", tone: "default" },
};

export const BUDDY_HEALTH_STATUS: Record<BuddyHealth, BuddyStatusEntry> = {
  ok: { label: "Healthy", tone: "success" },
  checking: { label: "Checking", tone: "default" },
  degraded: { label: "Degraded", tone: "warning" },
  failed: { label: "Failed", tone: "danger" },
};

export const BUDDY_ENABLED_STATUS: Record<BuddyEnabledState, BuddyStatusEntry> =
  {
    enabled: { label: "On", tone: "success" },
    disabled: { label: "Off", tone: "default" },
  };

export type BuddyStatusDimension =
  | "work"
  | "severity"
  | "read"
  | "health"
  | "enabled";

export interface BuddyStatusValueMap {
  work: BuddyWorkState;
  severity: BuddySeverity;
  read: BuddyReadState;
  health: BuddyHealth;
  enabled: BuddyEnabledState;
}

export const BUDDY_STATUS_MAPS: {
  [D in BuddyStatusDimension]: Record<BuddyStatusValueMap[D], BuddyStatusEntry>;
} = {
  work: BUDDY_WORK_STATUS,
  severity: BUDDY_SEVERITY_STATUS,
  read: BUDDY_READ_STATUS,
  health: BUDDY_HEALTH_STATUS,
  enabled: BUDDY_ENABLED_STATUS,
};

const WORK_ALIASES: Partial<Record<string, BuddyWorkState>> = {
  needs_attention: "needs_you",
  needs_you: "needs_you",
  working: "working",
  working_on: "working",
  delivered: "delivered",
  blocked: "blocked",
  cancelled: "cancelled",
  canceled: "cancelled",
  snoozed: "snoozed",
  watching: "watching",
};

const SEVERITY_ALIASES: Partial<Record<string, BuddySeverity>> = {
  critical: "critical",
  high: "high",
  medium: "medium",
  normal: "medium",
  low: "low",
};

const HEALTH_ALIASES: Partial<Record<string, BuddyHealth>> = {
  ok: "ok",
  healthy: "ok",
  enabled: "ok",
  loading: "checking",
  checking: "checking",
  degraded: "degraded",
  failed: "failed",
  error: "failed",
  unavailable: "failed",
};

export function toBuddyWorkState(
  value: string | null | undefined,
): BuddyWorkState | null {
  return value ? WORK_ALIASES[value] ?? null : null;
}

export function toBuddySeverity(
  value: string | null | undefined,
): BuddySeverity | null {
  return value ? SEVERITY_ALIASES[value] ?? null : null;
}

export function toBuddyHealth(
  value: string | null | undefined,
): BuddyHealth | null {
  return value ? HEALTH_ALIASES[value] ?? null : null;
}

export type BuddyOperatorMode =
  | "loading"
  | "error"
  | "enabled"
  | "disabled"
  | "unavailable";

export const BUDDY_OPERATOR_STATUS: Record<
  BuddyOperatorMode,
  BuddyStatusEntry & { health: BuddyHealth | null }
> = {
  loading: { label: "Checking", tone: "default", health: "checking" },
  error: { label: "Broken", tone: "danger", health: "failed" },
  enabled: { label: "Enabled", tone: "success", health: "ok" },
  disabled: { label: "Local only", tone: "default", health: null },
  unavailable: { label: "Not responding", tone: "danger", health: "failed" },
};

export function buddyStatus<D extends BuddyStatusDimension>(
  dimension: D,
  value: BuddyStatusValueMap[D],
): BuddyStatusEntry {
  const map = BUDDY_STATUS_MAPS[dimension] as Record<string, BuddyStatusEntry>;
  return map[value];
}
