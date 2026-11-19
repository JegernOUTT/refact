/**
 * Pure helpers for engine reads and changes made by scripts through
 * `engine(...)`. Values are untrusted JSON: everything returns plain text.
 */
import { createContext } from "react";
import { structuredPatch } from "diff";
import { shortText, visibleText } from "./safeText";
import type {
  DiffChunk,
  PythonApprovalRequest,
  PythonCallRecord,
  PythonEngineChange,
  PythonEngineClass,
  PythonEngineItem,
} from "../../../services/refact/types";

export const ENGINE_PREFIX = "engine.";
/** Inline values longer than this, or with a line break, are shown as blocks. */
export const ENGINE_INLINE_MAX_CHARS = 80;

export function isEngineTool(tool: string): boolean {
  return tool.startsWith(ENGINE_PREFIX) && tool.length > ENGINE_PREFIX.length;
}

/** "settings.set" from "engine.settings.set". */
export function engineFunction(tool: string): string {
  return isEngineTool(tool) ? tool.slice(ENGINE_PREFIX.length) : tool;
}

export type EngineCallKind = PythonEngineClass | "read" | "not_applied";

export const ENGINE_CLASS_LABEL: Record<EngineCallKind, string> = {
  read: "Read",
  change: "Change",
  risky: "Risky change",
  destructive: "Destructive change",
  not_applied: "Change (not applied)",
};

export type EngineValueView =
  | { kind: "absent"; text: string }
  | { kind: "inline"; text: string }
  | { kind: "block"; text: string };

/**
 * One side of an item: absent, a short inline scalar, or a block of
 * text/JSON. Invisible format characters are shown as ⟨U+XXXX⟩ markers.
 */
export function engineValueView(
  value: unknown,
  side: "old" | "new",
): EngineValueView {
  if (value === undefined)
    return { kind: "absent", text: side === "old" ? "not set" : "removed" };
  if (typeof value === "string") {
    const long =
      value.includes("\n") ||
      Array.from(value).length > ENGINE_INLINE_MAX_CHARS;
    return long
      ? { kind: "block", text: visibleText(value) }
      : { kind: "inline", text: visibleText(JSON.stringify(value)) };
  }
  if (value === null || typeof value === "number" || typeof value === "boolean")
    return { kind: "inline", text: String(value) };
  let text: string;
  try {
    text = JSON.stringify(value, null, 2);
  } catch {
    text = String(value);
  }
  text = visibleText(text);
  const compact = text.replace(/\s*\n\s*/g, " ");
  return compact.length <= ENGINE_INLINE_MAX_CHARS
    ? { kind: "inline", text: compact }
    : { kind: "block", text };
}

/** Neither side given: the item carries no value the GUI can show. */
export function hasNoValue(item: PythonEngineItem): boolean {
  return item.old === undefined && item.new === undefined;
}

/** A string pair that reads best as a diff: both strings, either multi-line or long. */
export function isTextPair(item: PythonEngineItem): item is PythonEngineItem & {
  old: string;
  new: string;
} {
  if (item.truncated) return false;
  if (typeof item.old !== "string" || typeof item.new !== "string")
    return false;
  return [item.old, item.new].some(
    (text) =>
      text.includes("\n") || Array.from(text).length > ENGINE_INLINE_MAX_CHARS,
  );
}

/**
 * Only an explicit statement counts: "permanent(ly)" wording, or a negated
 * one ("is not irreversible"), still gets the undo sentence.
 */
const UNDO_WORDS =
  /\b(cannot|can't|can not) be (undone|reversed|recovered)\b|(?<!\bnot\s)\birreversibl[ey]\b/i;

/** The warning shown for a risky or destructive request. */
export function engineWarning(
  engineClass: PythonEngineClass,
  warning: string | undefined,
): string | undefined {
  if (engineClass === "change") return undefined;
  const text = warning?.trim();
  if (engineClass === "destructive") {
    if (!text) return "This cannot be undone.";
    return UNDO_WORDS.test(text) ? text : `${text} This cannot be undone.`;
  }
  return text ?? "This change weakens a safety boundary.";
}

/** Upper-case first letter for a verb-phrase title used as a sentence. */
export function sentenceStart(text: string): string {
  return text ? `${text.charAt(0).toUpperCase()}${text.slice(1)}` : text;
}

/**
 * Kind of an engine call: from the applied change, else a refused or
 * cancelled change, else a read, else unknown.
 */
export function engineCallClass(
  call: PythonCallRecord,
  changes: readonly PythonEngineChange[] | undefined,
): EngineCallKind | undefined {
  if (!isEngineTool(call.tool)) return undefined;
  const change = changes?.find((item) => item.seq === call.seq);
  if (change) return change.class;
  if (call.approval === "denied" || call.approval === "cancelled")
    return "not_applied";
  if (call.approval === "not_needed") return "read";
  return undefined;
}

export function isEngineRequest(
  request: PythonApprovalRequest,
): request is PythonApprovalRequest & {
  engine: NonNullable<PythonApprovalRequest["engine"]>;
} {
  return request.engine !== undefined;
}

/** Applied engine changes of the run whose calls are being rendered. */
export const EngineChangesContext = createContext<
  readonly PythonEngineChange[] | undefined
>(undefined);

/** Live runs have no change list yet: count approved, finished engine calls. */
export function engineChangeCount(
  changes: readonly PythonEngineChange[] | undefined,
  total: number | undefined,
  calls: readonly PythonCallRecord[],
  finished: boolean,
): number {
  if (finished) return Math.max(total ?? 0, changes?.length ?? 0);
  return calls.filter(
    (call) =>
      isEngineTool(call.tool) &&
      call.status === "ok" &&
      call.approval !== "not_needed" &&
      call.approval !== "unknown",
  ).length;
}

/** What the live region says when a request appears. */
export function approvalAnnouncement(request: PythonApprovalRequest): string {
  const tool = shortText(request.tool_name).shown;
  if (!isEngineRequest(request)) return `Approval needed: ${tool}`;
  const warning = engineWarning(request.engine.class, request.engine.warning);
  return [
    `Approval needed: script wants to ${
      shortText(request.engine.title).shown
    } (${tool})`,
    warning &&
      `${ENGINE_CLASS_LABEL[request.engine.class]}: ${visibleText(warning)}`,
  ]
    .filter(Boolean)
    .join(". ");
}

const DIFF_CONTEXT_LINES = 2;

const lineText = (lines: readonly string[]) =>
  lines.length ? `${lines.join("\n")}\n` : "";

/**
 * One chunk per change of a hunk, holding only the changed lines and the
 * unchanged line on each side: the shared diff view would show unchanged
 * lines between two changes of one chunk as removed and added again.
 */
function hunkChunks(
  label: string,
  hunk: { oldStart: number; newStart: number; lines: readonly string[] },
): DiffChunk[] {
  const runs: { change: boolean; lines: string[] }[] = [];
  for (const line of hunk.lines) {
    if (!/^[ +-]/.test(line)) continue;
    const change = !line.startsWith(" ");
    const last = runs.at(-1);
    if (last?.change === change) last.lines.push(line);
    else runs.push({ change, lines: [line] });
  }
  const chunks: DiffChunk[] = [];
  let oldLine = hunk.oldStart;
  let newLine = hunk.newStart;
  let before: string | undefined;
  let pending: DiffChunk | undefined;
  for (const run of runs) {
    const body = run.lines.map((line) => line.slice(1));
    if (run.change) {
      const removed = body.filter((_, i) => run.lines[i].startsWith("-"));
      const added = body.filter((_, i) => run.lines[i].startsWith("+"));
      pending = {
        file_name: label,
        file_action: "edit",
        line1: oldLine,
        line2: newLine,
        lines_remove: lineText(removed),
        lines_add: lineText(added),
        lines_before: before === undefined ? null : `${before}\n`,
        lines_after: null,
      };
      oldLine += removed.length;
      newLine += added.length;
      continue;
    }
    if (pending) {
      chunks.push({ ...pending, lines_after: `${body[0]}\n` });
      pending = undefined;
      before = body.length > 1 ? body.at(-1) : undefined;
    } else before = body.at(-1);
    oldLine += body.length;
    newLine += body.length;
  }
  if (pending) chunks.push(pending);
  return chunks;
}

/** Line hunks between two texts, as chunks the shared diff view renders. */
export function textPairHunks(
  label: string,
  before: string,
  after: string,
): DiffChunk[] {
  const withNewline = (text: string) =>
    text.endsWith("\n") ? text : `${text}\n`;
  const patch = structuredPatch(
    label,
    label,
    withNewline(visibleText(before)),
    withNewline(visibleText(after)),
    "",
    "",
    { context: DIFF_CONTEXT_LINES },
  );
  return patch.hunks.flatMap((hunk) => hunkChunks(label, hunk));
}
