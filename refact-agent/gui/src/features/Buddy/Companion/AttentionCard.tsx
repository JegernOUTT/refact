import { useEffect, useRef, useState } from "react";
import { v4 as uuid } from "uuid";
import { Button } from "../../../components/ui";
import { QuestionResponseForm } from "../../../components/ChatContent/ToolCard/QuestionResponseForm";
import { useAppSelector } from "../../../hooks";
import { selectThreadTitleById } from "../../Chat/Thread";
import { selectConfig } from "../../Config/configSlice";
import {
  decideAttention,
  getDecisionReceipt,
  type AttentionRow,
  type AttentionSnapshot,
  type DecisionReceipt,
} from "../../../services/refact/buddyCompanion";
import { answerAttentionQuestion } from "../../../services/refact/buddyCompanion";
import type { QuestionDraft } from "../../../utils/questionResponses";
import { humanizeIdentifier } from "../../../utils/displayNames";
import { canDecide, sameRequest, visibleLiteral } from "./companionPolicy";
import styles from "./FloatingBuddy.module.css";

export type AttentionMemory = {
  receipt?: DecisionReceipt;
  intent?: { id: string; accepted: boolean | null; engine: string };
  error?: string;
  drafts: Partial<Record<string, QuestionDraft>>;
  comments: string;
  locked: boolean;
};
export function AttentionCard({
  row,
  snapshot,
  fresh,
  onNavigate,
  onAsk,
  onNext,
  memory,
  onMemory,
  onPersist,
}: {
  row: AttentionRow;
  snapshot: AttentionSnapshot;
  fresh: boolean;
  onNavigate: (chatId: string) => void;
  onAsk: (row: AttentionRow) => void;
  onNext: () => void;
  onPersist?: (intent: NonNullable<AttentionMemory["intent"]>) => void;
  memory: AttentionMemory;
  onMemory: (patch: Partial<AttentionMemory>) => void;
}) {
  const config = useAppSelector(selectConfig);
  const sourceTitle = useAppSelector((state) =>
    selectThreadTitleById(state, row.parent_chat_id ?? row.chat_id),
  );
  const { receipt, intent, error, drafts, comments } = memory;
  const setError = (value: string | undefined) => onMemory({ error: value });
  const setIntent = (value: AttentionMemory["intent"]) =>
    onMemory({ intent: value });
  const setDrafts = (value: AttentionMemory["drafts"]) =>
    onMemory({ drafts: value });
  const setComments = (value: string) => onMemory({ comments: value });
  const [submitting, setSubmitting] = useState(false);
  const onMemoryRef = useRef(onMemory);
  onMemoryRef.current = onMemory;
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  const current = snapshot.rows.find((candidate) => candidate.id === row.id);
  const stale = !fresh || !current || !sameRequest(row, current);
  const expired = row.expires_at != null && row.expires_at <= Date.now();
  const blocked = stale || expired || submitting || !!intent || memory.locked;
  useEffect(() => {
    if (!intent) return;
    let active = true;
    let polling = false;
    const poll = async () => {
      if (polling) return;
      polling = true;
      try {
        const result = await getDecisionReceipt(
          config,
          snapshot.project_root,
          row.chat_id,
          intent.id,
        );
        if (!active) return;
        if (
          result.chat_id !== row.chat_id ||
          result.client_request_id !== intent.id ||
          result.project_root !== snapshot.project_root ||
          result.engine_identity !== intent.engine ||
          result.target_id !== row.tool_call_id ||
          result.accepted !== intent.accepted ||
          (row.source_version != null &&
            result.source_version !== row.source_version)
        )
          throw new Error(
            "Receipt identity changed; inspect the original chat.",
          );
        onMemoryRef.current({ receipt: result, error: undefined });
      } catch (cause) {
        if (active)
          onMemoryRef.current({
            error: `Outcome unknown. No automatic resend. ${String(cause)}`,
          });
      } finally {
        polling = false;
      }
    };
    void poll();
    const timer = setInterval(() => {
      void poll();
    }, 2500);
    return () => {
      active = false;
      clearInterval(timer);
    };
  }, [
    intent,
    config,
    row.chat_id,
    row.source_version,
    row.tool_call_id,
    snapshot.project_root,
    snapshot.engine_identity,
  ]);
  const decide = async (accepted: boolean) => {
    if (blocked || memory.locked || !canDecide(row)) return;
    const next = { id: uuid(), accepted, engine: snapshot.engine_identity };
    try {
      onPersist?.(next);
    } catch (cause) {
      setError(String(cause));
      return;
    }
    memory.locked = true;
    onMemory({ locked: true });
    setIntent(next);
    setSubmitting(true);
    try {
      await decideAttention(config, snapshot, row, accepted, next.id);
    } catch (cause) {
      if (mounted.current)
        setError(`Decision outcome not confirmed: ${String(cause)}`);
    } finally {
      if (mounted.current) setSubmitting(false);
    }
  };
  return (
    <section
      className={styles.attention}
      aria-label="Selected attention request"
    >
      <strong>
        {sourceTitle ? sourceTitle : "Source chat"} —{" "}
        {humanizeIdentifier(row.kind)}
      </strong>
      <p>{visibleLiteral(row.reason)}</p>
      {row.display_command && (
        <details>
          <summary>Inspect command display (untrusted)</summary>
          <pre>{visibleLiteral(row.display_command)}</pre>
        </details>
      )}
      {row.literal_command && <pre>{visibleLiteral(row.literal_command)}</pre>}
      {row.tool_arguments != null && (
        <details>
          <summary>Exact tool arguments: {row.tool_name}</summary>
          <pre>
            {visibleLiteral(JSON.stringify(row.tool_arguments, null, 2))}
          </pre>
        </details>
      )}
      <p>
        Working directory:{" "}
        {row.cwd ? visibleLiteral(row.cwd) : "Unknown — not verified"}
      </p>
      {row.effect === "share_guarded_output" && (
        <p>
          The command already ran. Sharing releases its output to the model;
          withholding does not undo execution.
        </p>
      )}
      {(stale || expired) && (
        <p role="status">
          This selected request is no longer current. Its controls are disabled.
        </p>
      )}
      {receipt && (
        <p role="status">
          Decision: {humanizeIdentifier(receipt.status)}.{" "}
          {receipt.status === "queued" && "Queued is not applied."}{" "}
          {intent?.accepted === false &&
            `Refusal acknowledgement: ${humanizeIdentifier(receipt.followup)}.`}
        </p>
      )}
      {error && <p role="alert">{error}</p>}
      {!canDecide(row) && row.kind !== "question" && (
        <p>
          Review this request in its original chat. No safe compact approval
          adapter is available.
        </p>
      )}
      <div className={styles.actions}>
        <Button onClick={() => onNavigate(row.parent_chat_id ?? row.chat_id)}>
          Go to chat
        </Button>
        <Button onClick={() => onAsk(row)}>Ask about this request</Button>
        {canDecide(row) && (
          <>
            <Button
              disabled={blocked}
              onClick={() => {
                void decide(true);
              }}
            >
              {row.effect === "share_guarded_output"
                ? "Share output"
                : "Approve once"}
            </Button>
            <Button
              disabled={blocked}
              onClick={() => {
                void decide(false);
              }}
            >
              {row.effect === "share_guarded_output" ? "Withhold" : "Decline"}
            </Button>
          </>
        )}
        <Button onClick={onNext}>Next request</Button>
      </div>
      {row.kind === "question" && row.question && (
        <QuestionResponseForm
          instance={`buddy:${row.id}`}
          questions={row.question.questions}
          drafts={drafts}
          comments={comments}
          onDrafts={setDrafts}
          onComments={setComments}
          disabled={blocked}
          onSubmit={(response) => {
            if (blocked || !row.question || !row.tool_call_id) return;
            setSubmitting(true);
            if (memory.locked) return;
            const id = uuid();
            const next = {
              id,
              accepted: null,
              engine: snapshot.engine_identity,
            };
            try {
              onPersist?.(next);
            } catch (cause) {
              setSubmitting(false);
              setError(String(cause));
              return;
            }
            memory.locked = true;
            onMemory({
              locked: true,
              intent: next,
            });
            void answerAttentionQuestion(
              config,
              snapshot.project_root,
              row,
              response,
              id,
            )
              .catch((cause: unknown) => {
                if (mounted.current)
                  setError(
                    `Outcome unknown. No automatic resend. ${String(cause)}`,
                  );
              })
              .finally(() => {
                if (mounted.current) setSubmitting(false);
              });
          }}
        />
      )}
    </section>
  );
}
