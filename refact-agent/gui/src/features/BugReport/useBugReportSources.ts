import { useMemo, useSyncExternalStore } from "react";

import {
  useGetBugReportErrorsQuery,
  useGetBugReportLogsQuery,
} from "../../services/refact/bugReport";
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
};

export const LOG_TAIL_LINES = 400;
export const MAX_AGGREGATED_ERRORS = 30;
const LOGS_POLL_MS = 2000;
const ERRORS_POLL_MS = 5000;

const LEVEL_TOKEN = /\b(ERROR|WARN(?:ING)?|INFO|DEBUG|TRACE)\b/;

export function detectLineLevel(text: string): LogLevel {
  const match = LEVEL_TOKEN.exec(text);
  if (!match) return "unknown";
  const token = match[1];
  if (token === "ERROR") return "error";
  if (token.startsWith("WARN")) return "warn";
  if (token === "INFO") return "info";
  return "debug";
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

function toLogLines(lines: string[]): LogLine[] {
  return lines.map((text) => ({ text, level: detectLineLevel(text) }));
}

function formatWebuiEntry(entry: WebuiLogEntry): LogLine {
  const time = new Date(entry.at).toTimeString().slice(0, 8);
  return {
    text: `${time} ${entry.level.toUpperCase()} ${entry.message}`,
    level: entry.level,
  };
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

export function useBugReportSources(paused: boolean): {
  sources: BugReportSource[];
  aggregatedErrors: AggregatedError[];
  webuiLines: string[];
} {
  const pollingLogs = paused ? 0 : LOGS_POLL_MS;
  const pollingErrors = paused ? 0 : ERRORS_POLL_MS;
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

  const sources = useMemo<BugReportSource[]>(() => {
    const daemonLines = toLogLines(daemonLogs.data?.lines ?? []);
    const engineLines = toLogLines(engineLogs.data?.lines ?? []);
    const webuiLines = webuiEntries.map(formatWebuiEntry);
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
        lines: webuiLines,
        errorCount: countErrors(webuiLines),
      },
      {
        key: "ide",
        label: "IDE",
        available: false,
        exists: false,
        lines: [],
        errorCount: 0,
      },
    ];
  }, [daemonLogs.data, engineLogs.data, webuiEntries]);

  const aggregatedErrors = useMemo<AggregatedError[]>(() => {
    const backend = (backendErrors.data?.errors ?? []).map(
      (entry): AggregatedError => ({
        source: normalizeBackendSource(entry.source),
        level: entry.level === "warn" ? "warn" : "error",
        message: entry.message,
      }),
    );
    const webui = [...webuiEntries]
      .reverse()
      .filter((entry) => entry.level === "error")
      .map(
        (entry): AggregatedError => ({
          source: "webui",
          level: "error",
          message: entry.message,
          at: entry.at,
        }),
      );
    return [...backend, ...webui].slice(0, MAX_AGGREGATED_ERRORS);
  }, [backendErrors.data, webuiEntries]);

  const webuiLines = useMemo(
    () => webuiEntries.map((entry) => formatWebuiEntry(entry).text),
    [webuiEntries],
  );

  return { sources, aggregatedErrors, webuiLines };
}
