/**
 * Pure display helpers for the `python` tool card, its nested calls and the
 * approval bar. Inputs are untrusted engine strings: everything here returns
 * plain text for React text nodes, never HTML.
 */

import {
  isDiffChunk,
  type DiffChunk,
  type DiffMessage,
  type PythonCallRecord,
  type PythonRunExtra,
} from "../../../services/refact/types";
import { formatBytes } from "../../../features/Performance/performanceFormatters";
import { useEffect, useState } from "react";
import {
  Ban,
  CircleCheck,
  CircleHelp,
  CircleSlash,
  CircleX,
  Hourglass,
  LoaderCircle,
  type LucideIcon,
} from "lucide-react";
import type { IconProps } from "../../ui";
import { useShiki } from "../../../hooks/useShiki";
import { useAppearance } from "../../../hooks/useAppearance";
import type { TranscriptStatus } from "../TranscriptItem/statusVocabulary";
import { extractCodeLines } from "./editToolHighlight";
import type { PythonWorkspace } from "../../../services/refact/pythonWorkspace";

type PythonWorkspaceRun = PythonWorkspace["runs"][number];

function isLiveRun(run: PythonWorkspaceRun) {
  return (
    run.finished_at_ms === null &&
    (run.status === "running" || run.status === "background")
  );
}
import { isOversizedDiff } from "./diffPreview";

export type PythonArgs = Record<string, unknown>;

const SUMMARY_KEYS = [
  "description",
  "command",
  "path",
  "paths",
  "file_path",
  "pattern",
  "query",
  "url",
  "symbols",
] as const;

const PATH_KEYS = new Set(["path", "paths", "file_path", "workdir", "cwd"]);

export const SUMMARY_MAX_CHARS = 80;

/** Parses a bounded `args_preview`; returns null unless it is a JSON object. */
export function parsePythonArgs(text: string): PythonArgs | null {
  const trimmed = text.trim();
  if (!trimmed.startsWith("{")) return null;
  try {
    const value: unknown = JSON.parse(trimmed);
    return isArgsObject(value) ? value : null;
  } catch {
    return null;
  }
}

export const CUT_MARK = "…";

/**
 * Best-effort reading of a JSON object whose tail was cut off by a byte
 * bound (the engine's older `args_preview` form). Complete members are kept;
 * a string cut mid-way is closed and ends with `…`. Returns null when nothing
 * useful can be recovered.
 */
export function parseCutPythonArgs(text: string): PythonArgs | null {
  const source = text.trim();
  if (!source.startsWith("{")) return null;
  const closers: string[] = [];
  let inString = false;
  let escaped = false;
  for (const char of source) {
    if (inString) {
      if (escaped) escaped = false;
      else if (char === "\\") escaped = true;
      else if (char === '"') inString = false;
    } else if (char === '"') inString = true;
    else if (char === "{") closers.push("}");
    else if (char === "[") closers.push("]");
    else if (char === "}" || char === "]") closers.pop();
  }
  let repaired = source;
  if (inString) {
    if (escaped) repaired = repaired.slice(0, -1);
    repaired = repaired.replace(/\\u[0-9a-fA-F]{0,3}$/, "");
    repaired += `${CUT_MARK}"`;
  }
  const close = (body: string) => `${body}${[...closers].reverse().join("")}`;
  const attempts = [
    repaired,
    // A dangling key, colon or comma after the last complete member.
    repaired.replace(/,\s*"(?:[^"\\]|\\.)*"\s*:?\s*$/, ""),
    repaired.replace(/,\s*$/, ""),
    repaired.replace(/,?\s*"(?:[^"\\]|\\.)*"\s*:\s*[^,{}[\]"]*$/, ""),
  ];
  for (const attempt of attempts) {
    try {
      const value: unknown = JSON.parse(close(attempt));
      if (isArgsObject(value) && Object.keys(value).length > 0) return value;
    } catch {
      /* try the next repair */
    }
  }
  return null;
}

/** Valid JSON first, then a repaired cut object. */
export function readPythonArgs(text: string): PythonArgs | null {
  return parsePythonArgs(text) ?? parseCutPythonArgs(text);
}

function isArgsObject(value: unknown): value is PythonArgs {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function isPathKey(key: string): boolean {
  return PATH_KEYS.has(key);
}

export type WorkspaceRoots = string | readonly string[] | undefined;

function normalizeSeparators(path: string) {
  return path.replace(/\\/g, "/");
}

/**
 * Shows a path relative to the first workspace root that contains it.
 * Windows separators match either way, drive letters case-insensitively;
 * the original separators of the remainder are kept.
 */
export function relativeToRoot(path: string, roots: WorkspaceRoots) {
  const list = typeof roots === "string" ? [roots] : roots ?? [];
  const normalizedPath = normalizeSeparators(path);
  const windows = /^[a-zA-Z]:\//.test(normalizedPath);
  const comparable = (text: string) => (windows ? text.toLowerCase() : text);
  let best: string | undefined;
  for (const root of list) {
    if (!root) continue;
    const base = normalizeSeparators(root).replace(/\/+$/, "");
    if (!base) continue;
    if (comparable(normalizedPath) === comparable(base)) return ".";
    if (
      comparable(normalizedPath).startsWith(`${comparable(base)}/`) &&
      (best === undefined || base.length > best.length)
    )
      best = base;
  }
  return best === undefined ? path : path.slice(best.length + 1);
}

/**
 * A `paths` value may hold several comma-separated paths, but a single path
 * may itself contain a comma. Split only when every part looks like a path of
 * its own: all absolute when the first one is, else all with a separator or an
 * extension.
 */
export function splitPathList(value: string): string[] {
  const parts = value.split(",").map((part) => part.trim());
  if (parts.length < 2) return [value];
  const absolute = (part: string) => /^([a-zA-Z]:)?[\\/~]/.test(part);
  const relative = (part: string) => /[\\/]|\.[\w-]+$/.test(part);
  const ok = absolute(parts[0])
    ? parts.every(absolute)
    : parts.every((part) => part.length > 0 && relative(part));
  return ok ? parts : [value];
}

function relativePaths(value: string, roots: WorkspaceRoots) {
  return splitPathList(value)
    .map((part) => relativeToRoot(part, roots))
    .join(", ");
}

export function truncateText(text: string, max: number): string {
  const chars = Array.from(text);
  return chars.length > max ? `${chars.slice(0, max - 1).join("")}…` : text;
}

function oneLine(text: string) {
  return text.replace(/\s+/g, " ").trim();
}

function summaryValue(
  key: string,
  value: unknown,
  root: WorkspaceRoots,
): string | undefined {
  if (typeof value === "string" && value.trim()) {
    return isPathKey(key) ? relativePaths(value, root) : value;
  }
  if (Array.isArray(value)) {
    const items = value.filter(
      (item): item is string => typeof item === "string",
    );
    if (items.length === 0) return undefined;
    return items
      .map((item) => (isPathKey(key) ? relativeToRoot(item, root) : item))
      .join(", ");
  }
  return undefined;
}

/**
 * One-line argument summary for a nested call row: prefers command, then
 * paths, pattern, query, url and symbols, else the first string value. Never
 * returns raw JSON.
 */
export function summarizePythonArgs(
  argsPreview: string,
  root?: WorkspaceRoots,
): string {
  const args = readPythonArgs(argsPreview);
  let summary: string | undefined;
  if (args) {
    for (const key of SUMMARY_KEYS) {
      summary = summaryValue(key, args[key], root);
      if (summary) break;
    }
    if (!summary) {
      for (const [key, value] of Object.entries(args)) {
        summary = summaryValue(key, value, root);
        if (summary) break;
      }
    }
  } else if (!argsPreview.trim().startsWith("{")) {
    summary = argsPreview;
  }
  return truncateText(oneLine(summary ?? ""), SUMMARY_MAX_CHARS);
}

/** Orders argument entries so the command and paths come first. */
export function orderedArgEntries(args: PythonArgs): [string, unknown][] {
  const entries = Object.entries(args);
  const rank = (key: string) => {
    const index = (SUMMARY_KEYS as readonly string[]).indexOf(key);
    return index === -1 ? SUMMARY_KEYS.length : index;
  };
  return entries
    .map((entry, index) => ({ entry, index }))
    .sort((a, b) => rank(a.entry[0]) - rank(b.entry[0]) || a.index - b.index)
    .map(({ entry }) => entry);
}

export function formatArgValue(value: unknown): string {
  if (typeof value === "string") return value;
  if (value === undefined) return "";
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

/** The command line a request really runs, or undefined for other tools. */
export function approvalCommand(
  command: string | null,
  toolName: string,
  args: PythonArgs | null,
): string | undefined {
  if (command?.trim() && command !== toolName) return command;
  const fromArgs = args?.command;
  return typeof fromArgs === "string" && fromArgs.trim() ? fromArgs : undefined;
}

/**
 * Re-indents a JSON object or array without re-serializing it: the original
 * tokens (numbers beyond 2^53, duplicate keys, key order, escapes) are kept
 * verbatim and only whitespace between tokens changes. Returns undefined
 * when the text is not a JSON object or array.
 */
export function prettyJson(text: string): string | undefined {
  const trimmed = text.trim();
  if (!trimmed.startsWith("{") && !trimmed.startsWith("[")) return undefined;
  try {
    JSON.parse(trimmed);
  } catch {
    return undefined;
  }
  let out = "";
  let depth = 0;
  let inString = false;
  let escaped = false;
  const newline = () => `\n${"  ".repeat(depth)}`;
  for (let i = 0; i < trimmed.length; i++) {
    const char = trimmed[i];
    if (inString) {
      out += char;
      if (escaped) escaped = false;
      else if (char === "\\") escaped = true;
      else if (char === '"') inString = false;
      continue;
    }
    if (/\s/.test(char)) continue;
    if (char === '"') {
      inString = true;
      out += char;
    } else if (char === "{" || char === "[") {
      const close = char === "{" ? "}" : "]";
      let next = i + 1;
      while (next < trimmed.length && /\s/.test(trimmed[next])) next++;
      if (trimmed[next] === close) {
        out += char + close;
        i = next;
        continue;
      }
      depth++;
      out += char + newline();
    } else if (char === "}" || char === "]") {
      depth--;
      out += newline() + char;
    } else if (char === ",") {
      out += char + newline();
    } else if (char === ":") {
      out += ": ";
    } else {
      out += char;
    }
  }
  return out;
}

export type EditPair = { before: string; after: string };

const EDIT_PAIRS: readonly (readonly [string, string])[] = [
  ["old_str", "replacement"],
  ["old_str", "new_str"],
  ["old_string", "new_string"],
  ["old_text", "new_text"],
  ["search", "replace"],
];

/** A complete (uncut) before/after pair of an edit tool, if the preview has one. */
export function editPair(args: PythonArgs | null): EditPair | undefined {
  if (!args) return undefined;
  for (const [beforeKey, afterKey] of EDIT_PAIRS) {
    const before = args[beforeKey];
    const after = args[afterKey];
    if (typeof before !== "string" || typeof after !== "string") continue;
    if (isCutValue(before) || isCutValue(after)) return undefined;
    return { before, after };
  }
  return undefined;
}

export const EDIT_PAIR_KEYS = new Set(EDIT_PAIRS.flat());

/** Engine previews mark a cut value with "… (+N chars)" or a trailing "…". */
export function isCutValue(value: string): boolean {
  return /… \(\+\d+ chars\)$/.test(value) || value.endsWith(CUT_MARK);
}

/** Splits text into its first `limit` lines and the total line count. */
export function clampLines(text: string, limit: number) {
  const lines = text.replace(/\n$/, "").split("\n");
  return {
    total: lines.length,
    clipped: lines.length > limit,
    head: lines.slice(0, limit).join("\n"),
    full: lines.join("\n"),
  };
}

/** One duration format for call rows, the card header and the workspace panel. */
export function formatCallDuration(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)} ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)} s`;
  const seconds = Math.round(ms / 1000);
  if (seconds < 3600)
    return `${Math.floor(seconds / 60)} min ${seconds % 60} s`;
  const minutes = Math.floor(seconds / 60);
  return `${Math.floor(minutes / 60)} h ${minutes % 60} min`;
}

const ROLLBACK_NOTE =
  "the session was restored to the state saved before this run; variables set in this run are lost";
const SESSION_LOST_NOTE =
  "the session was lost; variables from earlier runs are gone";

function sentence(text: string) {
  const trimmed = text.trim();
  if (!trimmed) return trimmed;
  const capital = trimmed.charAt(0).toUpperCase() + trimmed.slice(1);
  return /[.!?]$/.test(capital) ? capital : `${capital}.`;
}

/** "Output limit" from the engine's `<Kind>Limit` error type. */
function limitName(errorType: string | undefined): string {
  const kind = /^([A-Za-z]+)Limit$/.exec(errorType ?? "")?.[1];
  if (!kind) return "Run limit";
  const words = kind.replace(/([a-z])([A-Z])/g, "$1 $2").toLowerCase();
  return `${words.charAt(0).toUpperCase()}${words.slice(1)} limit`;
}

/**
 * One sentence for a limit run: the specific limit, then the engine's own
 * words about the session (its leading "run limit exceeded:" replaced).
 */
export function limitSentence(extra: PythonRunExtra): string {
  const head = `${limitName(extra.error?.type)} exceeded`;
  const engine = (extra.state.note ?? extra.state.reset_reason ?? "").trim();
  const rest = engine.replace(/^run limit exceeded\s*[:.;,-]?\s*/i, "");
  const tail = rest
    ? rest
    : extra.error?.type === "MemoryError"
      ? SESSION_LOST_NOTE
      : ROLLBACK_NOTE;
  return sentence(`${head}: ${tail.charAt(0).toLowerCase()}${tail.slice(1)}`);
}

/** Session facts after a run: restored, saved size, reset reason. */
export function sessionSummary(extra: PythonRunExtra): string {
  const state = extra.state;
  const parts: string[] = [];
  if (state.restored) parts.push("Restored from disk");
  parts.push(
    state.persisted ? `Saved · ${formatBytes(state.bytes)}` : "Not saved",
  );
  if (state.reset_reason && extra.status !== "limit_exceeded")
    parts.push(`Reset: ${state.reset_reason}`);
  return parts.join(" · ");
}

/** One presentation of a script run status for the card, call rows and the panel. */
export type ScriptStatusTone =
  | "accent"
  | "warning"
  | "success"
  | "danger"
  | "muted";

export type ScriptStatusView = {
  label: string;
  tone: ScriptStatusTone;
  transcript: TranscriptStatus;
};

const SCRIPT_STATUS: Record<string, ScriptStatusView | undefined> = {
  running: { label: "Running", tone: "accent", transcript: "running" },
  background: { label: "Background", tone: "accent", transcript: "running" },
  waiting: {
    label: "Waiting for approval",
    tone: "warning",
    transcript: "needs_you",
  },
  completed: { label: "Completed", tone: "success", transcript: "done" },
  finished: { label: "Completed", tone: "success", transcript: "done" },
  failed: { label: "Failed", tone: "danger", transcript: "failed" },
  cancelled: { label: "Stopped", tone: "muted", transcript: "cancelled" },
  interrupted: { label: "Stopped", tone: "muted", transcript: "cancelled" },
  timed_out: { label: "Timed out", tone: "danger", transcript: "failed" },
  limit_exceeded: {
    label: "Limit exceeded",
    tone: "danger",
    transcript: "failed",
  },
  result_not_available: {
    label: "Result not available",
    tone: "muted",
    transcript: "unknown",
  },
};

const UNKNOWN_SCRIPT_STATUS: ScriptStatusView = {
  label: "Unknown status",
  tone: "muted",
  transcript: "unknown",
};

export function scriptStatusView(status: string): ScriptStatusView {
  return SCRIPT_STATUS[status] ?? UNKNOWN_SCRIPT_STATUS;
}

export const SCRIPT_TITLE_FALLBACK = "Run script";
export const SCRIPT_TITLE_MAX_CHARS = 200;
/** The engine's `code_preview` keeps at most this many characters. */
export const CODE_PREVIEW_MAX_CHARS = 120;

export type ScriptTitle = { text: string; mono: boolean };

function collapseWhitespace(text: string) {
  return text.replace(/\s+/g, " ").trim();
}

const NOT_A_TITLE = [
  "#",
  "import ",
  "from ",
  "def ",
  "async def ",
  "class ",
  "@",
  '"""',
  "'''",
];

/** Cuts to `max` characters (code points) with a trailing ellipsis, like the engine. */
export function cutTitle(text: string, max: number): string {
  let count = 0;
  for (let index = 0; index < text.length; ) {
    if (count === max - 1) {
      const rest = text.slice(index);
      return Array.from(rest).length > 1 ? `${text.slice(0, index)}…` : text;
    }
    const code = text.codePointAt(index) ?? 0;
    index += code > 0xffff ? 2 : 1;
    count++;
  }
  return text;
}

/**
 * The line of the code a run is titled by, trimmed (a comment keeps its
 * `#`): a leading comment with text, else the first top-level statement
 * that is not an import, definition, decorator or docstring, else the first
 * line that is not a comment. The engine's `code_preview` uses the same rule.
 */
export function codeTitleLine(code: string): string | undefined {
  const lines = code
    .split("\n")
    .map((line) => line.replace(/\r$/, ""))
    .filter((line) => line.trim() !== "");
  const first = lines.at(0)?.trim();
  if (
    first?.startsWith("#") &&
    !first.startsWith("#!") &&
    first.replace(/^#+/, "").trim() !== ""
  )
    return first;
  const statement = lines.find(
    (line) =>
      !/^\s/.test(line) &&
      !NOT_A_TITLE.some((prefix) => line.startsWith(prefix)),
  );
  if (statement !== undefined) return statement.trim();
  return lines.find((line) => !line.trim().startsWith("#"))?.trim();
}

/** Title from a code title line or the engine's `code_preview`. */
export function previewTitle(
  preview: string | null | undefined,
): ScriptTitle | undefined {
  const line = preview?.trim();
  if (!line) return undefined;
  if (line.startsWith("#") && !line.startsWith("#!")) {
    const text = line.replace(/^#+/, "").trim();
    return text ? { text, mono: false } : undefined;
  }
  return { text: line, mono: true };
}

/**
 * The title of a script run: the `description` argument, else the run's
 * stored description, else the title line of the code (or the engine's
 * preview of it), else a generic label. Always plain one-line text.
 */
export function scriptTitle({
  description,
  storedDescription,
  code,
  codePreview,
}: {
  description?: string | null;
  storedDescription?: string | null;
  code?: string | null;
  codePreview?: string | null;
}): ScriptTitle {
  for (const candidate of [description, storedDescription]) {
    const text = collapseWhitespace(candidate ?? "");
    if (text)
      return { text: cutTitle(text, SCRIPT_TITLE_MAX_CHARS), mono: false };
  }
  const line = code ? codeTitleLine(code) : undefined;
  const fromCode =
    line !== undefined
      ? previewTitle(cutTitle(line, CODE_PREVIEW_MAX_CHARS))
      : undefined;
  return (
    fromCode ??
    previewTitle(codePreview) ?? { text: SCRIPT_TITLE_FALLBACK, mono: false }
  );
}

/** The `code` and `description` of a script call, read from (possibly cut) arguments. */
export function scriptArgs(argumentsText: string): {
  code: string;
  description?: string;
  cut: boolean;
} {
  const whole = parsePythonArgs(argumentsText);
  const args = whole ?? parseCutPythonArgs(argumentsText);
  if (!args)
    return {
      code: argumentsText.trim().startsWith("{") ? "" : argumentsText,
      cut: false,
    };
  let code = typeof args.code === "string" ? args.code : "";
  const cut = !whole && code.endsWith(CUT_MARK);
  if (cut) code = code.slice(0, -CUT_MARK.length);
  const description =
    typeof args.description === "string" ? args.description : undefined;
  return description === undefined ? { code, cut } : { code, description, cut };
}

export type CallStatusKey = PythonCallRecord["status"] | "waiting";

/** Most significant status of several calls made from one line. */
const CALL_STATUS_RANK: CallStatusKey[] = [
  "waiting",
  "running",
  "failed",
  "denied",
  "cancelled",
  "unknown",
  "ok",
];

export function aggregateCallStatus(
  calls: readonly PythonCallRecord[],
  waitingCallIds?: ReadonlySet<string>,
): CallStatusKey {
  let best = CALL_STATUS_RANK.length - 1;
  for (const call of calls) {
    const status: CallStatusKey =
      call.status === "running" && waitingCallIds?.has(call.call_id)
        ? "waiting"
        : call.status;
    best = Math.min(best, CALL_STATUS_RANK.indexOf(status));
  }
  return CALL_STATUS_RANK[best];
}

export const APPROVAL_LABEL: Record<PythonCallRecord["approval"], string> = {
  not_needed: "",
  auto: "Auto-approved",
  allowed: "Allowed once",
  allowed_for_chat: "Allowed for chat",
  allowed_for_run: "Allowed for run",
  denied: "Denied",
  cancelled: "Stopped while waiting for approval",
  unknown: "Approval unknown",
};

export type CodeLine = { number: number; text: string };

export function splitCodeLines(code: string): string[] {
  const lines = code.replace(/\r\n?/g, "\n").split("\n");
  if (lines.length > 1 && lines[lines.length - 1] === "") lines.pop();
  return lines;
}

/** Calls placed on a valid line of this run's code, and the rest. */
export function placeCalls(
  calls: readonly PythonCallRecord[],
  lineCount: number,
): { byLine: Map<number, PythonCallRecord[]>; unplaced: PythonCallRecord[] } {
  const byLine = new Map<number, PythonCallRecord[]>();
  const unplaced: PythonCallRecord[] = [];
  for (const call of [...calls].sort((a, b) => a.seq - b.seq)) {
    if (call.line !== undefined && call.line >= 1 && call.line <= lineCount) {
      const list = byLine.get(call.line) ?? [];
      list.push(call);
      byLine.set(call.line, list);
    } else unplaced.push(call);
  }
  return { byLine, unplaced };
}

/** Code-unit range of a highlighted callee name within one line. */
export type CalleeRange = { start: number; end: number };

/** A displayed code line keeps at most this many characters. */
export const CODE_LINE_MAX_CHARS = 2_000;

/** Index of the code-unit boundary at or before `index` (never splits a surrogate pair). */
export function codePointBoundary(text: string, index: number): number {
  if (index <= 0 || index >= text.length)
    return Math.max(0, Math.min(index, text.length));
  const unit = text.charCodeAt(index - 1);
  return unit >= 0xd800 && unit <= 0xdbff ? index - 1 : index;
}

/** The shown part of a code line and how many characters were left out. */
export function cutCodeLine(text: string): { shown: string; hidden: number } {
  if (text.length <= CODE_LINE_MAX_CHARS) return { shown: text, hidden: 0 };
  const end = codePointBoundary(text, CODE_LINE_MAX_CHARS);
  return { shown: text.slice(0, end), hidden: text.length - end };
}

const IDENTIFIER_CALL = /[A-Za-z_][A-Za-z0-9_]*(?=\s*\()/g;

/** Index where a `#` comment starts outside of string literals, or the line length. */
function commentStart(line: string) {
  let quote: string | null = null;
  for (let i = 0; i < line.length; i++) {
    const char = line[i];
    if (quote) {
      if (char === "\\") i++;
      else if (char === quote) quote = null;
    } else if (char === '"' || char === "'") quote = char;
    else if (char === "#") return i;
  }
  return line.length;
}

/** Known tool names called on the shown part of this line, found lexically. */
export function lexicalCallees(
  line: string,
  toolNames: ReadonlySet<string>,
): CalleeRange[] {
  if (toolNames.size === 0) return [];
  const shown = cutCodeLine(line).shown;
  const end = commentStart(shown);
  const ranges: CalleeRange[] = [];
  for (const match of shown.slice(0, end).matchAll(IDENTIFIER_CALL)) {
    const start = match.index;
    if (toolNames.has(match[0]) && shown[start - 1] !== ".")
      ranges.push({ start, end: start + match[0].length });
  }
  return ranges;
}

/** Callee range of a call record from its column, counted in code points. */
export function calleeAtColumn(
  line: string,
  col: number,
): CalleeRange | undefined {
  let start = 0;
  for (let seen = 0; seen < col; seen++) {
    if (start >= line.length) return undefined;
    start += (line.codePointAt(start) ?? 0) > 0xffff ? 2 : 1;
  }
  if (start >= CODE_LINE_MAX_CHARS) return undefined;
  const match = /^[A-Za-z_][A-Za-z0-9_]*/.exec(
    line.slice(start, start + CODE_LINE_MAX_CHARS),
  );
  return match ? { start, end: start + match[0].length } : undefined;
}

export type CodeRow =
  | { kind: "line"; number: number }
  | { kind: "fold"; from: number; to: number };

export const CODE_WHOLE_MAX_LINES = 10;
export const CODE_FOLDED_MAX_ROWS = 12;
export const CODE_HEAD_LINES = 8;

const FOLD_STATUS_RANK: Partial<Record<CallStatusKey, number>> = {
  waiting: 0,
  running: 1,
  failed: 2,
  denied: 2,
  cancelled: 2,
};

/**
 * Lines a folded view must keep, most important first: the error line,
 * lines waiting for approval, running, failed/denied/stopped, then the
 * other call lines by line number.
 */
export function foldAnchors({
  errorLine,
  lineStatus,
  callLines,
}: {
  errorLine?: number;
  lineStatus: ReadonlyMap<number, CallStatusKey>;
  callLines: Iterable<number>;
}): number[] {
  const rank = (line: number) => {
    if (line === errorLine) return -1;
    const status = lineStatus.get(line);
    return status === undefined ? 3 : FOLD_STATUS_RANK[status] ?? 3;
  };
  const lines = new Set(callLines);
  lineStatus.forEach((_, line) => lines.add(line));
  if (errorLine !== undefined) lines.add(errorLine);
  return [...lines].sort((a, b) => rank(a) - rank(b) || a - b);
}

/**
 * Compact rows of the code view: short code whole; longer code keeps the
 * anchor lines in priority order and then the last non-empty line, within
 * the row budget, with fold rows between groups; without anchors the first
 * lines and one fold.
 */
export function foldCodeRows(
  lines: readonly string[],
  anchors: readonly number[],
): CodeRow[] {
  const total = lines.length;
  if (total <= CODE_WHOLE_MAX_LINES)
    return lines.map((_, index) => ({ kind: "line", number: index + 1 }));
  const rows: CodeRow[] = [];
  const valid = anchors.filter((n) => n >= 1 && n <= total);
  if (valid.length === 0) {
    for (let n = 1; n <= CODE_HEAD_LINES; n++)
      rows.push({ kind: "line", number: n });
    rows.push({ kind: "fold", from: CODE_HEAD_LINES + 1, to: total });
    return rows;
  }
  let last = total;
  while (last > 1 && lines[last - 1].trim() === "") last--;
  const picked = new Set<number>();
  for (const n of [...valid, last]) {
    if (picked.size >= CODE_FOLDED_MAX_ROWS) break;
    picked.add(n);
  }
  const sorted = [...picked].sort((a, b) => a - b);
  for (let i = 0; i + 1 < sorted.length; i++) {
    if (sorted[i + 1] - sorted[i] === 2 && picked.size < CODE_FOLDED_MAX_ROWS)
      picked.add(sorted[i] + 1);
  }
  let next = 1;
  for (const n of [...picked].sort((a, b) => a - b)) {
    if (n > next) rows.push({ kind: "fold", from: next, to: n - 1 });
    rows.push({ kind: "line", number: n });
    next = n + 1;
  }
  if (next <= total) rows.push({ kind: "fold", from: next, to: total });
  return rows;
}

/** Normalized comparison key of a file path. */
export function pathKey(path: string): string {
  const segments: string[] = [];
  for (const segment of normalizeSeparators(path).split("/")) {
    if (segment === "." || (segment === "" && segments.length > 0)) continue;
    if (segment === ".." && segments.length > 1) segments.pop();
    else segments.push(segment);
  }
  const normalized = segments.join("/");
  return /^[a-zA-Z]:\//.test(normalized)
    ? normalized.toLowerCase()
    : normalized;
}

export function isInsideDir(path: string, dir: string): boolean {
  const base = pathKey(dir);
  return base !== "" && pathKey(path).startsWith(`${base}/`);
}

/** Basename and project-relative directory of a path, for basename-first rows. */
export function splitDisplayPath(
  path: string,
  roots: WorkspaceRoots,
): { name: string; dir: string } {
  const relative = normalizeSeparators(relativeToRoot(path, roots));
  const slash = relative.lastIndexOf("/");
  if (slash < 0) return { name: relative, dir: "" };
  return { name: relative.slice(slash + 1), dir: relative.slice(0, slash) };
}

export type ScriptChange = {
  path: string;
  chunks: DiffChunk[];
  bytes?: number;
};

/**
 * Every file a run changed, once: diffs of its calls and files it wrote
 * directly, deduplicated by normalized path; artifacts are left out.
 */
export function scriptChanges(
  extra: Pick<PythonRunExtra, "calls" | "files_written">,
  liveCalls: readonly PythonCallRecord[] | undefined,
  artifactsDir: string | undefined,
): ScriptChange[] {
  const byPath = new Map<string, ScriptChange & { seen: Set<string> }>();
  const add = (path: string, chunks: DiffChunk[], bytes?: number) => {
    if (artifactsDir && isInsideDir(path, artifactsDir)) return;
    const key = pathKey(path);
    let entry = byPath.get(key);
    if (!entry) {
      entry = { path, chunks: [], seen: new Set() };
      byPath.set(key, entry);
    }
    if (bytes !== undefined) entry.bytes = bytes;
    for (const chunk of chunks) {
      const id = JSON.stringify([
        chunk.line1,
        chunk.line2,
        chunk.lines_remove,
        chunk.lines_add,
      ]);
      if (entry.seen.has(id)) continue;
      entry.seen.add(id);
      entry.chunks.push(chunk);
    }
  };
  for (const call of liveCalls ?? extra.calls) {
    const chunks = call.diffs.filter((diff): diff is DiffChunk =>
      isDiffChunk(diff),
    );
    for (const chunk of chunks) add(chunk.file_name, [chunk]);
  }
  for (const file of extra.files_written) {
    add(
      file.path,
      file.diffs.filter((diff): diff is DiffChunk => isDiffChunk(diff)),
      file.bytes,
    );
  }
  return [...byPath.values()].map(({ path, chunks, bytes }) =>
    bytes === undefined ? { path, chunks } : { path, chunks, bytes },
  );
}

/** Files under the run's artifacts directory, once each. */
export function scriptArtifacts(
  extra: Pick<PythonRunExtra, "artifacts" | "files_written">,
  artifactsDir: string | undefined,
): { path: string; bytes: number }[] {
  const seen = new Set<string>();
  const result: { path: string; bytes: number }[] = [];
  const candidates = [
    ...extra.artifacts,
    ...(artifactsDir
      ? extra.files_written.filter((file) =>
          isInsideDir(file.path, artifactsDir),
        )
      : []),
  ];
  for (const file of candidates) {
    const key = pathKey(file.path);
    if (seen.has(key)) continue;
    seen.add(key);
    result.push({ path: file.path, bytes: file.bytes });
  }
  return result;
}

function countDiffLines(text: string): number {
  if (!text) return 0;
  let count = 1;
  for (const char of text) if (char === "\n") count++;
  return text.endsWith("\n") ? count - 1 : count;
}

/** Diff stats of a set of calls, counted like the chat's Changes rows ("+5", "+2 −1"). */
export function callDiffStats(
  calls: readonly PythonCallRecord[],
): string | undefined {
  let added = 0;
  let removed = 0;
  let any = false;
  for (const call of calls)
    for (const diff of call.diffs.filter((item): item is DiffChunk =>
      isDiffChunk(item),
    )) {
      if (isOversizedDiff(diff)) continue;
      any = true;
      added += countDiffLines(diff.lines_add);
      removed += countDiffLines(diff.lines_remove);
    }
  if (!any) return undefined;
  const parts = [
    added > 0 ? `+${added}` : null,
    removed > 0 ? `−${removed}` : null,
  ].filter((part): part is string => part !== null);
  return parts.length > 0 ? parts.join(" ") : undefined;
}

export function plural(count: number, one: string, many = `${one}s`) {
  return `${count} ${count === 1 ? one : many}`;
}

/** Scroll the chat to the card of a script run; false when it is not rendered. */
export function revealScriptRun(runId: string): boolean {
  const card = Array.from(
    document.querySelectorAll("[data-python-run-id]"),
  ).find((item) => item.getAttribute("data-python-run-id") === runId);
  if (!card) return false;
  card.scrollIntoView({
    block: "center",
    behavior: window.matchMedia("(prefers-reduced-motion: reduce)").matches
      ? "instant"
      : "smooth",
  });
  return true;
}

export function byteLength(text: string) {
  return new TextEncoder().encode(text).length;
}

/** Wall time of the calls made from one line, or undefined while any runs. */
export function callsDuration(
  calls: readonly PythonCallRecord[],
): number | undefined {
  if (calls.length === 0) return undefined;
  let start = Infinity;
  let end = 0;
  for (const call of calls) {
    if (call.duration_ms === null) return undefined;
    start = Math.min(start, call.started_at_ms);
    end = Math.max(end, call.started_at_ms + call.duration_ms);
  }
  return Math.max(0, end - start);
}

export const CALL_STATUS: Record<
  CallStatusKey,
  { icon: LucideIcon; label: string; className?: string }
> = {
  running: { icon: LoaderCircle, label: "Running", className: "rf-spin" },
  waiting: { icon: Hourglass, label: "Waiting for your approval" },
  ok: { icon: CircleCheck, label: "Succeeded" },
  failed: { icon: CircleX, label: "Failed" },
  denied: { icon: Ban, label: "Denied" },
  cancelled: { icon: CircleSlash, label: "Stopped" },
  unknown: { icon: CircleHelp, label: "Unknown status" },
};

export const CALL_STATUS_TONE: Record<CallStatusKey, IconProps["tone"]> = {
  running: "accent",
  waiting: "warning",
  ok: "success",
  failed: "danger",
  denied: "warning",
  cancelled: "muted",
  unknown: "muted",
};

/** Diff messages per call record object, so unchanged rows keep identical props. */
const diffCache = new WeakMap<PythonCallRecord, DiffMessage[]>();
export function diffMessagesFor(record: PythonCallRecord): DiffMessage[] {
  let cached = diffCache.get(record);
  if (!cached) {
    cached = [
      {
        role: "diff",
        tool_call_id: record.call_id,
        content: record.diffs.filter((diff): diff is DiffChunk =>
          isDiffChunk(diff),
        ),
      },
    ];
    diffCache.set(record, cached);
  }
  return cached;
}

export type CodeToken = { text: string; color?: string };

const COLOR_STYLE = /(?:^|;)\s*color\s*:\s*([^;]+)/i;
const SAFE_COLOR =
  /^(#[0-9a-f]{3,8}|(rgba?|hsla?)\([0-9.,%\s/]+\)|var\(--[\w-]+\))$/i;

/** Text and theme colour of each run of a Shiki-highlighted line. */
export function htmlLineTokens(html: string): CodeToken[] {
  const doc = new DOMParser().parseFromString(
    `<code>${html}</code>`,
    "text/html",
  );
  const tokens: CodeToken[] = [];
  const walk = (node: Node, color: string | undefined) => {
    if (node.nodeType === Node.TEXT_NODE) {
      const text = node.textContent ?? "";
      if (text) tokens.push(color ? { text, color } : { text });
      return;
    }
    if (!(node instanceof Element)) return;
    const raw = COLOR_STYLE.exec(node.getAttribute("style") ?? "")?.[1].trim();
    const own = raw && SAFE_COLOR.test(raw) ? raw : undefined;
    node.childNodes.forEach((child) => walk(child, own ?? color));
  };
  const root = doc.body.firstElementChild;
  if (root) root.childNodes.forEach((child) => walk(child, undefined));
  return tokens;
}

const MAX_HIGHLIGHT_CHARS = 50_000;
const HIGHLIGHT_DELAY_MS = 150;

/** Highlights code once and splits it into per-line tokens; plain text until ready. */
export function useCodeTokens(
  code: string,
  lines: readonly string[],
): CodeToken[][] | null {
  const { highlight, isReady } = useShiki();
  const { appearance } = useAppearance();
  const [state, setState] = useState<{
    key: string;
    tokens: CodeToken[][];
  } | null>(null);
  const key = `${appearance}:${code}`;
  useEffect(() => {
    if (!isReady || code.length > MAX_HIGHLIGHT_CHARS || !code.trim()) return;
    let cancelled = false;
    const timer = setTimeout(() => {
      highlight(code, "python", appearance === "dark")
        .then((result) => {
          if (cancelled) return;
          const htmlLines = extractCodeLines(result.html);
          const tokens = lines.map((line, index) => {
            const parsed = htmlLineTokens(htmlLines[index] ?? "");
            return parsed.map((token) => token.text).join("") === line
              ? parsed
              : [{ text: line }];
          });
          setState({ key, tokens });
        })
        .catch(() => undefined);
    }, HIGHLIGHT_DELAY_MS);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [appearance, code, highlight, isReady, key, lines]);
  return state?.key === key ? state.tokens : null;
}

/** Title of a run: description, else the engine's code preview, else its ID. */
export function runTitle(run: PythonWorkspaceRun): ScriptTitle {
  const description = collapseWhitespace(run.description ?? "");
  if (description)
    return { text: cutTitle(description, SCRIPT_TITLE_MAX_CHARS), mono: false };
  return (
    previewTitle(run.code_preview) ?? { text: `Run ${run.run_id}`, mono: false }
  );
}

/**
 * Run meta in the order that loses least when cut: status · duration, then
 * (after the relative time the panel adds) calls · background.
 */
export function runSummary(run: PythonWorkspaceRun): {
  lead: string;
  tail: string;
} {
  const live = isLiveRun(run);
  const lead: string[] = [];
  if (live) {
    lead.push(
      scriptStatusView(run.background ? "background" : "running").label,
    );
  } else {
    lead.push(scriptStatusView(run.status).label);
    lead.push(
      run.finished_at_ms !== null && run.finished_at_ms > 0
        ? formatCallDuration(
            Math.max(0, run.finished_at_ms - run.started_at_ms),
          )
        : "duration unavailable",
    );
  }
  const tail: string[] = [];
  // Call records of model calls are counted in `calls_total` too.
  const models = Math.min(run.llm_calls ?? 0, run.calls_total);
  const tools = run.calls_total - models;
  if (tools > 0) tail.push(plural(tools, "call"));
  if (models > 0) tail.push(plural(models, "model call"));
  if (run.displays_total) tail.push(plural(run.displays_total, "display"));
  if (run.engine_changes)
    tail.push(plural(run.engine_changes, "engine change"));
  if (run.background && !live) tail.push("background");
  return { lead: lead.join(" · "), tail: tail.join(" · ") };
}
