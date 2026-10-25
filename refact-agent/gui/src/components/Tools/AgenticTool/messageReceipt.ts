const OUTCOMES = [
  "queued",
  "delivered",
  "duplicate",
  "no_live_session",
  "skipped",
  "excluded",
  "failed",
] as const;
type Outcome = (typeof OUTCOMES)[number];

export type CardReceipt = {
  card_id: string;
  title?: string;
  outcome: Outcome;
  reached: boolean;
  reason?: string | null;
};

export type BoardReceipt =
  | { kind: "agent_steer"; card: CardReceipt; prior_state?: string | null }
  | {
      kind: "task_broadcast";
      notified: number;
      total: number;
      cards: CardReceipt[];
    }
  | {
      kind: "agent_message";
      delivery_id: string;
      outcome:
        | "queued"
        | "delivered"
        | "consumed"
        | "duplicate"
        | "expired"
        | "superseded"
        | "failed";
      ttl_secs: number | null;
    };

function messageOutcomeLabel(
  outcome: Extract<BoardReceipt, { kind: "agent_message" }>["outcome"],
  ttl_secs: number | null,
): string {
  const ttl =
    ttl_secs === null ? "" : ` (expires after ${ttl_secs}s unless landed)`;
  switch (outcome) {
    case "queued":
      return `Message queued${ttl}`;
    case "delivered":
      return "Message landed at the recipient's boundary";
    case "consumed":
      return "Message consumed by the recipient";
    case "duplicate":
      return "Not queued: duplicate of an already-sent message";
    case "expired":
      return "Not queued: the deadline passed before any boundary opened";
    case "superseded":
      return "Not queued: replaced by a newer message with the same key";
    case "failed":
      return "Not queued: delivery failed before consumption";
  }
}

export function outcomeLabel(receipt: CardReceipt): string {
  switch (receipt.outcome) {
    case "queued":
      return "Queued for the agent";
    case "delivered":
      return "Delivered to the agent";
    case "duplicate":
      return "Not delivered: duplicate of an earlier message";
    case "no_live_session":
      return "Not delivered: no live agent session";
    case "excluded":
      return "Skipped: excluded by request";
    case "skipped":
      return "Not delivered: skipped";
    case "failed":
      return "Not delivered: failed";
  }
}

function cardReceipt(value: unknown): CardReceipt | null {
  if (typeof value !== "object" || value === null) return null;
  const row = value as Record<string, unknown>;
  const outcome = OUTCOMES.find((candidate) => candidate === row.outcome);
  if (
    typeof row.card_id !== "string" ||
    row.card_id === "" ||
    outcome === undefined ||
    typeof row.reached !== "boolean" ||
    (row.title !== undefined && typeof row.title !== "string") ||
    (row.reason !== undefined &&
      row.reason !== null &&
      typeof row.reason !== "string")
  )
    return null;
  const consistent =
    row.reached === (outcome === "queued" || outcome === "delivered");
  if (!consistent) return null;
  return {
    card_id: row.card_id,
    title: typeof row.title === "string" ? row.title : undefined,
    outcome,
    reached: row.reached,
    reason: typeof row.reason === "string" ? row.reason : null,
  };
}

export function boardReceipt(
  name: string | undefined,
  extra: Readonly<Record<string, unknown>> | undefined,
): BoardReceipt | null {
  if (!extra) return null;
  if (name === "agent_steer") {
    const raw = extra.delivery_receipt;
    if (typeof raw !== "object" || raw === null) return null;
    const row = raw as Record<string, unknown>;
    const card = cardReceipt(row);
    if (row.kind !== "agent_steer" || card === null) return null;
    return {
      kind: "agent_steer",
      card,
      prior_state: typeof row.prior_state === "string" ? row.prior_state : null,
    };
  }
  if (name === "agent_message") {
    const raw = extra.delivery_receipt;
    if (typeof raw !== "object" || raw === null) return null;
    const row = raw as Record<string, unknown>;
    const outcome = row.outcome;
    if (
      row.kind !== "agent_message" ||
      typeof row.delivery_id !== "string" ||
      row.delivery_id === "" ||
      typeof outcome !== "string" ||
      ![
        "queued",
        "delivered",
        "consumed",
        "duplicate",
        "expired",
        "superseded",
        "failed",
      ].includes(outcome) ||
      !(
        row.ttl_secs === null ||
        row.ttl_secs === undefined ||
        (typeof row.ttl_secs === "number" &&
          Number.isSafeInteger(row.ttl_secs) &&
          row.ttl_secs >= 1)
      )
    )
      return null;
    return {
      kind: "agent_message",
      delivery_id: row.delivery_id,
      outcome: outcome as Extract<
        BoardReceipt,
        { kind: "agent_message" }
      >["outcome"],
      ttl_secs: typeof row.ttl_secs === "number" ? row.ttl_secs : null,
    };
  }
  if (name === "task_broadcast") {
    const raw = extra.delivery_receipts;
    if (typeof raw !== "object" || raw === null) return null;
    const row = raw as Record<string, unknown>;
    if (
      row.kind !== "task_broadcast" ||
      !Array.isArray(row.cards) ||
      typeof row.notified !== "number" ||
      typeof row.total !== "number" ||
      !Number.isSafeInteger(row.notified) ||
      !Number.isSafeInteger(row.total) ||
      row.notified < 0 ||
      row.total < row.notified
    )
      return null;
    const cards = row.cards.map(cardReceipt);
    if (cards.some((card) => card === null) || cards.length !== row.total)
      return null;
    const reached = cards.filter((card) => card?.reached).length;
    if (reached !== row.notified) return null;
    return {
      kind: "task_broadcast",
      notified: row.notified,
      total: row.total,
      cards: cards as CardReceipt[],
    };
  }
  return null;
}

export function text(
  args: Readonly<Record<string, unknown>>,
  key: string,
): string | null {
  const value = args[key];
  return typeof value === "string" && value.trim() !== "" ? value.trim() : null;
}

export function receiptState(
  name: string | undefined,
  args: Readonly<Record<string, unknown>>,
  content: string | null,
  board: BoardReceipt | null,
): { label: string; question: string | null } {
  const unknown = { label: "Outcome unconfirmed", question: null };
  if (board?.kind === "agent_steer") {
    return { label: outcomeLabel(board.card), question: null };
  }
  if (board?.kind === "task_broadcast") {
    return {
      label: `${board.notified} of ${board.total} board agents reached`,
      question: null,
    };
  }
  if (board?.kind === "agent_message") {
    return {
      label: messageOutcomeLabel(board.outcome, board.ttl_secs),
      question: null,
    };
  }
  if (name === "progress_report") {
    const status = text(args, "status_line");
    return status !== null && content === `Progress updated: ${status}`
      ? { label: "Progress recorded", question: null }
      : unknown;
  }
  if (name !== "agent_message") return unknown;
  const recipient = text(args, "to");
  if (recipient === null || text(args, "text") === null) return unknown;
  if (recipient === "parent") {
    if (content === "Note sent to parent.")
      return { label: "Note accepted", question: null };
    const question = /^Question ([A-Za-z0-9_-]{1,240}) sent to parent\.$/u.exec(
      content ?? "",
    );
    return args.expects_reply === true && question
      ? { label: "Question recorded", question: question[1] }
      : unknown;
  }
  if (content === `Message queued for ${recipient}.`)
    return { label: "Message queued", question: null };
  return text(args, "reply_to") !== null &&
    content === `Answer recorded for finished agent ${recipient}.`
    ? { label: "Answer recorded", question: null }
    : unknown;
}
