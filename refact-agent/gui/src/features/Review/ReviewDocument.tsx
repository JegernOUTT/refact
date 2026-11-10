import { useLayoutEffect, useRef, useState } from "react";
import { Button, Badge, Select } from "../../components/ui";
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
  return (
    <article ref={root} className={styles.topic} data-step-id={step.id}>
      <header className={styles.topicHeader}>
        {!shown.html && <h2>{shown.title}</h2>}
        <Badge size="xs" tone="muted">
          v{shown.version}
        </Badge>
        {shown.version !== step.current.version && (
          <Button size="sm" onClick={() => apply(step.current)}>
            Apply updated topic · v{step.current.version}
          </Button>
        )}
        <Select
          value={String(shown.version)}
          onValueChange={(value) => {
            const version = [step.current, ...step.history].find(
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
            {[step.current, ...step.history].map((version) => (
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
        <ObjectHistory
          compact
          reviewId={review.id}
          current={{ kind: "tour_step", value: step }}
        />
      </header>
      {candidateId !== review.current_candidate_id && (
        <p className={styles.historical}>
          Historical candidate · {candidateId?.slice(0, 8)}
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
  onManualScroll,
  follow,
  active,
  onRevealAnchor,
  revealTarget,
}: {
  review: Review;
  revealTarget?: { target: DocumentTarget; token: number; threadId?: string };
  stepId: string | null;
  onManualScroll: () => void;
  follow: boolean;
  active: boolean;
  onRevealAnchor: (anchor: Anchor) => void;
}) {
  const root = useRef<HTMLDivElement>(null);
  const lastSelection = useRef<string | null>(null);
  const lastReveal = useRef<number | null>(null);
  const [known, setKnown] = useState(
    () => new Set(review.tour.map((step) => step.id)),
  );
  const newCount = review.tour.filter((step) => !known.has(step.id)).length;
  useLayoutEffect(() => {
    if (!active || !stepId) return;
    if (
      revealTarget?.target.step_id === stepId &&
      lastReveal.current !== revealTarget.token
    ) {
      lastReveal.current = revealTarget.token;
      lastSelection.current = stepId;
      return;
    }
    if (stepId === lastSelection.current) return;
    const section = Array.from(
      root.current?.querySelectorAll<HTMLElement>("[data-step-id]") ?? [],
    ).find((item) => item.dataset.stepId === stepId);
    if (section && root.current)
      root.current.scrollTop +=
        section.getBoundingClientRect().top -
        root.current.getBoundingClientRect().top;
    lastSelection.current = stepId;
  }, [stepId, active, follow, revealTarget]);
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
      aria-label="Review document"
      tabIndex={0}
    >
      {newCount > 0 && (
        <Button
          size="sm"
          variant="ghost"
          onClick={() => setKnown(new Set(review.tour.map((step) => step.id)))}
        >
          {newCount} new topic{newCount === 1 ? "" : "s"}
        </Button>
      )}
      {review.tour.length === 0 ? (
        <div className={styles.empty}>
          <h2>Your review tour</h2>
          <p>
            No topics have been published yet. The owning chat can investigate
            and publish explanations while independent review workers run.
            Switch to Review to inspect captured files now.
          </p>
        </div>
      ) : (
        review.tour.map((step) => (
          <Topic
            key={step.id}
            review={review}
            step={step}
            revealTarget={revealTarget}
            active={active}
            onRevealAnchor={onRevealAnchor}
          />
        ))
      )}
      <div className={styles.sourceInventory}>
        <h3>Source inventory</h3>
        <ReviewRichBlocks
          review={review}
          bindingKey={`${review.id}:inventory`}
          body=""
          blocks={[
            {
              id: "consulted-inventory",
              revision: 1,
              content: {
                kind: "sources",
                evidence_ids: review.evidence
                  .filter((item) => item.source.kind !== "executed")
                  .map((item) => item.id),
                relation: "consulted",
              },
            },
          ]}
        />
        {review.evidence.some((item) => item.source.kind === "executed") && (
          <ReviewRichBlocks
            review={review}
            bindingKey={`${review.id}:executed`}
            body=""
            blocks={[
              {
                id: "executed-inventory",
                revision: 1,
                content: {
                  kind: "evidence",
                  evidence_ids: review.evidence
                    .filter((item) => item.source.kind === "executed")
                    .map((item) => item.id),
                },
              },
            ]}
          />
        )}
      </div>
    </div>
  );
}
