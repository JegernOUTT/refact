import type { PerformanceAggregate } from "../../services/refact/performance";

function finiteNumber(value: number | null | undefined): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function rounded(value: number, digits = 1): string {
  return value.toLocaleString(undefined, {
    maximumFractionDigits: digits,
  });
}

export function formatCount(value: number | null | undefined): string {
  const numeric = finiteNumber(value);
  return numeric === null ? "—" : Math.round(numeric).toLocaleString();
}

export function formatDurationUs(value: number | null | undefined): string {
  const microseconds = finiteNumber(value);
  if (microseconds === null) return "—";
  if (microseconds >= 1_000_000)
    return `${rounded(microseconds / 1_000_000)} s`;
  if (microseconds >= 1_000) return `${rounded(microseconds / 1_000)} ms`;
  return `${rounded(microseconds, 0)} µs`;
}

export function formatBytes(value: number | null | undefined): string {
  const bytes = finiteNumber(value);
  if (bytes === null) return "—";
  if (bytes < 1_024) return `${rounded(bytes, 0)} B`;
  if (bytes < 1_024 ** 2) return `${rounded(bytes / 1_024)} KB`;
  if (bytes < 1_024 ** 3) return `${rounded(bytes / 1_024 ** 2)} MB`;
  return `${rounded(bytes / 1_024 ** 3)} GB`;
}

export function formatTimestamp(value: number | null | undefined): string {
  const timestamp = finiteNumber(value);
  if (timestamp === null || timestamp <= 0) return "—";
  const date = new Date(timestamp);
  return Number.isNaN(date.getTime()) ? "—" : date.toLocaleString();
}

export function formatUptime(value: number | null | undefined): string {
  const milliseconds = finiteNumber(value);
  if (milliseconds === null || milliseconds <= 0) return "0s";
  const seconds = Math.floor(milliseconds / 1_000);
  const days = Math.floor(seconds / 86_400);
  const hours = Math.floor((seconds % 86_400) / 3_600);
  const minutes = Math.floor((seconds % 3_600) / 60);
  const remainder = seconds % 60;

  if (days > 0) return `${days}d ${hours}h ${minutes}m`;
  if (hours > 0) return `${hours}h ${minutes}m`;
  if (minutes > 0) return `${minutes}m ${remainder}s`;
  return `${remainder}s`;
}

export function formatComponentName(value: string | null | undefined): string {
  if (!value) return "Unknown component";
  return value
    .split(".")
    .map((part) => part.replace(/_/g, " "))
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" · ");
}

export function formatCounters(aggregate: PerformanceAggregate): string {
  const counters: string[] = [];
  const bytes = finiteNumber(aggregate.size_bytes_sum);
  const items = finiteNumber(aggregate.item_count_sum);
  const batchSize = finiteNumber(aggregate.batch_size_sum);

  if (bytes !== null && bytes > 0) counters.push(formatBytes(bytes));
  if (items !== null && items > 0) counters.push(`${formatCount(items)} items`);
  if (batchSize !== null && batchSize > 0) {
    counters.push(`${formatCount(batchSize)} batch items`);
  }

  return counters.length > 0 ? counters.join(" · ") : "—";
}

export function formatRatio(value: number | null | undefined): string {
  const ratio = finiteNumber(value);
  return ratio === null ? "—" : rounded(ratio, 2);
}
