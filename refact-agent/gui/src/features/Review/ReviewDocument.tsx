import { useLayoutEffect, useRef, useState } from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { Button, Icon, Select } from "../../components/ui";
import type {
  Anchor,
  Review,
  TourStep,
  TourVersion,
  DocumentTarget,
} from "../../services/refact/review";
import { ReviewRichBlocks, BlockQuestion } from "./ReviewRichBlocks";
import { ArtifactPolicyProvider } from "../../components/Markdown/ArtifactPolicyContext";
import { ReviewHtmlPage } from "./ReviewHtmlPage";
import { ObjectHistory } from "./ObjectHistory";
import styles from "./ReviewDocument.module.css";

function Topic({
  review,
  step,
  onRevealAnchor,
  revealTarget,
  active,
}: {
  review: Review;
  revealTarget?: { target: DocumentTarget; token: number; threadId?: string };
  active: boolean;
  step: TourStep;
  onRevealAnchor: (anchor: Anchor) => void;
}) {
  const [shown, setShown] = useState<TourVersion>(step.current);
  const root = useRef<HTMLElement>(null);
  const consumedReveal = useRef<number | null>(null);
  const restore = useRef<{ id: string; offset: number } | null>(null);
  const apply = (version: TourVersion) => {
    const container = root.current?.closest("[data-review-document]");
    const top = container?.getBoundingClientRect().top ?? 0;
    const blocks =
      root.current?.querySelectorAll<HTMLElement>("[data-block-id]");
    const first =
      blocks &&
      Array.from(blocks).find(
        (block) => block.getBoundingClientRect().bottom >= top,
      );
    restore.current = first
      ? {
          id: first.dataset.blockId ?? "",
          offset: first.getBoundingClientRect().top - top,
        }
      : null;
    setShown(version);
  };
  useLayoutEffect(() => {
    if (!restore.current || !root.current) return;
    const container = root.current.closest<HTMLElement>(
      "[data-review-document]",
    );
    const block = Array.from(
      root.current.querySelectorAll<HTMLElement>("[data-block-id]"),
    ).find((item) => item.dataset.blockId === restore.current?.id);
    if (container && block)
      container.scrollTop +=
        block.getBoundingClientRect().top -
        container.getBoundingClientRect().top -
        restore.current.offset;
    restore.current = null;
  }, [shown]);
  useLayoutEffect(() => {
    if (
      !active ||
      !revealTarget ||
      consumedReveal.current === revealTarget.token ||
      revealTarget.target.step_id !== step.id
    )
      return;
    const version = [step.current, ...step.history].find(
      (item) => item.version === revealTarget.target.version,
    );
    if (version) setShown(version);
  }, [revealTarget, step, active]);
  useLayoutEffect(() => {
    if (
      !active ||
      !revealTarget ||
      consumedReveal.current === revealTarget.token ||
      revealTarget.target.step_id !== step.id ||
      shown.version !== revealTarget.target.version ||
      !root.current
    )
      return;
    const container = root.current.closest<HTMLElement>(
      "[data-review-document]",
    );
    const block = Array.from(
      root.current.querySelectorAll<HTMLElement>("[data-block-id]"),
    ).find((item) => item.dataset.blockId === revealTarget.target.block_id);
    const thread = revealTarget.threadId
      ? Array.from(
          (block ?? root.current).querySelectorAll<HTMLElement>(
            "[data-document-thread-id]",
          ),
        ).find(
          (item) => item.dataset.documentThreadId === revealTarget.threadId,
        )
      : undefined;
    if (revealTarget.threadId && !thread) return;
    const destination = thread ?? block ?? root.current;
    if (container)
      container.scrollTop +=
        destination.getBoundingClientRect().top -
        container.getBoundingClientRect().top;
    consumedReveal.current = revealTarget.token;
  }, [shown, revealTarget, step.id, active, review.threads]);
  const candidateId =
    shown.candidate_id ??
    shown.anchors.at(0)?.position.candidate_id ??
    review.current_candidate_id;
  const target = candidateId
    ? { candidate_id: candidateId, step_id: step.id, version: shown.version }
    : undefined;
  const versions = [step.current, ...step.history];
  return (
    <article
      ref={root}
      className={styles.topic}
      data-step-id={step.id}
      aria-label={shown.title}
    >
      <header className={styles.topicHeader} data-page={!!shown.html}>
        {!shown.html && <h2>{shown.title}</h2>}
        {shown.version !== step.current.version && (
          <Button size="sm" onClick={() => apply(step.current)}>
            Apply updated topic · v{step.current.version}
          </Button>
        )}
        {versions.length > 1 && (
          <Select
            value={String(shown.version)}
            onValueChange={(value) => {
              const version = versions.find(
                (item) => item.version === Number(value),
              );
              if (version) apply(version);
            }}
          >
            <Select.Trigger
              aria-label={`Version of ${shown.title}`}
              className={styles.versionSelector}
            />
            <Select.Content>
              {versions.map((version) => (
                <Select.Item
                  key={version.version}
                  value={String(version.version)}
                >
                  v{version.version}
                  {version.version === step.current.version
                    ? " · latest"
                    : " · historical"}
                </Select.Item>
              ))}
            </Select.Content>
          </Select>
        )}
        {versions.length > 1 && (
          <ObjectHistory
            compact
            reviewId={review.id}
            current={{ kind: "tour_step", value: step }}
          />
        )}
      </header>
      {candidateId !== review.current_candidate_id && (
        <p className={styles.historical}>
          Written for an earlier version of the code.
        </p>
      )}
      {shown.html ? (
        <ReviewHtmlPage
          key={`${step.id}:${shown.version}`}
          review={review}
          version={shown}
          revealTarget={active ? revealTarget : undefined}
          stepId={step.id}
          target={target}
        />
      ) : (
        <ArtifactPolicyProvider
          value={{
            mode: "review-seamless",
            bindingKey: `${review.id}:${step.id}`,
            revealAnchorCount: shown.anchors.length,
            onRevealAnchor: (index) => {
              const anchor = shown.anchors.at(index);
              if (anchor && anchor.status === "current") onRevealAnchor(anchor);
            },
          }}
        >
          <ReviewRichBlocks
            review={review}
            blocks={shown.blocks}
            body={shown.body}
            target={target}
            bindingKey={`${review.id}:${step.id}`}
          />
        </ArtifactPolicyProvider>
      )}
      {!shown.html && !shown.blocks?.length && target && (
        <BlockQuestion review={review} target={target} />
      )}
    </article>
  );
}

export function ReviewDocument({
  review,
  stepId,
  onSelectStep,
  onManualScroll,
  active,
  working = false,
  onRevealAnchor,
  revealTarget,
}: {
  review: Review;
  revealTarget?: { target: DocumentTarget; token: number; threadId?: string };
  stepId: string | null;
  onSelectStep?: (stepId: string) => void;
  onManualScroll: () => void;
  active: boolean;
  /** The owning chat or its workers are still writing. */
  working?: boolean;
  onRevealAnchor: (anchor: Anchor) => void;
}) {
  const root = useRef<HTMLDivElement>(null);
  const found = review.tour.findIndex((step) => step.id === stepId);
  const index = found >= 0 ? found : 0;
  const step = review.tour.at(index);
  const previous = index > 0 ? review.tour.at(index - 1) : undefined;
  const next = review.tour.at(index + 1);
  const shownId = step?.id;
  // A page opens at its top; a pending reveal scrolls to its own target.
  const revealing = useRef(false);
  revealing.current = revealTarget?.target.step_id === shownId;
  useLayoutEffect(() => {
    if (root.current && !revealing.current) root.current.scrollTop = 0;
  }, [shownId]);
  return (
    <div
      ref={root}
      className={styles.document}
      data-review-document="true"
      onPointerDown={(event) => {
        if (event.target === event.currentTarget) onManualScroll();
      }}
      onWheel={onManualScroll}
      onTouchMove={onManualScroll}
      onKeyDown={(event) => {
        if (
          [
            "PageDown",
            "PageUp",
            "Home",
            "End",
            "ArrowDown",
            "ArrowUp",
          ].includes(event.key) &&
          event.target === event.currentTarget
        )
          onManualScroll();
      }}
      aria-label="Review tour"
      tabIndex={0}
    >
      {step ? (
        <Topic
          key={step.id}
          review={review}
          step={step}
          revealTarget={revealTarget}
          active={active}
          onRevealAnchor={onRevealAnchor}
        />
      ) : (
        <div className={styles.empty}>
          <h2>{working ? "The tour is being written" : "No tour yet"}</h2>
          <p>
            {working
              ? "Pages appear here one by one while the review runs. The changed files are already under Changes."
              : "Ask in the chat for a walkthrough of this change. The changed files are under Changes."}
          </p>
        </div>
      )}
      {step && review.tour.length > 1 && (
        <nav className={styles.pager} aria-label="Tour pages">
          <Button
            size="sm"
            variant="ghost"
            disabled={!previous}
            onClick={() => previous && onSelectStep?.(previous.id)}
          >
            <Icon icon={ChevronLeft} size="sm" />
            <span className={styles.pagerTitle}>
              {previous ? previous.current.title : "Previous"}
            </span>
          </Button>
          <span className={styles.pagerPosition}>
            Page {index + 1} of {review.tour.length}
          </span>
          <Button
            size="sm"
            variant={next ? "primary" : "ghost"}
            disabled={!next}
            onClick={() => next && onSelectStep?.(next.id)}
          >
            <span className={styles.pagerTitle}>
              {next ? next.current.title : "Next"}
            </span>
            <Icon icon={ChevronRight} size="sm" />
          </Button>
        </nav>
      )}
    </div>
  );
}
