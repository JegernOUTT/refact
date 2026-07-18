import { redactBuddyFrontendErrorText } from "../Buddy/reportBuddyFrontendError";

export type WebuiLogLevel = "error" | "warn" | "info" | "debug";

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
let captureInProgress = false;

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

function recordCapturedEntry(level: WebuiLogLevel, parts: unknown[]): void {
  captureInProgress = true;
  try {
    recordWebuiLogEntry(level, parts);
  } finally {
    captureInProgress = false;
  }
}

function formatWindowError(ev: ErrorEvent): string | null {
  if (!ev.message) return null;
  if (ev.filename && ev.lineno) {
    return `Uncaught: ${ev.message} (${ev.filename}:${ev.lineno})`;
  }
  return `Uncaught: ${ev.message}`;
}

export function installWebuiConsoleCapture(): () => void {
  if (uninstall) return uninstall;
  /* eslint-disable no-console -- this module intentionally wraps console to capture Web UI logs */
  const originalError = console.error;
  const originalWarn = console.warn;
  const originalLog = console.log;
  const originalInfo = console.info;
  const originalDebug = console.debug;

  const wrapConsole = (
    original: (...args: unknown[]) => void,
    level: WebuiLogLevel,
  ) => {
    return (...args: unknown[]) => {
      if (!captureInProgress) {
        recordCapturedEntry(level, args);
      }
      original.apply(console, args);
    };
  };

  const onWindowError = (ev: ErrorEvent) => {
    const message = formatWindowError(ev);
    if (message) recordCapturedEntry("error", [message]);
  };

  const onUnhandledRejection = (ev: PromiseRejectionEvent) => {
    recordCapturedEntry("error", [
      `Unhandled rejection: ${formatPart(ev.reason)}`,
    ]);
  };

  console.error = wrapConsole(originalError, "error");
  console.warn = wrapConsole(originalWarn, "warn");
  console.log = wrapConsole(originalLog, "info");
  console.info = wrapConsole(originalInfo, "info");
  console.debug = wrapConsole(originalDebug, "debug");
  window.addEventListener("error", onWindowError);
  window.addEventListener("unhandledrejection", onUnhandledRejection);

  uninstall = () => {
    console.error = originalError;
    console.warn = originalWarn;
    console.log = originalLog;
    console.info = originalInfo;
    console.debug = originalDebug;
    window.removeEventListener("error", onWindowError);
    window.removeEventListener("unhandledrejection", onUnhandledRejection);
    uninstall = null;
  };
  /* eslint-enable no-console */
  return uninstall;
}
