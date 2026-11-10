import type { ReactNode } from "react";
import { useGetReviewQuery } from "../../services/refact/review";
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
  if (
    typeof value.id !== "string" ||
    !value.id ||
    typeof value.thread_id !== "string" ||
    !value.thread_id ||
    typeof value.contribution_id !== "string" ||
    !value.contribution_id ||
    !Array.isArray(value.blocks) ||
    !value.blocks.length
  )
    return null;
  return {
    id: value.id,
    thread_id: value.thread_id,
    contribution_id: value.contribution_id,
  };
}

// eslint-disable-next-line react-refresh/only-export-components
export function isRichReviewContribution(extra: unknown): boolean {
  return reference(extra) !== null;
}

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
  if (!ref || !chatId) return <>{fallback}</>;
  if (
    !review ||
    error != null ||
    review.id !== ref.id ||
    review.owner_chat_id !== chatId ||
    !thread ||
    !contribution?.blocks?.length
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
        </RichFileNavigationContext.Provider>
      </RichNavigationContext.Provider>
    </ReviewDocumentState>
  );
}
