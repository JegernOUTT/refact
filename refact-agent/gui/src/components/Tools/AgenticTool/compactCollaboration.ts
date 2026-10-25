import { JSON_SCHEMA, load } from "js-yaml";
import { normalizeToolName } from "../../../utils/toolNameAliases";
import { LOCAL_COLLABORATION_TOOL_NAMES } from "./agenticFamilies";
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
type Receipt = Pick<CompactAgenticPresentation, "headline" | "stateLabel"> & {
  facts?: CompactAgenticPresentation["facts"];
  tone?: CompactAgenticPresentation["tone"];
};

const INTENTS: Partial<Record<string, { headline: string; subject?: string }>> =
  {
    agent_mailbox: { headline: "Mailbox check requested" },
    board_claim: { headline: "Claim requested", subject: "card_id" },
    board_discover: {
      headline: "Board discovery requested",
      subject: "task_id",
    },
    team_seat: { headline: "Team seat change requested", subject: "team" },
    team_role_grant: { headline: "Role grant requested", subject: "team" },
    finalize_card: { headline: "Finalization requested", subject: "card_id" },
    claim_record: {
      headline: "Claim recording requested",
      subject: "predicate",
    },
    claim_list: { headline: "Claim lookup requested", subject: "subject" },
    disagreement_file: {
      headline: "Disagreement filing requested",
      subject: "claim",
    },
    disagreement_list: { headline: "Disagreement lookup requested" },
    disagreement_adjudicate: { headline: "Adjudication requested" },
    causal_trace: { headline: "Causal trace requested" },
    metrics: { headline: "Project metrics requested" },
    code_mode: { headline: "Orchestration requested" },
  };

const SEAT_INTENTS: Partial<Record<string, string>> = {
  claim: "Team seat claim requested",
  reclaim: "Team seat reclaim requested",
  heartbeat: "Seat heartbeat requested",
  release: "Team seat release requested",
  record_outcome: "Seat outcome recording requested",
};

const ERROR_LABELS: Partial<Record<string, string>> = {
  already_claimed: "Claim unavailable",
  dependency_unsatisfied: "Dependencies unmet",
  scope_denied: "Request denied",
  not_found_or_not_visible: "Not found or not visible",
  policy_unavailable: "Policy unavailable",
  unavailable: "Unavailable",
  cursor_expired: "Snapshot expired",
  invalid_cursor: "Invalid continuation",
  invalid_argument: "Invalid request",
};

function text(record: RecordValue, key: string): string | null {
  const value = record?.[key];
  return typeof value === "string" ? compactText(value) : null;
}

function count(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

function sameText(a: RecordValue, b: RecordValue, key: string): boolean {
  return text(a, key) !== null && a?.[key] === b?.[key];
}

function discoveryResult(value: unknown): RecordValue {
  if (typeof value !== "string") return readCompactRecord(value);
  try {
    return readCompactRecord(load(value, { schema: JSON_SCHEMA }));
  } catch {
    return null;
  }
}

function requestFacts(name: string, args: RecordValue) {
  if (name === "team_seat" || name === "team_role_grant") {
    return compactFacts([
      { label: "Requested team", value: text(args, "team") },
      { label: "Requested role", value: text(args, "role") },
      {
        label:
          name === "team_seat" ? "Requested outcome" : "Requested authority",
        value: text(args, name === "team_seat" ? "outcome" : "authority"),
      },
    ]);
  }
  return compactFacts([
    { label: "Requested card", value: text(args, "card_id") },
    { label: "Requested subject", value: text(args, "subject") },
    { label: "Requested assertion", value: text(args, "assertion") },
    { label: "Requested claim", value: text(args, "claim") },
    { label: "Requested revision", value: text(args, "current_revision") },
    { label: "Requested task", value: text(args, "task_id") },
    { label: "Requested event", value: text(args, "event_id") },
    { label: "Replay requested", value: args?.replay },
  ]);
}

function recognizedError(result: RecordValue): string | null {
  if (
    result === null ||
    typeof result.code !== "string" ||
    text(result, "message") === null ||
    typeof result.retryable !== "boolean" ||
    text(result, "request_id") === null
  )
    return null;
  const label = ERROR_LABELS[result.code];
  return typeof label === "string" ? label : null;
}

function failureLabel(name: string, result: unknown): string {
  if (typeof result !== "string") return "Request failed";
  if (
    (name === "code_mode" && result.startsWith("code mode is unavailable: ")) ||
    (name === "agent_mailbox" &&
      result === "mailbox privacy policy is unavailable") ||
    ((name === "claim_record" || name === "claim_list") &&
      result ===
        "no coordination store is available, so claims cannot be recorded or read") ||
    (name.startsWith("disagreement_") &&
      result ===
        "no coordination store is available, so disagreements cannot be filed or read")
  )
    return "Unavailable";
  if (name === "code_mode" && result.startsWith("code mode refused: "))
    return "Request denied";
  return "Request failed";
}

function recognizedReceipt(
  name: string,
  args: RecordValue,
  result: RecordValue,
  extra: RecordValue,
): Receipt | null {
  if (
    name === "board_claim" &&
    result?.type === "task_board_claim" &&
    sameText(result, args, "card_id") &&
    text(result, "task_id") !== null &&
    (args?.task_id === undefined || sameText(result, args, "task_id")) &&
    text(result, "claimant_chat_id") !== null &&
    count(result.rev)
  ) {
    if (result.action === "release" && args?.release === true)
      return {
        headline: "Card claim released",
        stateLabel: "Release recorded",
      };
    if (
      result.action === "claim" &&
      (args?.release === undefined || args.release === false) &&
      typeof result.already_held === "boolean" &&
      text(result, "idempotency_key") !== null &&
      (args?.idempotency_key === undefined ||
        sameText(result, args, "idempotency_key"))
    )
      return {
        headline: result.already_held
          ? "Card claim already held"
          : "Card claim recorded",
        stateLabel: "Claim receipt",
      };
  }
  if (name === "board_discover" && result !== null) {
    if (
      args?.task_id === undefined &&
      typeof result.projection_unavailable === "boolean" &&
      Array.isArray(result.cards)
    )
      return result.projection_unavailable
        ? {
            headline: "Board discovery unavailable",
            stateLabel: "Projection unavailable",
            tone: "warning",
          }
        : {
            headline: "Board snapshot checked",
            stateLabel: "Metadata only",
            facts: compactFacts([
              { label: "Cards shown", value: result.cards.length },
            ]),
          };
    if (
      sameText(result, args, "task_id") &&
      count(result.rev) &&
      Array.isArray(result.reviewer_chat_ids) &&
      readCompactRecord(result.claims) !== null &&
      readCompactRecord(result.dependency_overrides) !== null &&
      (result.review_policy === "owner_allowed" ||
        result.review_policy === "independent_required")
    )
      return { headline: "Board roster checked", stateLabel: "Metadata only" };
  }
  if (
    name === "finalize_card" &&
    result?.type === "task_card_finalize" &&
    sameText(result, args, "card_id") &&
    text(result, "task_id") !== null &&
    (args?.task_id === undefined || sameText(result, args, "task_id"))
  ) {
    const blocks = readCompactRecord(result.decision)?.blocks;
    if (Array.isArray(blocks)) {
      if (result.finalized === true && blocks.length === 0)
        return {
          headline: "Card finalization recorded",
          stateLabel: "Finalization receipt",
        };
      if (result.finalized === false && blocks.length > 0)
        return {
          headline: "Card finalization refused",
          stateLabel: "Requirements unmet",
          tone: "warning",
          facts: compactFacts([
            { label: "Unmet requirements", value: blocks.length },
          ]),
        };
    }
  }
  if (name === "agent_mailbox") {
    const mailbox = readCompactRecord(extra?.mailbox);
    if (
      mailbox !== null &&
      count(mailbox.total) &&
      count(mailbox.offset) &&
      count(mailbox.limit) &&
      mailbox.limit > 0 &&
      Array.isArray(mailbox.rows) &&
      mailbox.rows.length <= mailbox.limit &&
      mailbox.rows.length <= mailbox.total
    )
      return {
        headline: "Mailbox checked",
        stateLabel: "Snapshot received",
        facts: compactFacts([
          { label: "Deliveries shown", value: mailbox.rows.length },
          { label: "Visible total", value: mailbox.total },
          { label: "Offset", value: mailbox.offset },
        ]),
      };
  }
  if (name === "team_seat") {
    const seat = readCompactRecord(extra?.team_seat);
    if (
      seat !== null &&
      sameText(seat, args, "team") &&
      sameText(seat, args, "role") &&
      sameText(seat, args, "action") &&
      text(seat, "member_chat_id") !== null &&
      Array.isArray(seat.members)
    ) {
      const action = seat.action;
      if (
        (action === "claim" || action === "reclaim") &&
        (seat.result === "claimed" || seat.result === "already held")
      )
        return {
          headline: "Team seat claim recorded",
          stateLabel: "Seat receipt",
        };
      if (action === "release" && seat.result === "released")
        return {
          headline: "Team seat release recorded",
          stateLabel: "Seat receipt",
        };
      if (action === "heartbeat" && seat.result === "heartbeat recorded")
        return {
          headline: "Seat heartbeat recorded",
          stateLabel: "Seat receipt",
        };
      if (
        action === "record_outcome" &&
        seat.result === "outcome recorded" &&
        sameText(seat, args, "outcome")
      )
        return {
          headline: "Seat outcome recorded",
          stateLabel: "Self-reported outcome",
        };
    }
  }
  if (name === "team_role_grant") {
    const grant = readCompactRecord(extra?.team_role_grant);
    if (
      grant !== null &&
      sameText(grant, args, "team") &&
      sameText(grant, args, "role") &&
      sameText(grant, args, "authority") &&
      ["metadata", "mail", "control"].includes(String(grant.authority)) &&
      ["member", "recipient"].every((prefix) => {
        const agent = `${prefix}_agent_id`;
        const chat = `${prefix}_chat_id`;
        return (
          (sameText(grant, args, agent) &&
            grant[chat] === null &&
            args?.[chat] === undefined) ||
          (sameText(grant, args, chat) &&
            grant[agent] === null &&
            args?.[agent] === undefined)
        );
      })
    )
      return {
        headline: "Role grant receipt recorded",
        stateLabel: "Historical grant receipt",
        facts: compactFacts([
          { label: "Team", value: text(grant, "team") },
          { label: "Role", value: text(grant, "role") },
          {
            label: "Authority",
            value: "Historical receipt; not current permission",
          },
        ]),
      };
  }
  if (
    name === "metrics" &&
    result !== null &&
    text(result, "snapshot_id") !== null &&
    count(result.seq) &&
    text(result, "project") !== null &&
    Array.isArray(result.entries) &&
    count(result.total) &&
    count(result.series_total) &&
    count(result.explanations_total) &&
    result.series_total + result.explanations_total === result.total &&
    count(result.returned) &&
    result.returned === result.entries.length &&
    result.returned <= result.total &&
    typeof result.truncated === "boolean" &&
    count(result.generated_at_ms)
  )
    return {
      headline: "Project metrics checked",
      stateLabel: "Snapshot received",
      facts: compactFacts([
        { label: "Entries shown", value: result.returned },
        { label: "Metric series", value: result.series_total },
        { label: "Decision explanations", value: result.explanations_total },
      ]),
    };
  return null;
}

export function buildCompactCollaborationPresentation(
  input: CompactAgenticInput,
): CompactAgenticPresentation | null {
  const name = normalizeToolName(input.toolName);
  if (!name || !LOCAL_COLLABORATION_TOOL_NAMES.includes(name)) return null;
  const intent = INTENTS[name];
  if (!intent) return null;
  const args = readCompactRecord(input.arguments);
  const result =
    name === "board_discover"
      ? discoveryResult(input.result)
      : readCompactRecord(input.result);
  const extra = readCompactRecord(input.extra);
  const subject = intent.subject ? text(args, intent.subject) : null;
  const seatIntent = SEAT_INTENTS[text(args, "action") ?? ""];
  const requested =
    name === "board_claim" && args?.release === true
      ? "Claim release requested"
      : name === "team_seat"
        ? typeof seatIntent === "string"
          ? seatIntent
          : intent.headline
        : intent.headline;
  const headline =
    compactText(
      subject ? `${requested}: ${subject}` : requested,
      COMPACT_HEADLINE_CHARS,
    ) ?? requested;
  const base: CompactAgenticPresentation = {
    headline,
    ...compactTransportPresentation(input.transportState),
    facts: requestFacts(name, args),
  };
  if (input.transportState === "pending" || input.transportState === "running")
    return base;
  const error = recognizedError(result);
  if (error !== null || input.transportState === "failed")
    return {
      ...base,
      stateLabel: error ?? failureLabel(name, input.result),
      tone: "danger",
      facts: compactFacts([
        {
          label: "Error",
          value: text(result, "message") ?? compactText(input.result),
        },
        ...base.facts,
      ]),
    };
  if (input.transportState !== "succeeded") return base;
  const receipt = recognizedReceipt(name, args, result, extra);
  if (receipt === null) return { ...base, stateLabel: "Outcome unconfirmed" };
  const target =
    name === "board_claim" || name === "finalize_card"
      ? text(args, "card_id")
      : name === "team_seat"
        ? text(args, "team")
        : null;
  return {
    ...base,
    ...receipt,
    headline:
      compactText(
        target ? `${receipt.headline}: ${target}` : receipt.headline,
        COMPACT_HEADLINE_CHARS,
      ) ?? receipt.headline,
    facts: receipt.facts ?? base.facts,
  };
}
