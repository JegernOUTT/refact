export function computeXpFill(xp: number, xpNext: number): number {
  if (xpNext <= 0) return 100;
  return Math.min(100, Math.max(0, (xp / xpNext) * 100));
}

/**
 * Format a large integer count (tokens, messages, …) using compact unit
 * suffixes (k, M, B, T). Picks the largest unit that keeps the value
 * above 1 so very large totals (8_130_081_100) render as "8.1B" rather
 * than the broken "8130081.1k" produced by a fixed `/1000` formatter.
 */
export function formatCompactNumber(value: number): string {
  if (!Number.isFinite(value)) return "0";
  const abs = Math.abs(value);
  const sign = value < 0 ? "-" : "";
  if (abs < 1000) return `${sign}${abs}`;
  const units: { threshold: number; suffix: string }[] = [
    { threshold: 1e12, suffix: "T" },
    { threshold: 1e9, suffix: "B" },
    { threshold: 1e6, suffix: "M" },
    { threshold: 1e3, suffix: "k" },
  ];
  for (const u of units) {
    if (abs >= u.threshold) {
      const scaled = abs / u.threshold;
      const formatted =
        scaled >= 100
          ? Math.round(scaled).toString()
          : scaled.toFixed(1).replace(/\.0$/, "");
      return `${sign}${formatted}${u.suffix}`;
    }
  }
  return `${sign}${abs}`;
}
