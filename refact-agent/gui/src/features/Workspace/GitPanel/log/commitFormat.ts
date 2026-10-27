const MINUTE_MS = 60_000;
const HOUR_MS = 60 * MINUTE_MS;
const DAY_MS = 24 * HOUR_MS;
const WEEK_DAYS = 7;

export function formatCommitDate(timeMs: number, now = Date.now()): string {
  if (!Number.isFinite(timeMs) || timeMs <= 0) return "";
  const diff = now - timeMs;
  if (diff < MINUTE_MS) return "just now";
  if (diff < HOUR_MS) return `${Math.floor(diff / MINUTE_MS)} min ago`;
  if (diff < DAY_MS) return `${Math.floor(diff / HOUR_MS)} h ago`;
  const days = Math.floor(diff / DAY_MS);
  if (days === 1) return "Yesterday";
  if (days < WEEK_DAYS) return `${days} days ago`;
  const date = new Date(timeMs);
  const sameYear = date.getFullYear() === new Date(now).getFullYear();
  return date.toLocaleDateString(undefined, {
    day: "numeric",
    month: "short",
    year: sameYear ? undefined : "numeric",
  });
}

export function formatAbsoluteDate(timeMs: number): string {
  if (!Number.isFinite(timeMs) || timeMs <= 0) return "";
  return new Date(timeMs).toLocaleString(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  });
}

export function splitMessage(message: string): {
  subject: string;
  body: string;
} {
  const trimmed = message.replace(/\s+$/, "");
  const index = trimmed.indexOf("\n");
  if (index === -1) return { subject: trimmed, body: "" };
  return {
    subject: trimmed.slice(0, index),
    body: trimmed.slice(index + 1).replace(/^\s*\n/, ""),
  };
}
