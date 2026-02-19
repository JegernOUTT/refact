import type { DateRange } from "../types";

export function dateRangeToApiArgs(dateRange: DateRange): {
  from?: string;
  to?: string;
} {
  if (dateRange.preset === "all") return {};
  const days = dateRange.preset === "7d" ? 7 : 30;
  const from = new Date(Date.now() - days * 24 * 60 * 60 * 1000)
    .toISOString()
    .slice(0, 10);
  return { from };
}
