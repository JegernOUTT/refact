export declare function computeXpFill(xp: number, xpNext: number): number;
export declare function formatBuddyTime(ts: string | null | undefined): string;
export declare function formatFailureLabel(value: string | null | undefined): string | null;
/**
 * Format a large integer count (tokens, messages, …) using compact unit
 * suffixes (k, M, B, T). Picks the largest unit that keeps the value
 * above 1 so very large totals (8_130_081_100) render as "8.1B" rather
 * than the broken "8130081.1k" produced by a fixed `/1000` formatter.
 */
export declare function formatCompactNumber(value: number): string;
