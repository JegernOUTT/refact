import type { DateRange, DateRangePreset } from "../types";

const PRESET_DAYS: Record<
  Exclude<DateRangePreset, "all" | "custom" | "today">,
  number
> = {
  "7d": 7,
  "30d": 30,
  "90d": 90,
};

function toIsoDate(d: Date): string {
  return d.toISOString().slice(0, 10);
}

/** Date string (YYYY-MM-DD) `days` days ago from now. */
export function daysAgoIsoDate(days: number): string {
  return toIsoDate(new Date(Date.now() - days * 24 * 60 * 60 * 1000));
}

export function todayIsoDate(): string {
  return toIsoDate(new Date());
}

/**
 * Convert the dashboard date range into the `from`/`to` query the stats API
 * understands. The backend filters by calendar day, so all values are
 * `YYYY-MM-DD` strings.
 */
export function dateRangeToApiArgs(dateRange: DateRange): {
  from?: string;
  to?: string;
} {
  if (dateRange.preset === "all") return {};
  if (dateRange.preset === "custom") {
    const args: { from?: string; to?: string } = {};
    if (dateRange.from) args.from = dateRange.from;
    if (dateRange.to) args.to = dateRange.to;
    return args;
  }
  if (dateRange.preset === "today") {
    const today = todayIsoDate();
    return { from: today, to: today };
  }
  // Backend filters inclusively by calendar day, so "last N days" (today plus
  // the prior N-1 days) starts N-1 days ago.
  return { from: daysAgoIsoDate(PRESET_DAYS[dateRange.preset] - 1) };
}

/** Number of whole days the range covers, for averaging (min 1). */
export function dateRangeSpanDays(
  dateRange: DateRange,
  fallbackActiveDays: number,
): number {
  if (dateRange.preset === "all") {
    return Math.max(1, fallbackActiveDays);
  }
  if (dateRange.preset === "today") return 1;
  if (dateRange.preset === "custom") {
    const from = dateRange.from ? new Date(dateRange.from) : null;
    const to = dateRange.to ? new Date(dateRange.to) : new Date();
    if (from && !Number.isNaN(from.getTime()) && !Number.isNaN(to.getTime())) {
      const diff = Math.round(
        (to.getTime() - from.getTime()) / (24 * 60 * 60 * 1000),
      );
      return Math.max(1, diff + 1);
    }
    return Math.max(1, fallbackActiveDays);
  }
  return PRESET_DAYS[dateRange.preset];
}

export function describeDateRange(dateRange: DateRange): string {
  switch (dateRange.preset) {
    case "all":
      return "All time";
    case "today":
      return "Today";
    case "custom":
      if (dateRange.from && dateRange.to)
        return `${dateRange.from} → ${dateRange.to}`;
      if (dateRange.from) return `Since ${dateRange.from}`;
      if (dateRange.to) return `Until ${dateRange.to}`;
      return "Custom range";
    case "7d":
      return "Last 7 days";
    case "30d":
      return "Last 30 days";
    case "90d":
      return "Last 90 days";
  }
}
