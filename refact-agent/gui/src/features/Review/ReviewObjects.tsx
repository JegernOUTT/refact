import { useContext, useEffect, useId, useRef, useState } from "react";
import {
  Bot,
  CircleUserRound,
  MessageCircle,
  Reply,
  Settings2,
} from "lucide-react";

import { Badge, Button, Icon } from "../../components/ui";
import { Markdown } from "../../components/Markdown";
import {
  type Annotation,
  type Review,
  type ReviewThread,
} from "../../services/refact/review";
import {
  ReviewRichBlocks,
  SharedReviewComposer,
  ReviewComposerContext,
  RichAncestorThreads,
} from "./ReviewRichBlocks";
import { ObjectHistory } from "./ObjectHistory";
import { usePendingDeliveries } from "./diff/usePendingDeliveries";
import {
  APPLICABILITY_TONE,
  ASSESSMENT_TONE,
  DISPOSITION_TONE,
  SEVERITY_TONE,
  formatTime,
  label,
} from "./labels";
import { anchorsIn, firstAnchorOf } from "./model";
import styles from "./Review.module.css";

function useReviewComposerOwnership() {
  const composer = useContext(ReviewComposerContext);
  const ownerId = useId();
  const [localOpen, setLocalOpen] = useState(false);
  const setActiveKey = composer?.setActiveKey;
  const trigger = useRef<HTMLButtonElement>(null);
  const restoreFocus = useRef(false);
  const [focusOnOpen, setFocusOnOpen] = useState(false);
  const open = composer ? composer.activeKey === ownerId : localOpen;
  useEffect(() => {
    if (!open && restoreFocus.current) {
      restoreFocus.current = false;
      trigger.current?.focus();
    }
  }, [open]);
  useEffect(
    () => () => {
      setActiveKey?.((current) => (current === ownerId ? null : current));
    },
    [ownerId, setActiveKey],
  );
  return {
    ownerId,
    open,
    trigger,
    focusOnOpen,
    setFocusOnOpen,
    restoreFocus,
    setOpen: (value: boolean) => {
      if (setActiveKey)
        setActiveKey((current) =>
          value ? ownerId : current === ownerId ? null : current,
        );
      else setLocalOpen(value);
    },
  };
}

export type ReviewObjectsProps = {
  review: Review;
  candidateId: string;
  path: string;
  fallbackOnly?: boolean;
  placedIds?: ReadonlySet<string>;
  activeObjectId: string | null;
  onFocusObject: (objectId: string) => void;
};

function rangeLabel(annotationOrThread: { anchors: Annotation["anchors"] }) {
  const anchor = firstAnchorOf(annotationOrThread.anchors);
  if (!anchor) return "";
  const { side, start_line, end_line } = anchor.position;
  const lines =
    start_line === end_line ? `L${start_line}` : `L${start_line}–${end_line}`;
  return `${label(side)} ${lines}`;
}

export function AnnotationCard({
  review,
  annotation,
  active,
  onFocus,
}: {
  review: Review;
  annotation: Annotation;
  active: boolean;
  onFocus: () => void;
}) {
  const {
    ownerId,
    open: replying,
    setOpen: setReplying,
    trigger,
    focusOnOpen,
    setFocusOnOpen,
    restoreFocus,
  } = useReviewComposerOwnership();
  const [visitedReply, setVisitedReply] = useState(false);
  const anchor = firstAnchorOf(annotation.anchors);
  const hasThread = review.threads.some(
    (t) => t.annotation_id === annotation.id,
  );
  return (
    <article
      className={styles.object}
      data-active={active}
      aria-label={`${label(annotation.kind)}: ${annotation.title}`}
    >
      <div className={styles.objectHead}>
        <button type="button" className={styles.objectTitle} onClick={onFocus}>
          {annotation.title}
        </button>
        {annotation.kind === "finding" && (
          <Badge tone={SEVERITY_TONE[annotation.severity]} size="xs">
            {label(annotation.severity)}
          </Badge>
        )}
        <Badge tone={ASSESSMENT_TONE[annotation.assessment]} size="xs">
          {label(annotation.assessment)}
        </Badge>
        <Badge tone={DISPOSITION_TONE[annotation.disposition]} size="xs">
          {label(annotation.disposition)}
        </Badge>
        {annotation.applicability !== "current" && (
          <Badge tone={APPLICABILITY_TONE[annotation.applicability]} size="xs">
            {label(annotation.applicability)}
          </Badge>
        )}
      </div>
      <span className={styles.meta}>
        {label(annotation.kind)} · {rangeLabel(annotation)} · confidence{" "}
        {label(annotation.confidence).toLowerCase()}
        {annotation.evidence_ids.length > 0 &&
          ` · ${annotation.evidence_ids.length} evidence`}
      </span>
      <Markdown>{annotation.body}</Markdown>
      <ObjectHistory
        reviewId={review.id}
        current={{ kind: "annotation", value: annotation }}
      />
      {!hasThread && anchor && (
        <>
          {visitedReply && (
            <SharedReviewComposer
              reviewId={review.id}
              revision={review.revision}
              anchor={anchor}
              annotationId={annotation.id}
              label={`Discuss “${annotation.title}”`}
              ownerId={ownerId}
              open={replying}
              focusOnOpen={focusOnOpen}
              onClose={() => {
                restoreFocus.current = true;
                setReplying(false);
              }}
            />
          )}
          {!replying && (
            <div className={styles.composerActions}>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                ref={trigger}
                onClick={(event) => {
                  setFocusOnOpen(event.detail === 0);
                  setVisitedReply(true);
                  setReplying(true);
                }}
              >
                <Icon icon={MessageCircle} size="sm" /> Discuss
              </Button>
            </div>
          )}
        </>
      )}
    </article>
  );
}

function ContributionBody({ body }: { body: string }) {
  const [expanded, setExpanded] = useState(false);
  const bodyId = useId();
  const collapsible = body.length > 600 || body.split("\n").length > 6;
  return (
    <>
      <div
        id={bodyId}
        className={styles.replyBody}
        data-clamped={collapsible && !expanded}
      >
        <Markdown>{body}</Markdown>
      </div>
      {collapsible && (
        <div>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            aria-controls={bodyId}
            aria-expanded={expanded}
            onClick={() => setExpanded((value) => !value)}
          >
            {expanded ? "Show less" : "Show full reply"}
          </Button>
        </div>
      )}
    </>
  );
}

export function ThreadCard({
  review,
  thread,
  active,
  pending,
  onFocus,
}: {
  review: Review;
  thread: ReviewThread;
  active: boolean;
  pending: ReadonlySet<string>;
  onFocus: () => void;
}) {
  const {
    ownerId,
    open: replying,
    setOpen: setReplying,
    trigger,
    focusOnOpen,
    setFocusOnOpen,
    restoreFocus,
  } = useReviewComposerOwnership();
  const [visitedReply, setVisitedReply] = useState(false);
  const ancestors = useContext(RichAncestorThreads);
  const excluded = new Set([...ancestors, thread.id]);
  const annotation = thread.annotation_id
    ? review.annotations.find((a) => a.id === thread.annotation_id)
    : undefined;
  return (
    <article
      className={styles.object}
      data-active={active}
      data-review-thread-id={thread.id}
      aria-label={`Comment thread ${rangeLabel(thread)}`}
    >
      <div className={styles.objectHead}>
        <button type="button" className={styles.objectTitle} onClick={onFocus}>
          {annotation ? `Re: ${annotation.title}` : "Comment thread"}
        </button>
        <span className={styles.meta}>{rangeLabel(thread)}</span>
        {thread.resolved && (
          <Badge tone="muted" size="xs">
            Resolved
          </Badge>
        )}
      </div>
      {[...thread.contributions]
        .sort((a, b) => a.created_at_ms - b.created_at_ms)
        .map((c) => (
          <div
            key={c.id}
            className={styles.contribution}
            data-actor={c.actor.kind}
          >
            <span className={styles.contributionHead}>
              <Icon
                icon={
                  c.actor.kind === "user"
                    ? CircleUserRound
                    : c.actor.kind === "agent"
                      ? Bot
                      : Settings2
                }
                size="sm"
              />
              <span className={styles.meta}>
                <strong className={styles.actorName}>
                  {c.actor.kind === "user" ? "You" : label(c.actor.kind)}
                </strong>{" "}
                · {formatTime(c.created_at_ms)}
                {c.context?.candidate_id &&
                  c.context.candidate_id !== review.current_candidate_id &&
                  " · on an earlier candidate"}
                {c.context?.run_id && ` · run ${c.context.run_id}`}
                {c.context &&
                  c.context.applicability !== "current" &&
                  ` · ${label(c.context.applicability).toLowerCase()}`}
                {pending.has(c.id) &&
                  " · Saved to review · awaiting chat delivery"}
              </span>
            </span>
            {c.blocks?.length ? (
              <RichAncestorThreads.Provider value={excluded}>
                <ReviewRichBlocks
                  review={review}
                  blocks={c.blocks}
                  body={c.body}
                  bindingKey={`${review.id}:${thread.id}:${c.id}`}
                />
              </RichAncestorThreads.Provider>
            ) : (
              <ContributionBody body={c.body} />
            )}
          </div>
        ))}
      <ObjectHistory
        reviewId={review.id}
        current={{ kind: "thread", value: thread }}
        compact
      />
      {visitedReply && (
        <div hidden={!replying}>
          <SharedReviewComposer
            reviewId={review.id}
            revision={review.revision}
            threadId={thread.id}
            ownerId={ownerId}
            open={replying}
            focusOnOpen={focusOnOpen}
            onClose={() => {
              restoreFocus.current = true;
              setReplying(false);
            }}
          />
        </div>
      )}
      {!replying && (
        <div className={styles.composerActions}>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            ref={trigger}
            onClick={(event) => {
              setFocusOnOpen(event.detail === 0);
              setVisitedReply(true);
              setReplying(true);
            }}
          >
            <Icon icon={Reply} size="sm" /> Reply
          </Button>
        </div>
      )}
    </article>
  );
}

/** Inline annotations and comment threads for the visible file. */
export function ReviewObjects({
  review,
  candidateId,
  path,
  fallbackOnly = false,
  placedIds,
  activeObjectId,
  onFocusObject,
}: ReviewObjectsProps) {
  const annotations = review.annotations.filter(
    (a) =>
      anchorsIn(a.anchors, candidateId, path).length > 0 &&
      (!fallbackOnly ||
        !(
          placedIds?.has(`annotation:${a.id}`) ??
          a.anchors.some((anchor) => anchor.status === "current")
        )),
  );
  const threads = review.threads.filter(
    (t) =>
      anchorsIn(t.anchors, candidateId, path).length > 0 &&
      (!fallbackOnly ||
        !(
          placedIds?.has(`thread:${t.id}`) ??
          t.anchors.some((anchor) => anchor.status === "current")
        )),
  );
  const hasUserComments = review.threads.some((t) =>
    t.contributions.some((c) => c.actor.kind === "user"),
  );
  const pending = usePendingDeliveries(review.id, hasUserComments);
  if (annotations.length === 0 && threads.length === 0) return null;
  return (
    <section className={styles.objects} aria-label="Annotations and comments">
      {annotations.map((a) => (
        <AnnotationCard
          key={a.id}
          review={review}
          annotation={a}
          active={a.id === activeObjectId}
          onFocus={() => onFocusObject(a.id)}
        />
      ))}
      {threads.map((t) => (
        <ThreadCard
          key={t.id}
          review={review}
          thread={t}
          active={t.id === activeObjectId}
          pending={pending}
          onFocus={() => onFocusObject(t.id)}
        />
      ))}
    </section>
  );
}
