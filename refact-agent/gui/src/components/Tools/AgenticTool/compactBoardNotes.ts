import { JSON_SCHEMA, load } from "js-yaml";
import { normalizeToolName } from "../../../utils/toolNameAliases";
import {
  COMPACT_HEADLINE_CHARS,
  compactFacts,
  compactText,
  compactTransportPresentation,
  readCompactRecord,
  type CompactAgenticInput,
  type CompactAgenticPresentation,
} from "./compactPresentation";

type RecordValue = Readonly<Record<string, unknown>> | null;
type Receipt = Pick<
  CompactAgenticPresentation,
  "stateLabel" | "tone" | "facts"
>;

const INTENTS: Partial<Record<string, string>> = {
  board_create: "create card",
  board_update: "update card",
  board_move: "move card",
  board_delete: "delete card",
  board_get: "read board",
  board_create_batch: "create cards",
  mark_done: "record as done",
  mark_failed: "record failure",
  mark_done_batch: "record as done",
  mark_failed_batch: "record failures",
  ready_cards: "find ready work",
  task_state: "read task state",
  task_start: "start planning",
  task_verify_card: "request verification",
  agent_complete: "report completion",
  agent_fail: "report failure",
  doc_create: "create document",
  doc_update: "replace document text",
  doc_append: "append section",
  doc_delete: "delete document",
  doc_pin: "pin document",
  doc_history: "read document history",
  task_mem_save: "save note",
  task_mem_get: "recall notes",
  task_mem_search: "search notes",
  task_mem_inbox: "read note inbox",
  task_mem_pin: "pin note",
  task_mem_archive: "archive note",
  task_mem_unarchive: "restore note",
  task_mem_triage_done: "record inbox review",
};

const MAX_RECEIPT_CHARS = 256_000;
const MAX_FRONTMATTER_CHARS = 16_000;

function field(record: RecordValue, key: string): unknown {
  return record && Object.prototype.hasOwnProperty.call(record, key)
    ? record[key]
    : undefined;
}

function text(record: RecordValue, ...keys: string[]): string | null {
  for (const key of keys) {
    const value = field(record, key);
    if (typeof value === "string" && value.trim() !== "") return value;
  }
  return null;
}

function counter(value: unknown): string | null {
  if (typeof value === "number" && Number.isSafeInteger(value) && value >= 0)
    return String(value);
  return typeof value === "string" && /^\d{1,40}$/u.test(value) ? value : null;
}

function jsonValue(value: unknown): unknown {
  if (typeof value !== "string") return value;
  if (value.length > MAX_RECEIPT_CHARS) return null;
  try {
    return JSON.parse(value) as unknown;
  } catch {
    return null;
  }
}

function boardRecord(value: unknown): RecordValue {
  const record = readCompactRecord(jsonValue(value));
  if (record !== null || typeof value !== "string") return record;
  if (value.length > MAX_RECEIPT_CHARS) return null;
  try {
    return readCompactRecord(load(value, { schema: JSON_SCHEMA }));
  } catch {
    return null;
  }
}

function requestedItems(toolName: string, args: RecordValue): unknown[] | null {
  const items = field(
    args,
    toolName === "board_create_batch" ? "cards" : "items",
  );
  return Array.isArray(items) ? items : null;
}

function headline(toolName: string, args: RecordValue, intent: string): string {
  let subject: string | null;
  if (toolName.startsWith("doc_")) {
    subject = text(args, "name", "slug");
  } else if (toolName.startsWith("task_mem_")) {
    subject = text(args, "title", "query", "memory_id", "note", "content");
    if (toolName === "task_mem_get")
      subject ??= text(args, "kind", "namespace");
  } else {
    subject = text(args, "title", "card_id", "name", "note");
  }
  const items = toolName.endsWith("_batch")
    ? requestedItems(toolName, args)
    : null;
  if (items) {
    const first = readCompactRecord(items[0]);
    subject = text(first, "title", "card_id");
    if (subject && items.length > 1) {
      subject = `${compactText(subject, 70)} + ${items.length - 1} more`;
    }
    subject ??= `${items.length} requested cards`;
  }
  if (toolName === "doc_pin" && field(args, "pinned") === false)
    intent = "unpin document";
  if (toolName === "task_mem_pin" && field(args, "pinned") === false)
    intent = "unpin note";
  if (toolName === "board_move") {
    const column = text(args, "column");
    if (column) intent = `move to ${compactText(column, 48)}`;
  }
  return (
    compactText(
      subject ? `${compactText(subject, 100)} — ${intent}` : intent,
      COMPACT_HEADLINE_CHARS,
    ) ?? intent
  );
}

function requestFacts(toolName: string, args: RecordValue) {
  if (toolName.startsWith("task_mem_")) {
    return compactFacts([
      { label: "Requested kind", value: text(args, "kind") },
      { label: "Requested scope", value: text(args, "namespace") },
      { label: "Requested pin", value: field(args, "pinned") },
    ]);
  }
  return compactFacts([
    {
      label: "Card",
      value: text(args, "title") ? text(args, "card_id") : null,
    },
    {
      label: "Document",
      value: text(args, "name") ? text(args, "slug") : null,
    },
    { label: "Requested note", value: text(args, "note", "reason") },
  ]);
}

function batchReceipt(
  toolName: string,
  args: RecordValue,
  result: unknown,
): Receipt | null {
  const requested = requestedItems(toolName, args);
  const rows = jsonValue(result);
  if (!requested || !Array.isArray(rows) || rows.length > requested.length)
    return null;
  const ids = new Set(
    requested.map(
      (item, index) =>
        text(readCompactRecord(item), "card_id") ?? `<item ${index + 1}>`,
    ),
  );
  const validIds = new Set(
    requested.map((item) => text(readCompactRecord(item), "card_id")),
  );
  const returned = new Set<string>();
  let accepted = 0;
  for (const row of rows) {
    const record = readCompactRecord(row);
    const id = text(record, "card_id");
    const success = field(record, "success");
    const error = field(record, "error");
    if (
      id === null ||
      !ids.has(id) ||
      returned.has(id) ||
      typeof success !== "boolean" ||
      (success && !validIds.has(id)) ||
      (success && error !== undefined && error !== null) ||
      (!success && text(record, "error") === null)
    )
      return null;
    returned.add(id);
    if (success) accepted += 1;
  }
  const partial = rows.length < requested.length || accepted < rows.length;
  const done = toolName === "mark_done_batch";
  return {
    stateLabel: partial
      ? "Partial batch receipt"
      : done
        ? "Recorded; not verified"
        : toolName === "board_create_batch" && accepted > 0
          ? "Cards created"
          : "Batch receipt",
    tone: partial ? "warning" : "neutral",
    facts: compactFacts([
      {
        label: "Receipts returned",
        value: `${rows.length} of ${requested.length} requested`,
      },
      {
        label: "Requests accepted",
        value: `${accepted}; ${rows.length - accepted} failed`,
      },
      {
        label: "Verification",
        value: done ? "Not established by bookkeeping" : null,
      },
    ]),
  };
}

function boardReceipt(args: RecordValue, result: unknown): Receipt | null {
  const record = boardRecord(result);
  const cardId = text(args, "card_id");
  if (cardId) {
    if (text(record, "id") !== cardId) return null;
    const column = text(record, "column");
    if (!column && !Array.isArray(field(record, "status_updates"))) return null;
    return {
      stateLabel: "Card received",
      tone: "neutral",
      facts: compactFacts([
        { label: "Card", value: text(record, "title") ?? cardId },
        { label: "Column", value: column },
        { label: "Priority", value: text(record, "priority") },
      ]),
    };
  }
  const cards = field(record, "cards");
  const rev = counter(field(record, "rev"));
  if (
    !Array.isArray(cards) ||
    rev === null ||
    !cards.every((card) => {
      const item = readCompactRecord(card);
      return text(item, "id") !== null && text(item, "column") !== null;
    })
  )
    return null;
  return {
    stateLabel: "Board received",
    tone: "neutral",
    facts: compactFacts([
      { label: "Cards returned", value: cards.length },
      { label: "Board revision", value: rev },
      { label: "Coverage", value: "Returned selection only" },
    ]),
  };
}

function documentReceipt(
  toolName: string,
  args: RecordValue,
  result: unknown,
): Receipt | null {
  if (typeof result !== "string" || !result.startsWith("---\n")) return null;
  const end = result.indexOf("\n---\n", 4);
  if (end < 0 || end > MAX_FRONTMATTER_CHARS) return null;
  const record = boardRecord(result.slice(4, end));
  const slug = text(args, "slug");
  const version = counter(field(record, "version"));
  if (
    !slug ||
    text(record, "slug") !== slug ||
    text(record, "name") === null ||
    text(record, "kind") === null ||
    text(record, "author_role") === null ||
    text(record, "created_at") === null ||
    text(record, "updated_at") === null ||
    version === null ||
    typeof field(record, "pinned") !== "boolean" ||
    (toolName === "doc_pin" &&
      field(args, "pinned") !== field(record, "pinned"))
  )
    return null;
  return {
    stateLabel: "Document received",
    tone: "neutral",
    facts: compactFacts([
      { label: "Version", value: version },
      { label: "Kind", value: text(record, "kind") },
      { label: "Pinned", value: field(record, "pinned") },
    ]),
  };
}

function tableRowCount(result: string, headers: string[]): number | null {
  const lines = result.trim().split(/\r?\n/u);
  const cells = (line: string) =>
    line
      .trim()
      .slice(1, -1)
      .split(/(?<!\\)\|/u)
      .map((cell) => cell.trim());
  if (
    lines.length < 2 ||
    cells(lines[0]).join("|") !== headers.join("|") ||
    cells(lines[1]).length !== headers.length ||
    !cells(lines[1]).every((cell) => /^:?-{2,}:?$/u.test(cell)) ||
    !lines.every(
      (line) => line.trim().startsWith("|") && line.trim().endsWith("|"),
    )
  )
    return null;
  const rows = lines.slice(2);
  if (!rows.every((line) => cells(line).length === headers.length)) return null;
  if (
    headers[0] === "version" &&
    !rows.every((line) => counter(cells(line)[0]) !== null)
  )
    return null;
  return rows.length;
}

function historyReceipt(result: unknown): Receipt | null {
  if (typeof result !== "string" || result.length > MAX_RECEIPT_CHARS)
    return null;
  const count = tableRowCount(result, [
    "version",
    "updated_at",
    "snapshot_at",
    "state",
    "path",
  ]);
  if (count === null) return null;
  return {
    stateLabel: "History received",
    tone: "neutral",
    facts: compactFacts([
      { label: "Snapshots returned", value: count },
      { label: "Coverage", value: "Retained snapshots, not all versions" },
    ]),
  };
}

function readyReceipt(result: unknown): Receipt | null {
  if (typeof result !== "string" || result.length > MAX_RECEIPT_CHARS)
    return null;
  const start = result.indexOf(
    "| Card | Title | Priority | Depends On | Brief |",
  );
  if (!result.startsWith("# Ready Cards (") || start < 0) return null;
  const end = result.indexOf("\n\n", start);
  const count = tableRowCount(result.slice(start, end < 0 ? undefined : end), [
    "Card",
    "Title",
    "Priority",
    "Depends On",
    "Brief",
  ]);
  if (count === null) return null;
  return {
    stateLabel: "Readiness snapshot received",
    tone: "neutral",
    facts: compactFacts([
      { label: "Ready cards returned", value: count },
      { label: "Coverage", value: "Returned readiness snapshot" },
    ]),
  };
}

function searchReceipt(result: unknown): Receipt | null {
  const hits = jsonValue(result);
  if (
    !Array.isArray(hits) ||
    !hits.every((hit) => {
      const record = readCompactRecord(hit);
      const score = field(record, "score");
      const provenance = readCompactRecord(field(record, "provenance"));
      return (
        text(record, "path") !== null &&
        typeof field(record, "snippet") === "string" &&
        typeof score === "number" &&
        Number.isFinite(score) &&
        text(provenance, "trust") === "untrusted-historical"
      );
    })
  )
    return null;
  return {
    stateLabel: "Search results received",
    tone: "neutral",
    facts: compactFacts([
      { label: "Matches returned", value: hits.length },
      { label: "Coverage", value: "Returned page, not all stored notes" },
      { label: "Trust", value: "Historical notes; not verified facts" },
    ]),
  };
}

export function compactBoardNotesPresentation(
  input: CompactAgenticInput,
): CompactAgenticPresentation | null {
  const toolName = normalizeToolName(input.toolName) ?? input.toolName;
  if (!Object.prototype.hasOwnProperty.call(INTENTS, toolName)) return null;
  const intent = INTENTS[toolName];
  if (intent === undefined) return null;
  const args = readCompactRecord(input.arguments);
  const presentation: CompactAgenticPresentation = {
    headline: headline(toolName, args, intent),
    ...compactTransportPresentation(input.transportState),
    facts: requestFacts(toolName, args),
  };
  if (input.transportState !== "succeeded") return presentation;
  presentation.stateLabel = "Outcome unconfirmed";
  if (input.arguments !== undefined && args === null) return presentation;
  let receipt: Receipt | null = null;
  switch (toolName) {
    case "board_create_batch":
    case "mark_done_batch":
    case "mark_failed_batch":
      receipt = batchReceipt(toolName, args, input.result);
      break;
    case "board_get":
      receipt = boardReceipt(args, input.result);
      break;
    case "ready_cards":
      receipt = readyReceipt(input.result);
      break;
    case "doc_create":
    case "doc_update":
    case "doc_append":
    case "doc_pin":
      receipt = documentReceipt(toolName, args, input.result);
      break;
    case "doc_history":
      receipt = historyReceipt(input.result);
      break;
    case "task_mem_search":
      receipt = searchReceipt(input.result);
      break;
  }
  return receipt ? { ...presentation, ...receipt } : presentation;
}
