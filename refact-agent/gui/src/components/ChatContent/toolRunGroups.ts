import type {
  AssistantMessage,
  DiffChunk,
  EventMessage,
  ToolCall,
  ToolResult,
} from "../../services/refact";
import { normalizeToolName } from "../../utils/toolNameAliases";
import { isRecord } from "../../utils/isRecord";
import {
  execStateIsLive,
  execStateTranscriptStatus,
  isBusyStatus,
  isExecToolName,
  parseProcessArgs,
  resolveExecState,
  type ExecState,
  type ExecToolName,
} from "./ToolCard/execToolModel";
import { extractExecMetadata } from "../../services/refact/types";
import { parseToolJobReceipt } from "../../services/refact/toolJobs";
import {
  extractPrivacyShellMetadata,
  isPrivacyRefusalContent,
} from "../../services/refact/privacy";
import { isAppToolName } from "./ToolCard/appToolModel";
import { isRawTextDocToolCall } from "../Tools/types";
import { isAgenticToolName } from "../Tools/AgenticTool/agenticFamilies";
import { lookupAgenticFamily } from "../Tools/AgenticTool/agenticFamilies";
import { editToolFilePaths, getDiffStats } from "./ToolCard/DiffBlockStats";
import { normalizeReadPaths } from "./ToolCard/readToolPaths";
import {
  EDIT_TOOLS,
  hasThinkingText,
  type AssistantView,
  type DisplayItem,
  type DisplayItemAssistant,
} from "./ChatContentDisplayItems";

type ToolRunCategory =
  | "edit"
  | "run"
  | "read"
  | "search"
  | "browse"
  | "agents"
  | "other";

const TOOL_RUN_CATEGORY_ORDER: readonly ToolRunCategory[] = [
  "edit",
  "run",
  "read",
  "search",
  "browse",
  "agents",
  "other",
];

const RUN_TOOLS: ReadonlySet<string> = new Set([
  "shell",
  "shell_service",
  "process_start",
]);

const SEARCH_TOOLS: ReadonlySet<string> = new Set([
  "search_pattern",
  "search_semantic",
  "search_symbol_definition",
  "glob",
  "tree",
  "knowledge",
  "search_trajectories",
  "get_trajectory_context",
]);

const BROWSE_TOOLS: ReadonlySet<string> = new Set([
  "web",
  "web_search",
  "chrome",
]);

const BREAKER_TOOLS: ReadonlySet<string> = new Set([
  "ask_questions",
  "task_done",
  "agent_finish",
]);

const MIN_GROUP_CALLS = 2;

function isToolRunBreaker(call: ToolCall): boolean {
  return BREAKER_TOOLS.has(normalizeToolName(call.function.name) ?? "");
}

function toolRunCategory(name: string | undefined): ToolRunCategory {
  const normalized = normalizeToolName(name) ?? "";
  if (EDIT_TOOLS.has(normalized) || normalized === "mv") return "edit";
  if (RUN_TOOLS.has(normalized)) return "run";
  if (normalized === "cat") return "read";
  if (SEARCH_TOOLS.has(normalized)) return "search";
  if (BROWSE_TOOLS.has(normalized) || normalized.startsWith("chrome_")) {
    return "browse";
  }
  if (
    normalized === "subagent" ||
    lookupAgenticFamily(normalized)?.family === "spawn"
  ) {
    return "agents";
  }
  return "other";
}

function parseArgs(raw: string): Record<string, unknown> {
  try {
    const parsed: unknown = JSON.parse(raw);
    return parsed !== null &&
      typeof parsed === "object" &&
      !Array.isArray(parsed)
      ? (parsed as Record<string, unknown>)
      : {};
  } catch {
    return {};
  }
}

function editedPaths(call: ToolCall, diffs: readonly DiffChunk[]): string[] {
  if (diffs.length > 0) return diffs.map((diff) => diff.file_name);
  if (normalizeToolName(call.function.name) === "mv") {
    const destination = parseArgs(call.function.arguments).destination;
    return typeof destination === "string" && destination ? [destination] : [];
  }
  return editToolFilePaths(call.function.arguments);
}

function readPaths(call: ToolCall): string[] {
  return normalizeReadPaths(parseArgs(call.function.arguments));
}

type ToolRunDiffStats = { added: number; removed: number };

export type ToolRunSummaryPart = {
  category: ToolRunCategory;
  verb: string;
  rest: string;
  stats?: ToolRunDiffStats;
};

type ToolRunSummary = {
  parts: ToolRunSummaryPart[];
  failed: number;
  durationMs: number | null;
};

export type ToolRunCallState = {
  failed: boolean;
  busy: boolean;
};

function plural(count: number, one: string, many: string): string {
  return `${count} ${count === 1 ? one : many}`;
}

function uniqueCount(paths: readonly string[], fallback: number): number {
  const unique = new Set(paths.filter(Boolean));
  return unique.size > 0 ? unique.size : fallback;
}

export function toolRunDurationMs(calls: readonly ToolCall[]): number | null {
  let start = Infinity;
  let end = -Infinity;
  for (const call of calls) {
    const { started_at_ms: startedAt, completed_at_ms: completedAt } = call;
    if (typeof startedAt !== "number" || typeof completedAt !== "number") {
      continue;
    }
    start = Math.min(start, startedAt);
    end = Math.max(end, completedAt);
  }
  return Number.isFinite(start) && Number.isFinite(end)
    ? Math.max(0, end - start)
    : null;
}

const SECOND_MS = 1000;
const MINUTE_S = 60;
const HOUR_S = 60 * MINUTE_S;

export function formatToolRunDuration(ms: number): string {
  const totalSeconds = Math.max(1, Math.round(ms / SECOND_MS));
  if (totalSeconds < MINUTE_S) return `${totalSeconds}s`;
  if (totalSeconds < HOUR_S) {
    const minutes = Math.floor(totalSeconds / MINUTE_S);
    const seconds = totalSeconds % MINUTE_S;
    return seconds === 0 ? `${minutes}m` : `${minutes}m ${seconds}s`;
  }
  const hours = Math.floor(totalSeconds / HOUR_S);
  const minutes = Math.floor((totalSeconds % HOUR_S) / MINUTE_S);
  return minutes === 0 ? `${hours}h` : `${hours}h ${minutes}m`;
}

function categoryPart(
  category: ToolRunCategory,
  list: readonly ToolCall[],
  diffsByToolId: Readonly<Record<string, readonly DiffChunk[] | undefined>>,
): ToolRunSummaryPart {
  const count = list.length;
  switch (category) {
    case "edit": {
      const diffs = list.flatMap((call) =>
        call.id ? diffsByToolId[call.id] ?? [] : [],
      );
      const files = uniqueCount(
        list.flatMap((call) =>
          editedPaths(call, call.id ? diffsByToolId[call.id] ?? [] : []),
        ),
        count,
      );
      const rest = plural(files, "file", "files");
      const stats = diffs.length > 0 ? getDiffStats(diffs) : null;
      return stats && !stats.limited
        ? {
            category,
            verb: "Edited",
            rest,
            stats: { added: stats.added, removed: stats.removed },
          }
        : { category, verb: "Edited", rest };
    }
    case "run":
      return {
        category,
        verb: "Ran",
        rest: plural(count, "command", "commands"),
      };
    case "read":
      return {
        category,
        verb: "Read",
        rest: plural(
          uniqueCount(list.flatMap(readPaths), count),
          "file",
          "files",
        ),
      };
    case "search":
      return { category, verb: "Searched", rest: `${count}×` };
    case "browse":
      return { category, verb: "Browsed", rest: `${count}×` };
    case "agents":
      return {
        category,
        verb: "Started",
        rest: plural(count, "agent", "agents"),
      };
    case "other":
      return {
        category,
        verb: "Used",
        rest: plural(count, "other tool", "other tools"),
      };
  }
}

export function summarizeToolRun(
  calls: readonly ToolCall[],
  states: ReadonlyMap<string, ToolRunCallState>,
  diffsByToolId: Readonly<Record<string, readonly DiffChunk[] | undefined>>,
): ToolRunSummary {
  const byCategory = new Map<ToolRunCategory, ToolCall[]>();
  for (const call of calls) {
    const category = toolRunCategory(call.function.name);
    const list = byCategory.get(category) ?? [];
    list.push(call);
    byCategory.set(category, list);
  }

  const parts: ToolRunSummaryPart[] = [];
  for (const category of TOOL_RUN_CATEGORY_ORDER) {
    const list = byCategory.get(category);
    if (list) parts.push(categoryPart(category, list, diffsByToolId));
  }

  const failed = calls.filter(
    (call) => call.id !== undefined && states.get(call.id)?.failed === true,
  ).length;

  return { parts, failed, durationMs: toolRunDurationMs(calls) };
}

function toolRunPartStatsText(stats: ToolRunDiffStats): string {
  return `+${stats.added} −${stats.removed}`;
}

export function toolRunSummaryText(summary: ToolRunSummary): string {
  return summary.parts
    .map((part) =>
      [part.verb, part.rest, part.stats && toolRunPartStatsText(part.stats)]
        .filter(Boolean)
        .join(" "),
    )
    .join(" · ");
}

function serverToolFailed(
  blocks: readonly unknown[] | null | undefined,
  callId: string,
): boolean {
  return (blocks ?? []).some(
    (block) =>
      isRecord(block) &&
      typeof block.type === "string" &&
      block.type.endsWith("_tool_result") &&
      block.tool_use_id === callId &&
      isRecord(block.content) &&
      typeof block.content.type === "string" &&
      block.content.type.endsWith("_error"),
  );
}

const SPECIALIZED_TOOL_NAMES: ReadonlySet<string> = new Set([
  "python",
  "cat",
  "tree",
  "search_pattern",
  "search_semantic",
  "search_symbol_definition",
  "glob",
  "subagent",
  "delegate",
  "knowledge",
  "search_trajectories",
  "get_trajectory_context",
  "create_knowledge",
  "activate_skill",
  "web",
  "web_search",
  "mv",
  "rm",
  "add_workspace_folder",
  "tasks_set",
  "check_agents",
  "agent_pulse",
  "agent_diff",
  "doc_list",
  "doc_get",
  "agent_finish",
  "task_done",
  "ask_questions",
  "compress_chat_probe",
  "compress_chat_apply",
  "sleep",
  "chrome",
]);

function rendersAsToolJob(
  call: ToolCall,
  result: ToolResult | undefined,
  threadId: string,
): boolean {
  const job = parseToolJobReceipt(result?.extra?.tool_job);
  return (
    job !== null &&
    job.status === "accepted" &&
    job.chat_id === threadId &&
    job.tool_call_id === call.id &&
    job.tool_name === call.function.name
  );
}

function rendersSpecialized(name: string, call: ToolCall): boolean {
  return (
    SPECIALIZED_TOOL_NAMES.has(name) ||
    isAppToolName(name) ||
    isRawTextDocToolCall({ ...call, function: { ...call.function, name } }) ||
    isAgenticToolName(name) ||
    name.startsWith("openai_")
  );
}

function execRendererName(
  call: ToolCall,
  result: ToolResult | undefined,
  threadId: string,
): ExecToolName | null {
  if (
    result &&
    call.id &&
    (extractPrivacyShellMetadata(result.extra) !== null ||
      isPrivacyRefusalContent(result.content))
  ) {
    return null;
  }
  if (rendersAsToolJob(call, result, threadId)) return null;
  const name = normalizeToolName(call.function.name) ?? "";
  if (isExecToolName(name) && name !== "exec") return name;
  if (rendersSpecialized(name, call)) return null;
  return extractExecMetadata(result?.extra) ? "exec" : null;
}

function terminalNonZeroExit(state: ExecState): boolean {
  const exitCode = state.outcome?.exitCode ?? state.metadata?.exit_code;
  return (
    typeof exitCode === "number" &&
    exitCode !== 0 &&
    !isBusyStatus(state.status)
  );
}

export function toolRunCallState(
  call: ToolCall,
  owner: AssistantMessage,
  result: ToolResult | undefined,
  foldedEvent: EventMessage | undefined,
  threadId: string,
): ToolRunCallState {
  const serverFailed =
    call.id !== undefined &&
    (owner.server_executed_tools ?? []).some((item) => item.id === call.id) &&
    serverToolFailed(owner.server_content_blocks, call.id);
  const execName = execRendererName(call, result, threadId);
  if (!execName) {
    return {
      failed: serverFailed || result?.tool_failed === true,
      busy: false,
    };
  }
  const state = resolveExecState(
    execName,
    parseProcessArgs(call.function.arguments),
    result,
    foldedEvent,
    false,
  );
  return {
    failed:
      serverFailed ||
      execStateTranscriptStatus(state, false) === "failed" ||
      terminalNonZeroExit(state),
    busy: execStateIsLive(state),
  };
}

export function isInterruptedAssistant(message: AssistantMessage): boolean {
  return (
    message.extra?.interrupted === true || message.extra?._ui_only === true
  );
}

type ToolRunOwner = {
  call: ToolCall;
  owner: DisplayItemAssistant;
};

export type DisplayItemToolRun = {
  type: "tool_run";
  key: string;
  messageIndex: number;
  members: readonly DisplayItem[];
  owners: readonly ToolRunOwner[];
  diffsByToolId: Readonly<Record<string, readonly DiffChunk[] | undefined>>;
};

export type TranscriptDisplayItem = DisplayItem | DisplayItemToolRun;

type Piece = {
  item: DisplayItem;
  work: boolean;
  calls: number;
};

type ToolChunk =
  | { kind: "work"; calls: ToolCall[] }
  | { kind: "breaker"; call: ToolCall };

const ATTACHMENT_EXCLUDED_IDS: ReadonlySet<string> = new Set([
  "knowledge_enrichment",
  "project_context",
  "buddy_project_memory_pulse",
]);

function hasReasoning(item: DisplayItemAssistant): boolean {
  const message = item.message;
  return (
    Boolean(message.reasoning_content?.trim()) ||
    hasThinkingText(message) ||
    (message.server_content_blocks?.length ?? 0) > 0
  );
}

function toolChunks(item: DisplayItemAssistant): ToolChunk[] {
  const server = item.message.server_executed_tools ?? [];
  const chunks: ToolChunk[] = [];
  let current: ToolCall[] = [];
  const flush = () => {
    if (current.length === 0) return;
    chunks.push({ kind: "work", calls: current });
    current = [];
  };
  for (const call of [...server, ...(item.message.tool_calls ?? [])]) {
    if (isToolRunBreaker(call)) {
      flush();
      chunks.push({ kind: "breaker", call });
    } else {
      current.push(call);
    }
  }
  flush();
  return chunks;
}

const EMPTY_VIEW: AssistantView = {
  reasoning: false,
  prose: false,
  sources: false,
  footer: false,
  serverTools: false,
  toolCalls: [],
  activeTools: false,
};

export function pinnedToolRunItem(
  owner: DisplayItemAssistant,
  call: ToolCall,
): DisplayItemAssistant {
  return {
    ...owner,
    key: `${owner.key}:pin:${call.id ?? call.index}`,
    view: { ...EMPTY_VIEW, toolCalls: [call] },
  };
}

function chunkPiece(
  item: DisplayItemAssistant,
  chunk: ToolChunk,
  key: string,
  extra: Partial<AssistantView>,
): Piece {
  const calls = chunk.kind === "work" ? chunk.calls : [chunk.call];
  return {
    item: {
      ...item,
      key,
      view: { ...EMPTY_VIEW, toolCalls: calls, ...extra },
    },
    work: chunk.kind === "work",
    calls: calls.length,
  };
}

function wholePiece(item: DisplayItemAssistant, work: boolean): Piece[] {
  return [{ item, work, calls: 0 }];
}

function buildAssistantPieces(item: DisplayItemAssistant): Piece[] {
  if (isInterruptedAssistant(item.message)) return wholePiece(item, false);
  const prose = Boolean(
    typeof item.message.content === "string" && item.message.content.trim(),
  );
  const visible = prose || (item.message.citations?.length ?? 0) > 0;
  const reasoning = hasReasoning(item);
  const chunks = toolChunks(item);
  const pieces: Piece[] = [];

  if (visible) {
    if (reasoning) {
      pieces.push({
        item: {
          ...item,
          key: `${item.key}:pre`,
          view: { ...EMPTY_VIEW, reasoning: true },
        },
        work: true,
        calls: 0,
      });
    }
    pieces.push({
      item: {
        ...item,
        view: {
          ...EMPTY_VIEW,
          prose: true,
          sources: true,
          footer: true,
        },
      },
      work: false,
      calls: 0,
    });
    chunks.forEach((chunk, index) => {
      pieces.push(
        chunkPiece(
          item,
          chunk,
          index === 0 ? `${item.key}:post` : `${item.key}:post:${index}`,
          { activeTools: index === chunks.length - 1 },
        ),
      );
    });
  } else {
    const leadsWithWork = chunks[0]?.kind === "work";
    if (reasoning && !leadsWithWork) {
      pieces.push({
        item: { ...item, view: { ...EMPTY_VIEW, reasoning: true } },
        work: true,
        calls: 0,
      });
    }
    chunks.forEach((chunk, index) => {
      const keyIndex = pieces.length;
      pieces.push(
        chunkPiece(
          item,
          chunk,
          keyIndex === 0 ? item.key : `${item.key}:post:${keyIndex}`,
          {
            reasoning: index === 0 && reasoning && leadsWithWork,
            sources: index === 0,
            footer: index === chunks.length - 1,
            activeTools: index === chunks.length - 1,
          },
        ),
      );
    });
  }

  if (pieces.length === 0) return wholePiece(item, false);
  if (pieces.length === 1) return [{ ...pieces[0], item }];
  return pieces;
}

const piecesCache = new WeakMap<DisplayItemAssistant, Piece[]>();
const pieceSource = new WeakMap<DisplayItem, DisplayItemAssistant>();

function assistantPieces(item: DisplayItemAssistant): Piece[] {
  if (item.view) return [{ item, work: !item.view.prose, calls: 0 }];
  const cached = piecesCache.get(item);
  if (cached) return cached;
  const pieces = buildAssistantPieces(item);
  if (pieces.length > 1) {
    for (const piece of pieces) pieceSource.set(piece.item, item);
  }
  piecesCache.set(item, pieces);
  return pieces;
}

function isAttachment(item: DisplayItem): boolean {
  if (item.type === "diff_group") return true;
  return (
    item.type === "context_files" &&
    !ATTACHMENT_EXCLUDED_IDS.has(item.toolCallId ?? "")
  );
}

function groupOwners(members: readonly DisplayItem[]): ToolRunOwner[] {
  const owners: ToolRunOwner[] = [];
  for (const member of members) {
    if (member.type !== "assistant") continue;
    for (const call of member.view?.toolCalls ?? []) {
      owners.push({ call, owner: member });
    }
  }
  return owners;
}

function groupDiffs(
  owners: readonly ToolRunOwner[],
): Record<string, readonly DiffChunk[] | undefined> {
  const diffs: Record<string, readonly DiffChunk[] | undefined> = {};
  for (const { call, owner } of owners) {
    if (!call.id) continue;
    const chunks = owner.diffsByToolId[call.id] as DiffChunk[] | undefined;
    if (chunks) diffs[call.id] = chunks;
  }
  return diffs;
}

const memberCache = new WeakMap<DisplayItemAssistant, DisplayItemAssistant>();

function groupMember(item: DisplayItem): DisplayItem {
  if (item.type !== "assistant" || item.view) return item;
  const cached = memberCache.get(item);
  if (cached) return cached;
  const member: DisplayItemAssistant = {
    ...item,
    view: {
      ...EMPTY_VIEW,
      reasoning: true,
      sources: true,
      footer: true,
      activeTools: true,
      toolCalls: [
        ...(item.message.server_executed_tools ?? []),
        ...(item.message.tool_calls ?? []),
      ],
    },
  };
  memberCache.set(item, member);
  return member;
}

const groupCache = new WeakMap<DisplayItem, DisplayItemToolRun>();

function toolRunItem(segmentItems: DisplayItem[]): DisplayItemToolRun {
  const members = segmentItems.map(groupMember);
  const first = members[0];
  const cached = groupCache.get(first);
  if (
    cached?.members.length === members.length &&
    cached.members.every((member, index) => member === members[index])
  ) {
    return cached;
  }
  const owners = groupOwners(members);
  const group: DisplayItemToolRun = {
    type: "tool_run",
    key: `run:${first.key}`,
    messageIndex: first.messageIndex,
    members,
    owners,
    diffsByToolId: groupDiffs(owners),
  };
  groupCache.set(first, group);
  return group;
}

export function groupToolRuns(items: DisplayItem[]): TranscriptDisplayItem[] {
  const output: TranscriptDisplayItem[] = [];
  let segment: Piece[] = [];
  let inAssistantChain = false;

  const close = (followed: boolean) => {
    const calls = segment.reduce((total, piece) => total + piece.calls, 0);
    if (followed && calls >= MIN_GROUP_CALLS) {
      output.push(toolRunItem(segment.map((piece) => piece.item)));
    } else {
      for (const piece of segment) output.push(piece.item);
    }
    segment = [];
  };

  for (const item of items) {
    let pieces: Piece[];
    if (item.type === "assistant") {
      pieces = assistantPieces(item);
      inAssistantChain = pieces.at(-1)?.work ?? false;
    } else if (inAssistantChain && isAttachment(item)) {
      pieces = [{ item, work: true, calls: 0 }];
    } else {
      pieces = [{ item, work: false, calls: 0 }];
      inAssistantChain = false;
    }
    for (const piece of pieces) {
      if (piece.work) {
        segment.push(piece);
      } else {
        close(true);
        output.push(piece.item);
      }
    }
  }
  close(false);
  return mergeUngroupedPieces(output);
}

function mergeUngroupedPieces(
  output: TranscriptDisplayItem[],
): TranscriptDisplayItem[] {
  const merged: TranscriptDisplayItem[] = [];
  for (let index = 0; index < output.length; index++) {
    const item = output[index];
    const source = item.type === "tool_run" ? undefined : pieceSource.get(item);
    const pieces = source ? piecesCache.get(source) : undefined;
    if (
      source &&
      pieces &&
      pieces.every((piece, offset) => output[index + offset] === piece.item)
    ) {
      merged.push(source);
      index += pieces.length - 1;
      continue;
    }
    merged.push(item);
  }
  return merged;
}
