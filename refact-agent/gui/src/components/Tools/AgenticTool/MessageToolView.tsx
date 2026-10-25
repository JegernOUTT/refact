import { useState } from "react";
import { Badge, Button } from "../../ui";
import { normalizeToolName } from "../../../utils/toolNameAliases";
import { parseAgenticArgs } from "./agenticToolModel";
import { PagedText } from "./PagedText";
import {
  boardReceipt,
  outcomeLabel,
  receiptState,
  text,
  type CardReceipt,
} from "./messageReceipt";
import styles from "./AgenticTool.module.css";

const LABEL_CHARS = 240;
const CARD_PAGE = 5;

function CardReceipts({ cards }: { cards: CardReceipt[] }) {
  const [page, setPage] = useState(0);
  const pages = Math.max(1, Math.ceil(cards.length / CARD_PAGE));
  const index = Math.min(page, pages - 1);
  return (
    <section aria-label="Board delivery receipts" className={styles.section}>
      <ul className={styles.agentList}>
        {cards.slice(index * CARD_PAGE, (index + 1) * CARD_PAGE).map((card) => (
          <li key={card.card_id}>
            <span className={styles.detailLabel}>
              {(card.title ?? card.card_id).slice(0, LABEL_CHARS)}
            </span>
            {": "}
            {outcomeLabel(card)}
            {card.reason ? ` (${card.reason.slice(0, LABEL_CHARS)})` : ""}
          </li>
        ))}
      </ul>
      {pages > 1 && (
        <nav
          aria-label="Board delivery receipt pages"
          className={styles.actions}
        >
          <Button
            size="sm"
            disabled={index === 0}
            onClick={() => setPage(index - 1)}
          >
            Previous cards
          </Button>
          <span role="status">
            Cards {index + 1} of {pages}
          </span>
          <Button
            size="sm"
            disabled={index + 1 === pages}
            onClick={() => setPage(index + 1)}
          >
            Next cards
          </Button>
        </nav>
      )}
    </section>
  );
}

export function MessageToolView({
  toolName,
  argumentsRaw,
  content,
  extra,
  pending,
  failed,
}: {
  toolName: string;
  argumentsRaw: string | undefined;
  content: string | null;
  extra?: Record<string, unknown>;
  pending: boolean;
  failed: boolean;
}) {
  const name = normalizeToolName(toolName);
  const { args, malformed } = parseAgenticArgs(argumentsRaw);
  const board = pending || failed ? null : boardReceipt(name, extra);
  const progress = name === "progress_report";
  const body = text(
    args,
    progress ? "status_line" : name === "agent_message" ? "text" : "message",
  );
  const recipient = progress
    ? "Work status"
    : name === "task_broadcast"
      ? "Eligible board agents"
      : text(args, name === "agent_steer" ? "card_id" : "to");
  const receipt = receiptState(name, args, content, board);
  const replyTo = text(args, "reply_to");
  return (
    <section
      aria-label={progress ? "Progress receipt" : "Agent message receipt"}
      className={styles.section}
    >
      <div className={styles.metaRow}>
        <Badge tone="muted">Historical interaction receipt</Badge>
        <Badge tone={failed ? "danger" : "muted"}>
          {pending
            ? "Awaiting receipt"
            : failed
              ? "Request failed"
              : receipt.label}
        </Badge>
      </div>
      {pending && <p role="status">No interaction outcome confirmed yet.</p>}
      {(failed || malformed) && (
        <p role="alert">
          Interaction failed or its request could not be read. Do not assume
          delivery.
        </p>
      )}
      <dl className={styles.detailGrid}>
        <div className={styles.detailItem}>
          <dt className={styles.detailLabel}>
            {progress ? "Destination" : "Requested recipient"}
          </dt>
          <dd className={styles.detailValue}>
            {recipient?.slice(0, LABEL_CHARS) ?? "Unavailable"}
          </dd>
        </div>
        {replyTo && (
          <div className={styles.detailItem}>
            <dt className={styles.detailLabel}>Reply correlation</dt>
            <dd className={styles.detailValue}>
              {replyTo.slice(0, LABEL_CHARS)}
            </dd>
          </div>
        )}
        {!pending && !failed && receipt.question && (
          <div className={styles.detailItem}>
            <dt className={styles.detailLabel}>Question ID</dt>
            <dd className={styles.detailValue}>{receipt.question}</dd>
          </div>
        )}
      </dl>
      {args.expects_reply === true && (
        <p>
          A reply was requested. This receipt is not an answer and does not
          prove that a blocking wait started.
        </p>
      )}
      {board?.kind === "agent_steer" && board.prior_state && (
        <dl className={styles.detailGrid}>
          <div className={styles.detailItem}>
            <dt className={styles.detailLabel}>Agent state before steering</dt>
            <dd className={styles.detailValue}>
              {board.prior_state.slice(0, LABEL_CHARS)}
            </dd>
          </div>
        </dl>
      )}
      {board?.kind === "agent_steer" && board.card.reason && (
        <p role="status">{board.card.reason.slice(0, LABEL_CHARS)}</p>
      )}
      {board?.kind === "task_broadcast" && <CardReceipts cards={board.cards} />}
      {body !== null && (
        <div data-testid="agentic-tool-message">
          <PagedText
            label={progress ? "Reported progress" : "Requested message"}
            body={body}
          />
        </div>
      )}
      {!pending &&
        content !== null &&
        (failed || receipt.label === "Outcome unconfirmed") && (
          <PagedText label="Unconfirmed response" body={content} />
        )}
      <p className={styles.note}>
        {progress
          ? "Progress is self-reported activity, not verified completion."
          : "Accepted or queued mail is not proof of consumption, an answer, or completed work. A message cannot grant permissions."}
      </p>
    </section>
  );
}
