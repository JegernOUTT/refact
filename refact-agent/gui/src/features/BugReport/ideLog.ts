import { createAction } from "@reduxjs/toolkit";

export type IdeLogEntry = {
  at?: number;
  level: "error" | "warn" | "info" | "debug";
  message: string;
};

export const MAX_IDE_LOG_ENTRIES = 500;

let entries: IdeLogEntry[] = [];
const listeners = new Set<() => void>();

export const ideLogLines = createAction<{ lines: unknown }>("ide/logLines");

function notify(): void {
  listeners.forEach((listener) => listener());
}

function normalizeLevel(value: unknown): IdeLogEntry["level"] {
  if (typeof value !== "string") return "info";
  const level = value.toLowerCase();
  if (
    level === "error" ||
    level === "warn" ||
    level === "info" ||
    level === "debug"
  ) {
    return level;
  }
  return "info";
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

export function normalizeIdeLogLines(payload: unknown): IdeLogEntry[] {
  if (!Array.isArray(payload)) return [];
  return payload.flatMap((item): IdeLogEntry[] => {
    if (typeof item === "string") {
      return [{ level: "info", message: item }];
    }
    if (!isRecord(item) || typeof item.message !== "string") return [];
    const entry: IdeLogEntry = {
      level: normalizeLevel(item.level),
      message: item.message,
    };
    if (typeof item.at === "number") entry.at = item.at;
    return [entry];
  });
}

export function setIdeLogEntries(lines: IdeLogEntry[]): void {
  entries = lines.slice(-MAX_IDE_LOG_ENTRIES);
  notify();
}

export function getIdeLogEntries(): IdeLogEntry[] {
  return entries;
}

export function subscribeIdeLog(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function clearIdeLogEntries(): void {
  entries = [];
  notify();
}
