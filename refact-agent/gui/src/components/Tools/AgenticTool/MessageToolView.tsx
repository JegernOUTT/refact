import { useState } from "react";
import { Button } from "../../ui";
import { Markdown } from "../../Markdown";
import { normalizeToolName } from "../../../utils/toolNameAliases";
import { parseAgenticArgs } from "./agenticToolModel";
import {
  boardReceipt,
  outcomeLabel,
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
          <li className={styles.agentRow} key={card.card_id}>
            <span className={styles.agentTitle}>
              {(card.title ?? card.card_id).slice(0, LABEL_CHARS)}
            </span>
            <span className={styles.agentDetail}>
              {outcomeLabel(card)}
              {card.reason ? ` (${card.reason.slice(0, LABEL_CHARS)})` : ""}
            </span>
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

const BODY_KEYS: Partial<Record<string, string>> = {
  progress_report: "status_line",
  agent_update: "status_line",
  agent_ask_planner: "question",
  planner_reply: "answer",
};

export function MessageToolView({
  toolName,
  argumentsRaw,
  content,
  extra,
}: {
  toolName: string;
  argumentsRaw: string | undefined;
  content: string | null;
  extra?: Record<string, unknown>;
}) {
  const name = normalizeToolName(toolName) ?? toolName;
  const { args } = parseAgenticArgs(argumentsRaw);
  const board = boardReceipt(name, extra);
  const body =
    text(args, BODY_KEYS[name] ?? "text") ?? text(args, "message") ?? null;
  return (
    <section aria-label="Agent message receipt" className={styles.section}>
      {board?.kind === "task_broadcast" && (
        <p>
          {board.notified} of {board.total} board agents reached
        </p>
      )}
      {board?.kind === "agent_steer" && <p>{outcomeLabel(board.card)}</p>}
      {board?.kind === "task_broadcast" && <CardReceipts cards={board.cards} />}
      {body !== null ? (
        <div data-testid="agentic-tool-message">
          <Markdown canHaveInteractiveElements={false}>{body}</Markdown>
        </div>
      ) : (
        content !== null && <p>{content}</p>
      )}
    </section>
  );
}
