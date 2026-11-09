import { useRef, useState } from "react";
import type { ReactNode } from "react";
import {
  isReviewConflict,
  useAcceptTourRevisionProposalMutation,
  useGetReviewQuery,
} from "../../services/refact/review";
import type { DocumentTarget } from "../../services/refact/review";
import { Button } from "../../components/ui";
import { newRequestKey } from "./model";
import {
  ReviewDocumentState,
  ReviewRichBlocks,
  RichNavigationContext,
  RichFileNavigationContext,
} from "./ReviewRichBlocks";
import { revealReview } from "./reviewNavigation";

type ReviewReference = {
  id: string;
  thread_id: string;
  contribution_id: string;
  tour_revision_proposal_id?: string;
};
function reference(extra: unknown): ReviewReference | null {
  if (!extra || typeof extra !== "object" || !("review" in extra)) return null;
  const value = extra.review;
  if (
    !value ||
    typeof value !== "object" ||
    !("id" in value) ||
    !("thread_id" in value) ||
    !("contribution_id" in value) ||
    !("blocks" in value)
  )
    return null;
  const proposalId =
    "tour_revision_proposal_id" in value &&
    typeof value.tour_revision_proposal_id === "string"
      ? value.tour_revision_proposal_id
      : undefined;
  if (
    typeof value.id !== "string" ||
    !value.id ||
    typeof value.thread_id !== "string" ||
    !value.thread_id ||
    typeof value.contribution_id !== "string" ||
    !value.contribution_id ||
    !Array.isArray(value.blocks) ||
    (!value.blocks.length && !proposalId)
  )
    return null;
  return {
    id: value.id,
    thread_id: value.thread_id,
    contribution_id: value.contribution_id,
    tour_revision_proposal_id: proposalId,
  };
}

// eslint-disable-next-line react-refresh/only-export-components
export function isRichReviewContribution(extra: unknown): boolean {
  return reference(extra) !== null;
}

type AcceptRequest = {
  reviewId: string;
  proposalId: string;
  owner_chat_id: string;
  expected_revision: number;
  request_key: string;
  target: DocumentTarget;
};
export function ReviewChatContribution({
  extra,
  chatId,
  fallback,
  onOpenDiscussion,
}: {
  extra: unknown;
  chatId?: string;
  fallback: ReactNode;
  onOpenDiscussion?: (threadId: string) => void;
}) {
  const ref = reference(extra);
  const [accept] = useAcceptTourRevisionProposalMutation();
  const [pending, setPending] = useState(false);
  const [message, setMessage] = useState("");
  const busy = useRef(false);
  const request = useRef<AcceptRequest | null>(null);
  const {
    currentData: review,
    isFetching,
    error,
  } = useGetReviewQuery(
    { reviewId: ref?.id ?? "" },
    { skip: !ref || !chatId, refetchOnMountOrArgChange: true },
  );
  const thread = review?.threads.find((item) => item.id === ref?.thread_id);
  const contribution = thread?.contributions.find(
    (item) => item.id === ref?.contribution_id && item.thread_id === thread.id,
  );
  const proposal = review?.tour_revision_proposals?.find(
    (item) =>
      item.id === ref?.tour_revision_proposal_id &&
      item.thread_id === thread?.id &&
      (item.proposal_contribution_id === contribution?.id ||
        item.acceptance?.contribution_id === contribution?.id),
  );
  if (!ref || !chatId) return <>{fallback}</>;
  if (
    !review ||
    error != null ||
    review.id !== ref.id ||
    review.owner_chat_id !== chatId ||
    !thread ||
    !contribution ||
    (ref.tour_revision_proposal_id ? !proposal : !contribution.blocks?.length)
  )
    return (
      <>
        {fallback}
        {isFetching && <span role="status">Loading Review contribution…</span>}
      </>
    );
  const candidateId = contribution.context?.candidate_id;
  const historical =
    candidateId != null && candidateId !== review.current_candidate_id;
  const applicability = contribution.context?.applicability;
  const noncurrent = applicability != null && applicability !== "current";
  const openDiscussion = () => {
    if (onOpenDiscussion) onOpenDiscussion(thread.id);
    else revealReview({ reviewId: review.id, threadId: thread.id });
  };
  const target = proposal?.target;
  const step = review.tour.find((item) => item.id === target?.step_id);
  const block = step?.current.blocks?.find(
    (item) => item.id === target?.block_id,
  );
  const stale =
    !!target &&
    (review.current_candidate_id !== target.candidate_id ||
      !step ||
      step.current.candidate_id !== target.candidate_id ||
      step.current.version !== target.version ||
      (target.block_id != null &&
        (!block || block.revision !== target.block_revision)) ||
      (target.block_id == null && target.block_revision != null));
  const acceptProposal = async () => {
    if (!proposal || stale || proposal.acceptance != null || busy.current)
      return;
    busy.current = true;
    setPending(true);
    setMessage("");
    const saved = request.current;
    const payload = (saved?.reviewId === review.id &&
    saved.proposalId === proposal.id &&
    saved.owner_chat_id === chatId
      ? saved
      : null) ?? {
      reviewId: review.id,
      proposalId: proposal.id,
      owner_chat_id: chatId,
      expected_revision: review.revision,
      request_key: newRequestKey(),
      target: proposal.target,
    };
    request.current = payload;
    try {
      await accept(payload).unwrap();
      request.current = null;
    } catch (err) {
      if (isReviewConflict(err)) {
        request.current = null;
        setMessage(
          "Review changed. The latest status is being loaded; check it before accepting again.",
        );
      } else
        setMessage("Acceptance could not be confirmed. You may retry safely.");
    } finally {
      busy.current = false;
      setPending(false);
    }
  };
  return (
    <ReviewDocumentState>
      <RichNavigationContext.Provider
        value={(anchor) => revealReview({ reviewId: review.id, anchor })}
      >
        <RichFileNavigationContext.Provider
          value={(candidateId, path) =>
            revealReview({ reviewId: review.id, candidateId, path })
          }
        >
          {(historical || noncurrent) && (
            <p role="status">
              {historical ? "Historical candidate" : "Review contribution"}
              {noncurrent ? ` · ${applicability.replace(/_/g, " ")}` : ""}
            </p>
          )}
          <button type="button" onClick={openDiscussion}>
            Open discussion
          </button>
          <ReviewRichBlocks
            review={review}
            blocks={contribution.blocks}
            body={contribution.body}
            bindingKey={`${review.id}:${contribution.id}`}
          />
          {proposal && (
            <section>
              <p>Requested change: {proposal.change_request}</p>
              <p role="status">
                {proposal.fulfilment
                  ? "Fulfilled"
                  : proposal.acceptance
                    ? `Accepted pending${stale ? " · Stale target" : ""}`
                    : stale
                      ? "Stale"
                      : "Proposed"}
              </p>
              {proposal.acceptance && !proposal.fulfilment && (
                <p>Acceptance does not start work. Ask Review to continue.</p>
              )}
              {!proposal.acceptance && !proposal.fulfilment && (
                <Button
                  type="button"
                  size="sm"
                  variant="primary"
                  disabled={stale || pending || isFetching}
                  onClick={() => void acceptProposal()}
                >
                  Accept Tour revision
                </Button>
              )}
              {message && !proposal.acceptance && !proposal.fulfilment && (
                <p role="status">{message}</p>
              )}
            </section>
          )}
        </RichFileNavigationContext.Provider>
      </RichNavigationContext.Provider>
    </ReviewDocumentState>
  );
}
