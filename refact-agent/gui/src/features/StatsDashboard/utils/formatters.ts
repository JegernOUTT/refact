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

export function formatCostDisplay(
  usd: number | null,
  coins: number | null,
): string {
  if (usd != null && usd > 0) return `$${usd.toFixed(2)}`;
  if (coins != null && coins > 0) return `${coins.toFixed(1)} coins`;
  if (usd != null) return `$${usd.toFixed(2)}`;
  if (coins != null) return `${coins.toFixed(1)} coins`;
  return "—";
}

export function formatDuration(ms: number): string {
  if (ms >= 60000) return `${(ms / 60000).toFixed(1)}min`;
  return `${(ms / 1000).toFixed(1)}s`;
}

export function formatDate(iso: string): string {
  return new Date(iso).toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}
