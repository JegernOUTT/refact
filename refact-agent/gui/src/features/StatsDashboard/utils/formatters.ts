export function formatTokenCount(tokens: number): string {
  if (tokens >= 1_000_000_000) return `${(tokens / 1_000_000_000).toFixed(1)}B`;
  if (tokens >= 1_000_000) return `${(tokens / 1_000_000).toFixed(1)}M`;
  if (tokens >= 1_000) return `${(tokens / 1_000).toFixed(1)}K`;
  return tokens.toString();
}

export function formatCost(usd: number | null): string {
  if (usd == null) return "—";
  return `$${usd.toFixed(2)}`;
}

export function formatCostDisplay(usd: number | null): string {
  if (usd != null && usd > 0) return `$${usd.toFixed(2)}`;
  if (usd != null) return `$${usd.toFixed(2)}`;
  return "—";
}

/** Smaller amounts keep more precision so sub-cent costs aren't all "$0.00". */
export function formatCostPrecise(usd: number | null): string {
  if (usd == null) return "—";
  if (usd === 0) return "$0.00";
  if (Math.abs(usd) < 0.01) return `$${usd.toFixed(4)}`;
  if (Math.abs(usd) < 1) return `$${usd.toFixed(3)}`;
  return `$${usd.toFixed(2)}`;
}

export function formatDuration(ms: number): string {
  if (ms >= 60000) return `${(ms / 60000).toFixed(1)}min`;
  return `${(ms / 1000).toFixed(1)}s`;
}

/** Human duration that scales from ms up to hours, for cumulative totals. */
export function formatDurationLong(ms: number): string {
  if (ms >= 3_600_000) return `${(ms / 3_600_000).toFixed(1)}h`;
  if (ms >= 60_000) return `${(ms / 60_000).toFixed(1)}min`;
  if (ms >= 1_000) return `${(ms / 1_000).toFixed(1)}s`;
  return `${Math.round(ms)}ms`;
}

export function formatDate(iso: string): string {
  return new Date(iso).toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

export function formatNumber(n: number): string {
  return n.toLocaleString(undefined, { maximumFractionDigits: 0 });
}

/** Compact count, e.g. 1.4K, 3.0M. */
export function formatCompact(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return formatNumber(n);
}

export function formatPercent(value: number, fractionDigits = 0): string {
  return `${value.toFixed(fractionDigits)}%`;
}

/** Ratio (0..1) → percentage string. */
export function formatRatioPercent(
  numerator: number,
  denominator: number,
  fractionDigits = 0,
): string {
  if (denominator <= 0) return "—";
  return formatPercent((numerator / denominator) * 100, fractionDigits);
}

/** Completion-token throughput (tokens/sec) given total tokens and ms. */
export function formatThroughput(tokens: number, durationMs: number): string {
  if (durationMs <= 0) return "—";
  const tps = tokens / (durationMs / 1000);
  if (tps >= 1000) return `${(tps / 1000).toFixed(1)}K tok/s`;
  return `${tps.toFixed(0)} tok/s`;
}

/** Relative "time ago" label from an ISO timestamp. */
export function formatRelativeTime(iso: string | null | undefined): string {
  if (!iso) return "—";
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return "—";
  const diffMs = Date.now() - then;
  const sec = Math.round(diffMs / 1000);
  if (sec < 60) return "just now";
  const min = Math.round(sec / 60);
  if (min < 60) return `${min}m ago`;
  const hr = Math.round(min / 60);
  if (hr < 24) return `${hr}h ago`;
  const day = Math.round(hr / 24);
  if (day < 30) return `${day}d ago`;
  return formatDate(iso);
}

export function shortId(id: string, length = 8): string {
  return id.length > length ? id.slice(0, length) : id;
}
