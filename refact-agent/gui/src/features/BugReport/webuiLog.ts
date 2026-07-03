import { redactBuddyFrontendErrorText } from "../Buddy/reportBuddyFrontendError";

export type WebuiLogLevel = "error" | "warn";

export type WebuiLogEntry = {
  at: number;
  level: WebuiLogLevel;
  message: string;
};

export const MAX_WEBUI_LOG_ENTRIES = 500;
const MAX_MESSAGE_CHARS = 600;

let entries: WebuiLogEntry[] = [];
const listeners = new Set<() => void>();
let uninstall: (() => void) | null = null;

function formatPart(part: unknown): string {
  if (typeof part === "string") return part;
  if (part instanceof Error) return `${part.name}: ${part.message}`;
  try {
    return JSON.stringify(part);
  } catch {
    return String(part);
  }
}

function notify(): void {
  listeners.forEach((listener) => listener());
}

export function recordWebuiLogEntry(
  level: WebuiLogLevel,
  parts: unknown[],
): void {
  const raw = parts.map(formatPart).join(" ").trim();
  if (!raw) return;
  const message = redactBuddyFrontendErrorText(raw).slice(0, MAX_MESSAGE_CHARS);
  entries = [
    ...entries.slice(-(MAX_WEBUI_LOG_ENTRIES - 1)),
    { at: Date.now(), level, message },
  ];
  notify();
}

export function getWebuiLogEntries(): WebuiLogEntry[] {
  return entries;
}

export function clearWebuiLogEntries(): void {
  entries = [];
  notify();
}

export function subscribeWebuiLog(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function installWebuiConsoleCapture(): () => void {
  if (uninstall) return uninstall;
  /* eslint-disable no-console -- this module intentionally wraps console to capture Web UI logs */
  const originalError = console.error.bind(console);
  const originalWarn = console.warn.bind(console);
  console.error = (...args: unknown[]) => {
    recordWebuiLogEntry("error", args);
    originalError(...args);
  };
  console.warn = (...args: unknown[]) => {
    recordWebuiLogEntry("warn", args);
    originalWarn(...args);
  };
  uninstall = () => {
    console.error = originalError;
    console.warn = originalWarn;
    uninstall = null;
  };
  /* eslint-enable no-console */
  return uninstall;
}
