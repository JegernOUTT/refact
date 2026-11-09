import type { Anchor } from "../../services/refact/review";

export type ReviewNavigationRequest = {
  reviewId: string;
  anchor?: Anchor;
  candidateId?: string;
  path?: string;
  threadId?: string;
};

export function revealReview(request: ReviewNavigationRequest) {
  window.dispatchEvent(
    new CustomEvent<ReviewNavigationRequest>("refact:review-reveal", {
      detail: request,
    }),
  );
}
