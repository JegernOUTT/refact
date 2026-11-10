import { useCallback } from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";

import { Badge, IconButton } from "../../components/ui";
import { Markdown } from "../../components/Markdown";
import { ArtifactPolicyProvider } from "../../components/Markdown/ArtifactPolicyContext";
import type { Anchor, Review, TourStep } from "../../services/refact/review";
import { ReviewHtmlPage } from "./ReviewHtmlPage";
import { ObjectHistory } from "./ObjectHistory";
import { label } from "./labels";
import { shortOid } from "./model";
import styles from "./ReviewTour.module.css";

type TourStepProps = {
  review: Review;
  stepId: string | null;
  onSelectStep: (stepId: string) => void;
  onRevealAnchor: (anchor: Anchor) => void;
};

export type TourStepContentProps = TourStepProps;

function TourBody({
  step,
  bindingKey,
  onRevealAnchor,
}: {
  step: TourStep;
  bindingKey: string;
  onRevealAnchor: (anchor: Anchor) => void;
}) {
  const reveal = useCallback(
    (index: number) => {
      if (!Number.isSafeInteger(index) || index < 0) return;
      const anchor = step.current.anchors.at(index);
      if (anchor && anchor.status !== "stalled") onRevealAnchor(anchor);
    },
    [step, onRevealAnchor],
  );
  return (
    <ArtifactPolicyProvider
      value={{
        mode: "review-seamless",
        title: step.current.title,
        bindingKey,
        onRevealAnchor: reveal,
      }}
    >
      <Markdown>{step.current.body}</Markdown>
    </ArtifactPolicyProvider>
  );
}

function anchorLines(anchor: Anchor): string {
  const { start_line, end_line } = anchor.position;
  return start_line === end_line
    ? `L${start_line}`
    : `L${start_line}–${end_line}`;
}

function anchorLabel(anchor: Anchor): string {
  return `${anchor.position.path} · ${label(
    anchor.position.side,
  )} ${anchorLines(anchor)}`;
}

function AnchorChips({
  review,
  anchors,
  onRevealAnchor,
}: {
  review: Review;
  anchors: Anchor[];
  onRevealAnchor: (anchor: Anchor) => void;
}) {
  if (anchors.length === 0) {
    return <p className={styles.meta}>This step has no code ranges.</p>;
  }
  return (
    <ul className={styles.anchors} aria-label="Code ranges">
      {anchors.map((anchor, index) => {
        const stalled = anchor.status === "stalled";
        const historical =
          anchor.position.candidate_id !== review.current_candidate_id;
        const text = anchorLabel(anchor);
        return (
          <li
            key={`${anchor.position.candidate_id}:${text}:${index}`}
            className={styles.anchorItem}
          >
            <button
              type="button"
              className={styles.anchor}
              data-status={anchor.status}
              disabled={stalled}
              title={stalled ? anchor.stalled_reason ?? undefined : undefined}
              aria-label={`Show ${text}${
                historical
                  ? ` on candidate ${shortOid(anchor.position.candidate_id)}`
                  : ""
              }`}
              onClick={() => onRevealAnchor(anchor)}
            >
              <span className={styles.anchorPath}>{anchor.position.path}</span>
              <span className={styles.anchorRange}>
                {label(anchor.position.side)} {anchorLines(anchor)}
              </span>
            </button>
            {historical && (
              <Badge tone="muted" size="xs">
                Historical · {shortOid(anchor.position.candidate_id)}
              </Badge>
            )}
            {stalled && (
              <Badge tone="warning" size="xs">
                Stalled
                {anchor.stalled_reason ? ` · ${anchor.stalled_reason}` : ""}
              </Badge>
            )}
          </li>
        );
      })}
    </ul>
  );
}

function StepNav({
  review,
  index,
  onSelectStep,
}: {
  review: Review;
  index: number;
  onSelectStep: (stepId: string) => void;
}) {
  const previous = index > 0 ? review.tour.at(index - 1) : undefined;
  const next = review.tour.at(index + 1);
  return (
    <div className={styles.nav}>
      <IconButton
        icon={ChevronLeft}
        aria-label="Previous step"
        size="sm"
        variant="ghost"
        disabled={!previous}
        onClick={() => previous && onSelectStep(previous.id)}
      />
      <span className={styles.meta}>
        Step {index + 1} of {review.tour.length}
      </span>
      <IconButton
        icon={ChevronRight}
        aria-label="Next step"
        size="sm"
        variant="ghost"
        disabled={!next}
        onClick={() => next && onSelectStep(next.id)}
      />
    </div>
  );
}

function StepHeading({ step }: { step: TourStep }) {
  const stalled = step.current.anchors.filter(
    (a) => a.status === "stalled",
  ).length;
  return (
    <div className={styles.heading}>
      <h3 className={styles.title}>{step.current.title}</h3>
      {step.current.version > 1 && (
        <Badge tone="muted" size="xs">
          v{step.current.version}
        </Badge>
      )}
      {stalled > 0 && (
        <Badge tone="warning" size="xs">
          {stalled} stalled range{stalled === 1 ? "" : "s"}
        </Badge>
      )}
    </div>
  );
}

function findStep(review: Review, stepId: string | null) {
  if (!stepId) return null;
  const index = review.tour.findIndex((s) => s.id === stepId);
  const step = review.tour.at(index);
  return index >= 0 && step ? { step, index } : null;
}

export function TourStepContent({
  review,
  stepId,
  onSelectStep,
  onRevealAnchor,
}: TourStepContentProps) {
  const found = findStep(review, stepId);
  if (!found) return null;
  const { step, index } = found;
  return (
    <section
      key={step.id}
      className={styles.step}
      aria-label="Tour step"
      data-step-id={step.id}
    >
      <StepNav review={review} index={index} onSelectStep={onSelectStep} />
      {!step.current.html && <StepHeading step={step} />}
      {!step.current.html && (
        <AnchorChips
          review={review}
          anchors={step.current.anchors}
          onRevealAnchor={onRevealAnchor}
        />
      )}
      {step.current.html ? (
        <ReviewHtmlPage
          review={review}
          version={step.current}
          stepId={step.id}
          target={
            step.current.candidate_id ?? review.current_candidate_id
              ? {
                  candidate_id:
                    step.current.candidate_id ??
                    review.current_candidate_id ??
                    "",
                  step_id: step.id,
                  version: step.current.version,
                }
              : undefined
          }
        />
      ) : step.current.body ? (
        <div className={styles.body}>
          <TourBody
            key={`${review.id}:${review.current_candidate_id}:${step.id}:${step.current.version}`}
            bindingKey={`${review.id}:${review.current_candidate_id}:${step.id}:${step.current.version}`}
            step={step}
            onRevealAnchor={onRevealAnchor}
          />
        </div>
      ) : (
        <p className={styles.meta}>No written narrative for this step.</p>
      )}
      <ObjectHistory
        reviewId={review.id}
        current={{ kind: "tour_step", value: step }}
      />
    </section>
  );
}
