import { useEffect, useMemo, useSyncExternalStore } from "react";

import { ideRequestLogs } from "../../hooks/useEventBusForIDE";
import { usePostMessage } from "../../hooks/usePostMessage";
import {
  useGetBugReportErrorsQuery,
  useGetBugReportLogsQuery,
} from "../../services/refact/bugReport";
import { getIdeLogEntries, subscribeIdeLog, type IdeLogEntry } from "./ideLog";
import {
  getWebuiLogEntries,
  subscribeWebuiLog,
  type WebuiLogEntry,
} from "./webuiLog";

export type LogLevel = "error" | "warn" | "info" | "debug" | "unknown";
export type LevelFilter = "all" | "error" | "warn" | "info" | "debug";

export type LogLine = {
  text: string;
  level: LogLevel;
};

export type BugReportSourceKey = "daemon" | "engine" | "webui" | "ide";

export type BugReportSource = {
  key: BugReportSourceKey;
  label: string;
  available: boolean;
  exists: boolean;
  path?: string;
  readError?: string;
  lines: LogLine[];
  errorCount: number;
};

export type AggregatedError = {
  source: BugReportSourceKey;
  level: "error" | "warn";
  message: string;
  at?: number;
  count?: number;
  location?: string;
};

export const LOG_TAIL_LINES = 400;
export const MAX_AGGREGATED_ERRORS = 30;
const LOGS_POLL_MS = 2000;
const ERRORS_POLL_MS = 5000;

const LEVEL_TOKEN = /\b(ERROR|WARN(?:ING)?|INFO|DEBUG|TRACE)\b/;
const ENGINE_HEADER = /^\d{5,6}\.\d{1,3} (ERROR|WARN|INFO|DEBUG|TRACE)\b/;
const DAEMON_HEADER =
  /^\d{4}-\d{2}-\d{2}T[\d:.]+Z\s+(ERROR|WARN(?:ING)?|INFO|DEBUG|TRACE)\b/;
const IDEA_LOG_HEADER =
  /^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2},\d+ \[\s*\d+\]\s+(ERROR|WARN|INFO|DEBUG|TRACE)\b/;
const SIMPLE_TIME_HEADER = /^\d{2}:\d{2}:\d{2} (ERROR|WARN|INFO|DEBUG)\b/;

function levelFromToken(token: string): LogLevel {
  if (token === "ERROR") return "error";
  if (token.startsWith("WARN")) return "warn";
  if (token === "INFO") return "info";
  return "debug";
}

export function detectLineLevel(text: string): LogLevel {
  const positionalMatch =
    ENGINE_HEADER.exec(text) ??
    DAEMON_HEADER.exec(text) ??
    IDEA_LOG_HEADER.exec(text) ??
    SIMPLE_TIME_HEADER.exec(text);
  if (positionalMatch) return levelFromToken(positionalMatch[1]);
  const match = LEVEL_TOKEN.exec(text);
  if (!match) return "unknown";
  return levelFromToken(match[1]);
}

export function lineMatchesFilter(
  line: LogLine,
  filter: string,
  level: LevelFilter,
): boolean {
  if (level !== "all" && line.level !== level) return false;
  if (filter) {
    return line.text.toLowerCase().includes(filter.toLowerCase());
  }
  return true;
}

export function toLogLines(lines: string[]): LogLine[] {
  let previousLevel: LogLevel = "unknown";
  return lines.map((text) => {
    const detectedLevel = detectLineLevel(text);
    const level =
      detectedLevel === "unknown" && previousLevel !== "unknown"
        ? previousLevel
        : detectedLevel;
    previousLevel = level;
    return { text, level };
  });
}

function formatTime(at: number): string {
  return new Date(at).toTimeString().slice(0, 8);
}

function formatWebuiEntry(entry: WebuiLogEntry): LogLine {
  return {
    text: `${formatTime(entry.at)} ${entry.level.toUpperCase()} ${
      entry.message
    }`,
    level: entry.level,
  };
}

function formatIdeEntry(entry: IdeLogEntry): LogLine {
  const text =
    entry.at === undefined
      ? `${entry.level.toUpperCase()} ${entry.message}`
      : `${formatTime(entry.at)} ${entry.level.toUpperCase()} ${entry.message}`;
  return { text, level: entry.level };
}

function countErrors(lines: LogLine[]): number {
  return lines.reduce(
    (count, line) => (line.level === "error" ? count + 1 : count),
    0,
  );
}

function normalizeBackendSource(source: string): BugReportSourceKey {
  return source === "daemon" ? "daemon" : "engine";
}

function isIdeAvailable(host: string): boolean {
  return host === "vscode" || host === "jetbrains" || host === "ide";
}

export function useBugReportSources(
  paused: boolean,
  host: string,
): {
  sources: BugReportSource[];
  aggregatedErrors: AggregatedError[];
  webuiLines: string[];
  ideLines: string[];
} {
  const pollingLogs = paused ? 0 : LOGS_POLL_MS;
  const pollingErrors = paused ? 0 : ERRORS_POLL_MS;
  const postMessage = usePostMessage();
  const ideAvailable = isIdeAvailable(host);
  const engineLogs = useGetBugReportLogsQuery(
    { source: "engine", tail: LOG_TAIL_LINES },
    { pollingInterval: pollingLogs },
  );
  const daemonLogs = useGetBugReportLogsQuery(
    { source: "daemon", tail: LOG_TAIL_LINES },
    { pollingInterval: pollingLogs },
  );
  const backendErrors = useGetBugReportErrorsQuery(undefined, {
    pollingInterval: pollingErrors,
  });
  const webuiEntries = useSyncExternalStore(
    subscribeWebuiLog,
    getWebuiLogEntries,
    getWebuiLogEntries,
  );
  const ideEntries = useSyncExternalStore(
    subscribeIdeLog,
    getIdeLogEntries,
    getIdeLogEntries,
  );

  useEffect(() => {
    if (!ideAvailable || paused) return undefined;
    const requestLogs = () => {
      postMessage(ideRequestLogs({ limit: LOG_TAIL_LINES }));
    };
    requestLogs();
    const interval = window.setInterval(requestLogs, LOGS_POLL_MS);
    return () => window.clearInterval(interval);
  }, [ideAvailable, paused, postMessage]);

  const sources = useMemo<BugReportSource[]>(() => {
    const daemonLines = toLogLines(daemonLogs.data?.lines ?? []);
    const engineLines = toLogLines(engineLogs.data?.lines ?? []);
    const webuiLogLines = webuiEntries.map(formatWebuiEntry);
    const ideLogLines = ideAvailable ? ideEntries.map(formatIdeEntry) : [];
    return [
      {
        key: "daemon",
        label: "Daemon",
        available: true,
        exists: daemonLogs.data?.exists ?? false,
        path: daemonLogs.data?.path,
        readError: daemonLogs.data?.read_error,
        lines: daemonLines,
        errorCount: countErrors(daemonLines),
      },
      {
        key: "engine",
        label: "Engine",
        available: true,
        exists: engineLogs.data?.exists ?? false,
        path: engineLogs.data?.path,
        readError: engineLogs.data?.read_error,
        lines: engineLines,
        errorCount: countErrors(engineLines),
      },
      {
        key: "webui",
        label: "Web UI",
        available: true,
        exists: true,
        lines: webuiLogLines,
        errorCount: countErrors(webuiLogLines),
      },
      {
        key: "ide",
        label: "IDE",
        available: ideAvailable,
        exists: ideAvailable,
        lines: ideLogLines,
        errorCount: countErrors(ideLogLines),
      },
    ];
  }, [
    daemonLogs.data,
    engineLogs.data,
    ideAvailable,
    ideEntries,
    webuiEntries,
  ]);

  const aggregatedErrors = useMemo<AggregatedError[]>(() => {
    const backend = (backendErrors.data?.errors ?? []).map(
      (entry): AggregatedError => ({
        source: normalizeBackendSource(entry.source),
        level: entry.level === "warn" ? "warn" : "error",
        message: entry.message,
        count: entry.count,
        location: entry.location,
      }),
    );
    const webui = [...webuiEntries]
      .reverse()
      .filter((entry) => entry.level === "error" || entry.level === "warn")
      .map(
        (entry): AggregatedError => ({
          source: "webui",
          level: entry.level === "warn" ? "warn" : "error",
          message: entry.message,
          at: entry.at,
        }),
      );
    const ide = ideAvailable
      ? [...ideEntries]
          .reverse()
          .filter((entry) => entry.level === "error" || entry.level === "warn")
          .map(
            (entry): AggregatedError => ({
              source: "ide",
              level: entry.level === "warn" ? "warn" : "error",
              message: entry.message,
              at: entry.at,
            }),
          )
      : [];
    return [...backend, ...webui, ...ide].slice(0, MAX_AGGREGATED_ERRORS);
  }, [backendErrors.data, ideAvailable, ideEntries, webuiEntries]);

  const webuiLines = useMemo(
    () => webuiEntries.map((entry) => formatWebuiEntry(entry).text),
    [webuiEntries],
  );
  const ideLines = useMemo(
    () =>
      ideAvailable ? ideEntries.map((entry) => formatIdeEntry(entry).text) : [],
    [ideAvailable, ideEntries],
  );

  return { sources, aggregatedErrors, webuiLines, ideLines };
}
