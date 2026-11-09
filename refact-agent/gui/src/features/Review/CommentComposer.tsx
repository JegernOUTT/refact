import { useId, useState } from "react";
import { Send, X } from "lucide-react";

import { Button, IconButton, Tooltip } from "../../components/ui";
import {
  isReviewConflict,
  reviewErrorMessage,
  useAddReviewCommentMutation,
  type Anchor,
  type ReviewCommentRequest,
} from "../../services/refact/review";
import { newRequestKey } from "./model";
import { useReviewReadOnly } from "./readOnly";
import styles from "./Review.module.css";

export type CommentTarget =
  | { kind: "reply"; threadId: string }
  | { kind: "anchor"; anchor: Anchor; annotationId?: string };

export type CommentComposerProps = {
  reviewId: string;
  revision: number;
  target: CommentTarget;
  label: string;
  autoFocus?: boolean;
  onPosted?: () => void;
  onCancel?: () => void;
};

type Outcome =
  | { kind: "idle" }
  | { kind: "unknown"; request: ReviewCommentRequest; message: string }
  | { kind: "conflict" }
  | { kind: "refused"; message: string };

function buildRequest(
  reviewId: string,
  revision: number,
  target: CommentTarget,
  body: string,
): ReviewCommentRequest {
  return {
    reviewId,
    expected_revision: revision,
    request_key: newRequestKey(),
    body,
    ...(target.kind === "reply"
      ? { thread_id: target.threadId }
      : { anchor: target.anchor, annotation_id: target.annotationId }),
  };
}

function isDefiniteRefusal(error: unknown): boolean {
  if (typeof error !== "object" || error === null) return false;
  const status = (error as { status?: unknown }).status;
  return typeof status === "number" && status >= 400 && status < 500;
}

/**
 * While an attempt has an unknown outcome the exact request, including its
 * key and expected revision, is retried unchanged and the draft is locked, so
 * a committed-but-unacknowledged comment is replayed instead of duplicated.
 */
export function CommentComposer({
  reviewId,
  revision,
  target,
  label,
  autoFocus = false,
  onPosted,
  onCancel,
}: CommentComposerProps) {
  const inputId = useId();
  const [draft, setDraft] = useState("");
  const [outcome, setOutcome] = useState<Outcome>({ kind: "idle" });
  const [addComment, { isLoading }] = useAddReviewCommentMutation();
  const readOnly = useReviewReadOnly();
  const locked = outcome.kind === "unknown";

  const send = async (request: ReviewCommentRequest) => {
    const result = await addComment(request);
    if ("data" in result) {
      setOutcome({ kind: "idle" });
      setDraft("");
      onPosted?.();
      return;
    }
    if (isReviewConflict(result.error)) {
      setOutcome({ kind: "conflict" });
    } else if (isDefiniteRefusal(result.error)) {
      setOutcome({
        kind: "refused",
        message: reviewErrorMessage(result.error),
      });
    } else {
      setOutcome({
        kind: "unknown",
        request,
        message: reviewErrorMessage(result.error),
      });
    }
  };

  const submit = () => {
    if (isLoading || readOnly) return;
    if (outcome.kind === "unknown") {
      void send(outcome.request);
      return;
    }
    const body = draft.trim();
    if (!body) return;
    void send(buildRequest(reviewId, revision, target, body));
  };

  return (
    <form
      className={styles.composer}
      onSubmit={(event) => {
        event.preventDefault();
        submit();
      }}
    >
      <label htmlFor={inputId} className={styles.meta}>
        {label}
      </label>
      <textarea
        id={inputId}
        className={styles.composerInput}
        value={draft}
        readOnly={locked}
        aria-describedby={
          outcome.kind !== "idle" ? `${inputId}-status` : undefined
        }
        autoFocus={autoFocus}
        onChange={(event) => setDraft(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
            event.preventDefault();
            submit();
          } else if (event.key === "Escape" && onCancel && !locked) {
            event.preventDefault();
            onCancel();
          }
        }}
        placeholder="Write a comment… (Ctrl+Enter to send)"
      />
      {outcome.kind === "conflict" && (
        <p id={`${inputId}-status`} role="alert" className={styles.errorText}>
          The review changed before your comment was saved. Your draft is kept;
          check the update and send again.
        </p>
      )}
      {outcome.kind === "refused" && (
        <p id={`${inputId}-status`} role="alert" className={styles.errorText}>
          {outcome.message}
        </p>
      )}
      {outcome.kind === "unknown" && (
        <p id={`${inputId}-status`} role="alert" className={styles.errorText}>
          Not confirmed: {outcome.message}. It may already be saved; retry sends
          the identical request so it is never stored twice.
        </p>
      )}
      <div className={styles.composerActions}>
        {!locked && onCancel && (
          <Tooltip content="Cancel">
            <IconButton
              icon={X}
              aria-label="Cancel"
              size="sm"
              onClick={onCancel}
            />
          </Tooltip>
        )}
        {locked ? (
          <Button
            type="submit"
            variant="primary"
            size="sm"
            loading={isLoading}
            disabled={readOnly}
          >
            Retry
          </Button>
        ) : (
          <Tooltip content="Comment">
            <IconButton
              icon={Send}
              aria-label="Comment"
              type="submit"
              variant="primary"
              size="sm"
              loading={isLoading}
              disabled={readOnly || !draft.trim()}
            />
          </Tooltip>
        )}
      </div>
    </form>
  );
}
