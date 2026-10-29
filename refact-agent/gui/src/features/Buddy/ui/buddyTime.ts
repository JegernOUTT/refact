export type BuddyTimeInput = string | number | Date | null | undefined;

export interface BuddyTimeOptions {
  now?: number | Date;
  prefix?: string;
}

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;
const RELATIVE_DAYS = 7;
const LOCALE = "en-US";

const shortDateFormat = new Intl.DateTimeFormat(LOCALE, {
  month: "short",
  day: "numeric",
});
const shortDateYearFormat = new Intl.DateTimeFormat(LOCALE, {
  month: "short",
  day: "numeric",
  year: "numeric",
});
const timeFormat = new Intl.DateTimeFormat(LOCALE, {
  hour: "numeric",
  minute: "2-digit",
});

export function parseBuddyTime(value: BuddyTimeInput): number | null {
  if (value === null || value === undefined || value === "") return null;
  const time =
    value instanceof Date
      ? value.getTime()
      : typeof value === "number"
        ? value
        : Date.parse(value);
  return Number.isFinite(time) ? time : null;
}

function resolveNow(now: BuddyTimeOptions["now"]): number {
  if (now instanceof Date) return now.getTime();
  return now ?? Date.now();
}

function withPrefix(text: string, prefix: string | undefined): string {
  return prefix ? `${prefix} ${text}` : text;
}

export function formatBuddyDate(
  value: BuddyTimeInput,
  options: BuddyTimeOptions = {},
): string {
  const time = parseBuddyTime(value);
  if (time === null) return "";
  const date = new Date(time);
  const sameYear =
    date.getFullYear() === new Date(resolveNow(options.now)).getFullYear();
  return (sameYear ? shortDateFormat : shortDateYearFormat).format(date);
}

export function formatBuddyRelative(
  value: BuddyTimeInput,
  options: BuddyTimeOptions = {},
): string {
  const time = parseBuddyTime(value);
  if (time === null) return "";
  const diff = resolveNow(options.now) - time;
  const elapsed = Math.abs(diff);
  const minutes = Math.floor(elapsed / MINUTE);
  const hours = Math.floor(elapsed / HOUR);
  const days = Math.floor(elapsed / DAY);
  let text: string;
  if (minutes < 1) text = "now";
  else if (diff < 0) {
    if (minutes < 60) text = `in ${minutes}m`;
    else if (hours < 24) text = `in ${hours}h`;
    else text = formatBuddyDate(time, options);
  } else if (minutes < 60) text = `${minutes}m ago`;
  else if (hours < 24) text = `${hours}h ago`;
  else if (days < RELATIVE_DAYS) text = `${days}d ago`;
  else text = formatBuddyDate(time, options);
  return withPrefix(text, options.prefix);
}

export function formatBuddyAbsolute(value: BuddyTimeInput): string {
  const time = parseBuddyTime(value);
  if (time === null) return "";
  const date = new Date(time);
  return `${shortDateYearFormat.format(date)}, ${timeFormat.format(date)}`;
}

export function toBuddyIso(value: BuddyTimeInput): string | null {
  const time = parseBuddyTime(value);
  return time === null ? null : new Date(time).toISOString();
}

function sameMinute(a: number, b: number): boolean {
  return Math.floor(a / MINUTE) === Math.floor(b / MINUTE);
}

function sameLocalDay(a: number, b: number): boolean {
  const left = new Date(a);
  const right = new Date(b);
  return (
    left.getFullYear() === right.getFullYear() &&
    left.getMonth() === right.getMonth() &&
    left.getDate() === right.getDate()
  );
}

export function formatBuddyRange(
  start: BuddyTimeInput,
  end: BuddyTimeInput,
): string {
  const from = parseBuddyTime(start);
  const to = parseBuddyTime(end);
  if (from === null) return to === null ? "" : formatBuddyAbsolute(to);
  if (to === null || sameMinute(from, to)) return formatBuddyAbsolute(from);
  if (sameLocalDay(from, to)) {
    return `${formatBuddyAbsolute(from)} – ${timeFormat.format(new Date(to))}`;
  }
  return `${formatBuddyAbsolute(from)} – ${formatBuddyAbsolute(to)}`;
}
