import { normalizeToolName } from "../../../utils/toolNameAliases";
import {
  COMPACT_HEADLINE_CHARS,
  COMPACT_STATE_CHARS,
  compactFacts,
  compactText,
  readCompactRecord,
  type CompactAgenticInput,
  type CompactAgenticPresentation,
} from "./compactPresentation";
import {
  boardReceipt,
  receiptState,
  text,
  type BoardReceipt,
} from "./messageReceipt";

type RecordValue = Readonly<Record<string, unknown>>;
type State = Pick<CompactAgenticPresentation, "stateLabel" | "tone">;
type Fact = { label: string; value: string | null };

const LAUNCH_NAMES = [
  "spawn_agent",
  "task_spawn_agent",
  "spawn_ab",
  "spawn_agents_batch",
];
const MESSAGE_NAMES = [
  "agent_message",
  "agent_steer",
  "task_broadcast",
  "planner_reply",
  "agent_ask_planner",
];
const PROGRESS_NAMES = ["progress_report", "agent_update"];
const UNCONFIRMED: State = {
  stateLabel: "Outcome unconfirmed",
  tone: "neutral",
};

function firstText(record: RecordValue, ...keys: string[]): string | null {
  for (const key of keys) {
    const value = text(record, key);
    if (value !== null) return value;
  }
  return null;
}

function deliveryState(outcome: string): State {
  switch (outcome) {
    case "queued":
      return { stateLabel: "Queued", tone: "neutral" };
    case "delivered":
      return { stateLabel: "Delivered", tone: "neutral" };
    case "consumed":
      return { stateLabel: "Consumed", tone: "neutral" };
    case "duplicate":
      return { stateLabel: "Duplicate", tone: "warning" };
    case "expired":
      return { stateLabel: "Expired", tone: "warning" };
    case "superseded":
      return { stateLabel: "Superseded", tone: "warning" };
    case "no_live_session":
      return { stateLabel: "No live session", tone: "warning" };
    case "skipped":
      return { stateLabel: "Skipped", tone: "warning" };
    case "excluded":
      return { stateLabel: "Excluded", tone: "neutral" };
    case "failed":
      return { stateLabel: "Delivery failed", tone: "danger" };
    default:
      return UNCONFIRMED;
  }
}

function messageState(board: BoardReceipt | null, legacy: string): State {
  if (board?.kind === "agent_message") return deliveryState(board.outcome);
  if (board?.kind === "agent_steer") return deliveryState(board.card.outcome);
  if (board?.kind === "task_broadcast") {
    const queued = board.cards.filter(
      (card) => card.outcome === "queued",
    ).length;
    const delivered = board.cards.filter(
      (card) => card.outcome === "delivered",
    ).length;
    const state =
      board.total > 0 && queued === board.total
        ? "queued"
        : board.total > 0 && delivered === board.total
          ? "delivered"
          : "reached";
    return {
      stateLabel: `${board.notified}/${board.total} ${state}`,
      tone: board.notified < board.total ? "warning" : "neutral",
    };
  }
  switch (legacy) {
    case "Message queued":
      return deliveryState("queued");
    case "Note accepted":
    case "Question recorded":
    case "Answer recorded":
    case "Progress recorded":
      return { stateLabel: legacy, tone: "neutral" };
    default:
      return UNCONFIRMED;
  }
}

function broadcastFacts(
  board: Extract<BoardReceipt, { kind: "task_broadcast" }>,
): Fact[] {
  const counts = new Map<string, number>();
  for (const card of board.cards) {
    const label = deliveryState(card.outcome).stateLabel;
    counts.set(label, (counts.get(label) ?? 0) + 1);
  }
  return [
    {
      label: "Delivery receipts",
      value: Array.from(
        counts,
        ([label, count]) => `${count} ${label.toLowerCase()}`,
      ).join(" · "),
    },
  ];
}

function launchState(
  name: string,
  result: RecordValue | null,
  raw: unknown,
): { state: State; facts: Fact[] } {
  if (name === "spawn_agents_batch") {
    let parsed: unknown = raw;
    if (typeof raw === "string") {
      try {
        parsed = JSON.parse(raw) as unknown;
      } catch {
        return { state: UNCONFIRMED, facts: [] };
      }
    }
    const rows: unknown[] | null = Array.isArray(parsed)
      ? parsed
      : Array.isArray(result?.results)
        ? result.results
        : null;
    if (rows === null || rows.length === 0)
      return { state: UNCONFIRMED, facts: [] };
    const receipts = rows.map(readCompactRecord);
    if (
      receipts.some(
        (row) =>
          row === null ||
          text(row, "card_id") === null ||
          typeof row.success !== "boolean" ||
          (row.agent_chat_id !== undefined &&
            (typeof row.agent_chat_id !== "string" ||
              row.agent_chat_id === "")) ||
          (row.error !== undefined &&
            row.error !== null &&
            typeof row.error !== "string") ||
          (row.success && row.error !== undefined && row.error !== null),
      )
    )
      return { state: UNCONFIRMED, facts: [] };
    const accepted = receipts.filter((row) => row?.success === true).length;
    const failed = receipts.length - accepted;
    return {
      state:
        failed > 0
          ? {
              stateLabel:
                accepted > 0 ? "Some launches failed" : "Launches failed",
              tone: accepted > 0 ? "warning" : "danger",
            }
          : { stateLabel: "Launches accepted", tone: "neutral" },
      facts: [
        {
          label: "Returned receipts",
          value: `${accepted} accepted · ${failed} failed; other requests unconfirmed`,
        },
      ],
    };
  }
  if (result === null || text(result, "card_id") === null)
    return { state: UNCONFIRMED, facts: [] };
  const child = (row: RecordValue | null) =>
    row !== null &&
    text(row, "agent_id") !== null &&
    text(row, "agent_chat_id") !== null;
  const recognized =
    name === "spawn_ab"
      ? child(readCompactRecord(result.variant_a)) &&
        child(readCompactRecord(result.variant_b))
      : child(result);
  if (!recognized) return { state: UNCONFIRMED, facts: [] };
  const states: Partial<Record<string, State>> = {
    queued: { stateLabel: "Queued", tone: "neutral" },
    running: { stateLabel: "Running", tone: "active" },
    completed: { stateLabel: "Child finished · unverified", tone: "neutral" },
    failed: { stateLabel: "Child failed", tone: "danger" },
    cancelled: { stateLabel: "Child cancelled", tone: "warning" },
  };
  const executionState =
    typeof result.execution_state === "string" &&
    Object.prototype.hasOwnProperty.call(states, result.execution_state)
      ? states[result.execution_state]
      : undefined;
  return {
    state:
      result.execution_state === undefined
        ? { stateLabel: "Launch accepted", tone: "neutral" }
        : executionState ?? UNCONFIRMED,
    facts: [{ label: "Model", value: text(result, "model") }],
  };
}

export function buildCompactLaunchMessagePresentation(
  input: CompactAgenticInput,
): CompactAgenticPresentation | null {
  const name = normalizeToolName(input.toolName) ?? input.toolName;
  const launch = LAUNCH_NAMES.includes(name);
  const progress = PROGRESS_NAMES.includes(name);
  if (!launch && !progress && !MESSAGE_NAMES.includes(name)) return null;
  const argsRecord = readCompactRecord(input.arguments);
  const args = argsRecord ?? {};
  const received = input.transportState === "succeeded";
  const result = received ? readCompactRecord(input.result) : null;
  const board = received
    ? boardReceipt(name, readCompactRecord(input.extra) ?? undefined)
    : null;
  const legacy = receiptState(
    name,
    args,
    typeof input.result === "string" ? input.result : null,
    board,
  );
  const body = launch
    ? firstText(args, "task", "title", "prompt", "card_title") ??
      firstText(result ?? {}, "card_title", "title")
    : progress
      ? firstText(args, "status_line", "message")
      : name === "agent_ask_planner"
        ? text(args, "question")
        : name === "planner_reply"
          ? text(args, "answer")
          : firstText(args, "text", "message");
  const recipient =
    board?.kind === "agent_steer" && text(board.card, "title") !== null
      ? text(board.card, "title")
      : firstText(args, "recipient_title", "card_title", "title") ??
        (name === "task_broadcast"
          ? "Eligible board agents"
          : name === "agent_ask_planner"
            ? "Task planner"
            : firstText(args, "card_id", "to", "target", "recipient"));
  const launched =
    received && launch ? launchState(name, result, input.result) : null;
  let state = launch
    ? launched?.state ?? UNCONFIRMED
    : messageState(board, legacy.label);
  if (input.arguments !== undefined && argsRecord === null) state = UNCONFIRMED;
  if (!received) state = UNCONFIRMED;
  if (
    input.transportState === "pending" ||
    input.transportState === "running"
  ) {
    state = {
      stateLabel: launch ? "Requesting" : "Awaiting receipt",
      tone: "neutral",
    };
  }
  if (input.transportState === "failed")
    state = { stateLabel: "Request failed", tone: "danger" };
  const facts: Fact[] = [];
  if (launch) {
    facts.push({
      label: "Card",
      value:
        firstText(args, "card_title", "card_id") ??
        firstText(result ?? {}, "card_title", "card_id"),
    });
    facts.push(...(launched?.facts ?? []));
    facts.push({
      label: "Evidence",
      value: "Launch acceptance is not completed or verified work",
    });
  } else if (progress) {
    facts.push({
      label: "Card",
      value: firstText(args, "card_title", "card_id"),
    });
    facts.push({
      label: "Evidence",
      value: "Self-reported activity, not verified completion",
    });
  } else {
    facts.push({ label: "Recipient", value: recipient });
    if (board?.kind === "task_broadcast") facts.push(...broadcastFacts(board));
    if (board?.kind === "agent_message" && board.ttl_secs !== null) {
      facts.push({
        label: "Delivery deadline",
        value: `${board.ttl_secs}s unless landed`,
      });
    }
    if (board?.kind === "agent_steer")
      facts.push({ label: "Reason", value: board.card.reason ?? null });
    if (args.expects_reply === true || name === "agent_ask_planner") {
      facts.push({
        label: "Reply",
        value: "Requested; this receipt is not an answer",
      });
    }
  }
  return {
    headline:
      compactText(
        body ??
          (launch
            ? "Requested work"
            : progress
              ? "Reported activity"
              : "Requested message"),
        COMPACT_HEADLINE_CHARS,
      ) ?? "Requested work",
    ...state,
    stateLabel:
      compactText(state.stateLabel, COMPACT_STATE_CHARS) ??
      UNCONFIRMED.stateLabel,
    facts: compactFacts(facts),
  };
}
