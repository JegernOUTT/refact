import type { StreamStatus } from "../../streamSelectors";
import styles from "./Stream.module.css";

/**
 * Same shape as the relative label the old RecentItem rendered, but fed by the
 * epoch-millisecond timestamps the stream selectors expose.
 */
export function formatRelativeTime(timestampMs: number): string {
  const date = new Date(timestampMs);
  const diffMs = Date.now() - timestampMs;
  const diffMin = Math.floor(diffMs / 60_000);
  const diffHr = Math.floor(diffMs / 3_600_000);
  const diffDay = Math.floor(diffMs / 86_400_000);

  if (diffMin < 1) return "just now";
  if (diffMin < 60) return `${diffMin}m ago`;
  if (diffHr < 24) return `${diffHr}h ago`;
  if (diffDay < 7) return `${diffDay}d ago`;
  return date.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

export function formatCompactNumber(value: number): string {
  if (value >= 1_000_000) return `${Math.round(value / 100_000) / 10}M`;
  if (value >= 1_000) return `${Math.round(value / 100) / 10}K`;
  return String(value);
}

export type ChipTone = "accent" | "success" | "danger" | "warning" | "neutral";

export const TONE_CLASS: Record<ChipTone, string> = {
  accent: styles.toneAccent,
  success: styles.toneSuccess,
  danger: styles.toneDanger,
  warning: styles.toneWarning,
  neutral: styles.toneNeutral,
};

export function statusTone(status: StreamStatus): ChipTone {
  switch (status) {
    case "streaming":
    case "working":
      return "accent";
    case "done":
      return "success";
    case "failed":
      return "danger";
    case "paused":
    case "planning":
      return "warning";
    default:
      return "neutral";
  }
}

const STAGGER_CLASSES = [
  styles.d0,
  styles.d1,
  styles.d2,
  styles.d3,
  styles.d4,
  styles.d5,
  styles.d6,
  styles.d7,
];

export function staggerClass(index: number): string {
  return STAGGER_CLASSES[Math.min(Math.max(index, 0), 7)];
}
