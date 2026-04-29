import { store } from "../../app/store";
import { postBuddyErrorRequest } from "../../services/refact/buddy";

const REPORT_DEDUPE_MS = 10000;
const REPORT_TRIM_LEN = 4000;
const CRASH_STORAGE_KEY = "refact:buddy:frontend-crash:v1";
const CRASH_SESSION_VERSION = 1;
const CRASH_DETAIL_LEN = 500;
const CRASH_MAX_BREADCRUMBS = 12;
const CRASH_MAX_AGE_MS = 24 * 60 * 60 * 1000;

const recentReports = new Map<string, number>();
let crashSessionCache: BuddyCrashSession | null = null;

const SECRET_PATTERNS: [RegExp, string][] = [
  [/Bearer\s+[^\s"'`]+/gi, "Bearer [REDACTED]"],
  [/sk-[A-Za-z0-9]{20,}/g, "[REDACTED_SK_TOKEN]"],
  [/\bghp_[A-Za-z0-9]{10,}\b/g, "[REDACTED_GH_TOKEN]"],
  [/\bglpat-[A-Za-z0-9_-]{10,}\b/g, "[REDACTED_GL_TOKEN]"],
  [
    /\b(api[_-]?key|token|secret|password)\s*[:=]\s*[^\s,;]+/gi,
    "$1=[REDACTED]",
  ],
  [/(https?:\/\/[^\s?#]+)\?[^\s)\]]+/gi, "$1?[REDACTED]"],
  [/file:\/\/[^\s)\]]+/gi, "file://[REDACTED_PATH]"],
  [/[A-Za-z]:\\[^\s)\]]+/g, "[REDACTED_PATH]"],
  [/\/(?:Users|home)\/[^\s)\]]+/g, "[REDACTED_PATH]"],
];

export const BUDDY_FRONTEND_ERROR_NOISE_PATTERNS: RegExp[] = [
  /ResizeObserver loop (?:completed with undelivered notifications|limit exceeded)/i,
  /^Script error\.?$/i,
  /^AbortError:/i,
  /The (?:user aborted|operation was aborted)/i,
  /^Non-Error promise rejection captured with value: undefined$/i,
];

export function isBuddyFrontendErrorNoise(text: string): boolean {
  if (!text) return false;
  return BUDDY_FRONTEND_ERROR_NOISE_PATTERNS.some((pattern) =>
    pattern.test(text),
  );
}

export type BuddyFrontendErrorSource =
  | "window_error"
  | "unhandledrejection"
  | "react_error_boundary"
  | "react_root_render"
  | "react_recoverable"
  | "artifact_iframe"
  | "ui_error_state"
  | "mermaid_render"
  | "possible_renderer_crash";

type BuddyCrashHotSlot = "tool" | "report" | "reasoning" | "tasks";

type BuddyCrashBreadcrumb = {
  ts: number;
  label: string;
  detail: string;
};

type BuddyCrashSession = {
  version: number;
  sessionId: string;
  status: "running" | "closed";
  startedAt: number;
  updatedAt: number;
  closedAt?: number;
  host?: string;
  page?: string;
  chatId?: string;
  isStreaming?: boolean;
  visibility?: string;
  userAgent?: string;
  heapUsed?: number;
  heapLimit?: number;
  hot?: Partial<Record<BuddyCrashHotSlot, string>>;
  breadcrumbs: BuddyCrashBreadcrumb[];
};

type BuddyCrashContext = {
  host?: string;
  page?: string;
  chatId?: string;
  isStreaming?: boolean;
};

function clipText(text: string, maxLen: number): string {
  if (text.length <= maxLen) return text;
  return `${text.slice(0, maxLen - 1).trimEnd()}…`;
}

function currentVisibility(): string | undefined {
  if (typeof document === "undefined") return undefined;
  return document.visibilityState;
}

function currentUserAgent(): string | undefined {
  if (typeof navigator === "undefined") return undefined;
  return navigator.userAgent;
}

function currentMemory(): { heapUsed?: number; heapLimit?: number } {
  if (typeof performance === "undefined") return {};
  const perf = performance as Performance & {
    memory?: { usedJSHeapSize?: number; jsHeapSizeLimit?: number };
  };
  return {
    heapUsed:
      typeof perf.memory?.usedJSHeapSize === "number"
        ? perf.memory.usedJSHeapSize
        : undefined,
    heapLimit:
      typeof perf.memory?.jsHeapSizeLimit === "number"
        ? perf.memory.jsHeapSizeLimit
        : undefined,
  };
}

function storage(): Storage | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    return null;
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function finiteNumber(value: unknown): number | undefined {
  return typeof value === "number" && Number.isFinite(value)
    ? value
    : undefined;
}

function optionalString(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

function optionalBoolean(value: unknown): boolean | undefined {
  return typeof value === "boolean" ? value : undefined;
}

function parseCrashBreadcrumb(value: unknown): BuddyCrashBreadcrumb | null {
  if (!isRecord(value)) return null;
  const ts = finiteNumber(value.ts);
  const label = optionalString(value.label);
  const detail = optionalString(value.detail);
  if (ts === undefined || label === undefined || detail === undefined) {
    return null;
  }
  return { ts, label, detail };
}

function parseCrashHotSlots(
  value: unknown,
): Partial<Record<BuddyCrashHotSlot, string>> {
  if (!isRecord(value)) return {};
  return Object.fromEntries(
    Object.entries(value).filter(
      (entry): entry is [BuddyCrashHotSlot, string] => {
        const [key, slotValue] = entry;
        return (
          ["tool", "report", "reasoning", "tasks"].includes(key) &&
          typeof slotValue === "string"
        );
      },
    ),
  );
}

function readCrashSession(): BuddyCrashSession | null {
  if (crashSessionCache) return crashSessionCache;
  const handle = storage();
  if (!handle) return null;

  try {
    const raw = handle.getItem(CRASH_STORAGE_KEY);
    if (!raw) return null;
    const parsed: unknown = JSON.parse(raw);
    if (!isRecord(parsed)) return null;
    const startedAt = finiteNumber(parsed.startedAt);
    const updatedAt = finiteNumber(parsed.updatedAt);
    const sessionId = optionalString(parsed.sessionId);
    const status =
      parsed.status === "running" || parsed.status === "closed"
        ? parsed.status
        : undefined;
    if (
      parsed.version !== CRASH_SESSION_VERSION ||
      sessionId === undefined ||
      startedAt === undefined ||
      updatedAt === undefined ||
      status === undefined
    ) {
      return null;
    }

    const session: BuddyCrashSession = {
      version: CRASH_SESSION_VERSION,
      sessionId,
      status,
      startedAt,
      updatedAt,
      closedAt: finiteNumber(parsed.closedAt),
      host: optionalString(parsed.host),
      page: optionalString(parsed.page),
      chatId: optionalString(parsed.chatId),
      isStreaming: optionalBoolean(parsed.isStreaming),
      visibility: optionalString(parsed.visibility),
      userAgent: optionalString(parsed.userAgent),
      heapUsed: finiteNumber(parsed.heapUsed),
      heapLimit: finiteNumber(parsed.heapLimit),
      hot: parseCrashHotSlots(parsed.hot),
      breadcrumbs: Array.isArray(parsed.breadcrumbs)
        ? parsed.breadcrumbs
            .map(parseCrashBreadcrumb)
            .filter((entry): entry is BuddyCrashBreadcrumb => entry !== null)
            .slice(-CRASH_MAX_BREADCRUMBS)
        : [],
    };
    crashSessionCache = session;
    return session;
  } catch {
    return null;
  }
}

function writeCrashSession(session: BuddyCrashSession | null): void {
  crashSessionCache = session;
  const handle = storage();
  if (!handle) return;

  try {
    if (!session) {
      handle.removeItem(CRASH_STORAGE_KEY);
      return;
    }
    handle.setItem(CRASH_STORAGE_KEY, JSON.stringify(session));
  } catch {
    return;
  }
}

function sessionId(): string {
  if (
    typeof crypto !== "undefined" &&
    typeof crypto.randomUUID === "function"
  ) {
    return crypto.randomUUID();
  }
  return `crash-${Date.now()}-${Math.random().toString(16).slice(2, 10)}`;
}

function applyCrashContext(
  session: BuddyCrashSession,
  context: BuddyCrashContext,
): void {
  if (context.host !== undefined) {
    session.host = context.host || undefined;
  }
  if (context.page !== undefined) {
    session.page = context.page || undefined;
  }
  if (context.chatId !== undefined) {
    session.chatId = context.chatId || undefined;
  }
  if (context.isStreaming !== undefined) {
    session.isStreaming = context.isStreaming;
  }
  session.visibility = currentVisibility();
  session.userAgent = currentUserAgent();
  const memory = currentMemory();
  session.heapUsed = memory.heapUsed;
  session.heapLimit = memory.heapLimit;
  session.updatedAt = Date.now();
}

function pushCrashBreadcrumb(
  session: BuddyCrashSession,
  label: string,
  detail: string,
): void {
  const next = clipText(detail, CRASH_DETAIL_LEN);
  if (!next) return;
  const last = session.breadcrumbs.at(-1);
  const now = Date.now();
  if (last && last.label === label && last.detail === next) {
    last.ts = now;
    session.updatedAt = now;
    return;
  }

  session.breadcrumbs.push({ ts: now, label, detail: next });
  if (session.breadcrumbs.length > CRASH_MAX_BREADCRUMBS) {
    session.breadcrumbs.splice(
      0,
      session.breadcrumbs.length - CRASH_MAX_BREADCRUMBS,
    );
  }
  session.updatedAt = now;
}

function formatBytes(bytes?: number): string | null {
  if (typeof bytes !== "number" || !Number.isFinite(bytes) || bytes <= 0) {
    return null;
  }
  return `${Math.round(bytes / (1024 * 1024))} MiB`;
}

function recoverableCrashSession(
  session: BuddyCrashSession | null,
): BuddyCrashSession | null {
  if (!session) return null;
  if (session.status !== "running") return null;
  if (Date.now() - session.updatedAt > CRASH_MAX_AGE_MS) return null;
  return session;
}

export function beginBuddyCrashSession(
  context: BuddyCrashContext,
): BuddyCrashSession | null {
  const previous = recoverableCrashSession(readCrashSession());
  const now = Date.now();
  const next: BuddyCrashSession = {
    version: CRASH_SESSION_VERSION,
    sessionId: sessionId(),
    status: "running",
    startedAt: now,
    updatedAt: now,
    host: context.host,
    page: context.page,
    chatId: context.chatId,
    isStreaming: context.isStreaming,
    visibility: currentVisibility(),
    userAgent: currentUserAgent(),
    ...currentMemory(),
    hot: {},
    breadcrumbs: [],
  };
  pushCrashBreadcrumb(
    next,
    "session_start",
    `host=${context.host ?? "unknown"} page=${
      context.page ?? "unknown"
    } streaming=${String(Boolean(context.isStreaming))}`,
  );
  writeCrashSession(next);
  return previous ? { ...previous } : null;
}

export function touchBuddyCrashSession(context: BuddyCrashContext): void {
  const session = readCrashSession();
  if (!session || session.status !== "running") return;
  applyCrashContext(session, context);
  writeCrashSession(session);
}

export function closeBuddyCrashSession(reason = "pagehide"): void {
  const session = readCrashSession();
  if (!session) return;
  session.status = "closed";
  session.closedAt = Date.now();
  pushCrashBreadcrumb(session, "session_end", reason);
  writeCrashSession(session);
}

export function setBuddyCrashHotSlot(
  slot: BuddyCrashHotSlot,
  detail: string | null,
): void {
  const session = readCrashSession();
  if (!session || session.status !== "running") return;
  const hot = session.hot ?? {};
  const normalized = detail
    ? clipText(redactBuddyFrontendErrorText(detail).trim(), CRASH_DETAIL_LEN)
    : "";
  if (!normalized) {
    session.hot = Object.fromEntries(
      Object.entries(hot).filter(([key]) => key !== slot),
    );
  } else {
    hot[slot] = normalized;
    session.hot = hot;
  }
  session.updatedAt = Date.now();
  writeCrashSession(session);
}

export function addBuddyCrashBreadcrumb(label: string, detail: unknown): void {
  const session = readCrashSession();
  if (!session || session.status !== "running") return;
  const normalized = clipText(
    redactBuddyFrontendErrorText(errorToText(detail)).trim(),
    CRASH_DETAIL_LEN,
  );
  if (!normalized) return;
  pushCrashBreadcrumb(session, label, normalized);
  writeCrashSession(session);
}

export function buildBuddyCrashRecoveryError(
  session: BuddyCrashSession,
): string {
  const ageSeconds = Math.max(
    0,
    Math.round((Date.now() - session.updatedAt) / 1000),
  );
  const hotLines = Object.entries(session.hot ?? {})
    .filter(([, value]) => typeof value === "string" && value.length > 0)
    .map(([key, value]) => `- ${key}: ${value}`);
  const breadcrumbLines = session.breadcrumbs.map(
    (entry) =>
      `- ${new Date(entry.ts).toISOString()} ${entry.label}: ${entry.detail}`,
  );
  const heapUsed = formatBytes(session.heapUsed);
  const heapLimit = formatBytes(session.heapLimit);

  return [
    "Possible renderer crash/termination detected before the app restarted.",
    "Browser JavaScript cannot capture a native SIGILL/SIGKILL stack after the renderer dies, so this report contains the last persisted frontend breadcrumbs instead.",
    "",
    `Previous session id: ${session.sessionId}`,
    `Started at: ${new Date(session.startedAt).toISOString()}`,
    `Last heartbeat: ${new Date(
      session.updatedAt,
    ).toISOString()} (${ageSeconds}s before recovery)`,
    session.host ? `Host: ${session.host}` : "",
    session.page ? `Page: ${session.page}` : "",
    session.chatId ? `Chat ID: ${session.chatId}` : "",
    `Streaming: ${session.isStreaming ? "yes" : "no"}`,
    session.visibility ? `Visibility: ${session.visibility}` : "",
    heapUsed ?? heapLimit
      ? `JS heap: ${heapUsed ?? "unknown"}${heapLimit ? ` / ${heapLimit}` : ""}`
      : "",
    "",
    "Last hot-path state:",
    hotLines.length > 0 ? hotLines.join("\n") : "- none",
    "",
    "Recent breadcrumbs:",
    breadcrumbLines.length > 0 ? breadcrumbLines.join("\n") : "- none",
  ]
    .filter(Boolean)
    .join("\n");
}

function errorToText(error: unknown): string {
  if (error instanceof Error) {
    return error.stack ?? error.message;
  }
  if (typeof error === "string") return error;
  if (typeof error === "object" && error !== null) {
    if ("message" in error && typeof error.message === "string") {
      return error.message;
    }
    try {
      return JSON.stringify(error);
    } catch {
      return String(error);
    }
  }
  return String(error);
}

export function redactBuddyFrontendErrorText(text: string): string {
  return SECRET_PATTERNS.reduce(
    (current, [pattern, replacement]) => current.replace(pattern, replacement),
    text,
  );
}

export function buildBuddyFrontendErrorDedupeKey(
  args: {
    source: BuddyFrontendErrorSource;
    sourceFile?: string;
    toolName?: string;
    chatId?: string;
  },
  normalized: string,
): string {
  return [
    args.source,
    args.sourceFile ?? "",
    args.toolName ?? "",
    args.chatId ?? "",
    normalized.slice(0, 240),
  ].join("|");
}

export function resetBuddyFrontendErrorReportCache(): void {
  recentReports.clear();
  writeCrashSession(null);
}

function shouldReport(key: string, now: number): boolean {
  const previous = recentReports.get(key);
  if (previous && now - previous < REPORT_DEDUPE_MS) {
    return false;
  }

  recentReports.set(key, now);
  for (const [entry, timestamp] of recentReports) {
    if (now - timestamp > REPORT_DEDUPE_MS) {
      recentReports.delete(entry);
    }
  }
  return true;
}

type BuddyFrontendReporterState = {
  config: {
    apiKey: string | null;
    lspPort: number;
  };
};

type BuddyFrontendErrorDeps = {
  getState: () => BuddyFrontendReporterState;
  post: typeof postBuddyErrorRequest;
  now: () => number;
};

const defaultDeps: BuddyFrontendErrorDeps = {
  getState: () => store.getState() as BuddyFrontendReporterState,
  post: postBuddyErrorRequest,
  now: () => Date.now(),
};

export function installBuddyErrorReporter(): () => void {
  const onError = (e: ErrorEvent) => {
    void reportBuddyFrontendError({
      source: "window_error",
      error: e.error ?? e.message,
      sourceFile: e.filename || "frontend/window_error",
    });
  };
  const onRejection = (e: PromiseRejectionEvent) => {
    void reportBuddyFrontendError({
      source: "unhandledrejection",
      error: e.reason,
      sourceFile: "frontend/unhandledrejection",
    });
  };
  window.addEventListener("error", onError);
  window.addEventListener("unhandledrejection", onRejection);
  return () => {
    window.removeEventListener("error", onError);
    window.removeEventListener("unhandledrejection", onRejection);
  };
}

export async function reportBuddyFrontendError(
  args: {
    source: BuddyFrontendErrorSource;
    error: unknown;
    sourceFile?: string;
    toolName?: string;
    chatId?: string;
  },
  deps: BuddyFrontendErrorDeps = defaultDeps,
): Promise<void> {
  const state = deps.getState();
  const port = state.config.lspPort;
  if (!port) return;

  const apiKey = state.config.apiKey ?? undefined;
  const rawText = errorToText(args.error).trim();
  if (isBuddyFrontendErrorNoise(rawText)) return;
  const normalized = clipText(
    redactBuddyFrontendErrorText(rawText),
    REPORT_TRIM_LEN,
  );
  if (!normalized) return;

  const key = buildBuddyFrontendErrorDedupeKey(args, normalized);
  if (!shouldReport(key, deps.now())) return;

  try {
    await deps.post(port, apiKey, {
      error: `[frontend:${args.source}] ${normalized}`,
      source_file: args.sourceFile ?? `frontend/${args.source}`,
      tool_name: args.toolName ?? args.source,
      chat_id: args.chatId,
    });
  } catch {
    return;
  }
}
