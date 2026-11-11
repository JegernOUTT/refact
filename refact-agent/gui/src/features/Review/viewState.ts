import type { DiffMode } from "../Workspace/GitPanel/diff/diffEditors";
import type { Review } from "../../services/refact/review";
import { type CompareMode, firstAnchorOf, orderedCandidates } from "./model";

export type ReviewTab = "tour" | "files" | "findings" | "discussion";

/**
 * Local, per-surface view preferences. Server-owned review state never lives
 * here; this only records where the user is looking.
 */
export type ReviewViewState = {
  /** null means "follow the review's current candidate". */
  pinnedCandidateId: string | null;
  compare: CompareMode;
  diffMode: DiffMode;
  path: string | null;
  showUnchanged: boolean;
  navTab: ReviewTab;
  followTour: boolean;
  tourStepId: string | null;
  activeObjectId: string | null;
  sourcesOpen: boolean;
  activityOpen: boolean;
};

export const initialViewState: ReviewViewState = {
  pinnedCandidateId: null,
  compare: "full",
  diffMode: "side-by-side",
  path: null,
  showUnchanged: false,
  navTab: "files",
  followTour: false,
  tourStepId: null,
  activeObjectId: null,
  sourcesOpen: false,
  activityOpen: false,
};

export type ReviewViewAction =
  | { type: "select_candidate"; candidateId: string | null }
  | { type: "set_compare"; compare: CompareMode }
  | { type: "set_diff_mode"; diffMode: DiffMode }
  | { type: "select_file"; path: string }
  | { type: "toggle_unchanged" }
  | { type: "set_nav_tab"; tab: ReviewTab }
  | { type: "start_follow"; stepId: string | null; path: string | null }
  | { type: "stop_follow" }
  | { type: "tour_step"; stepId: string; path: string | null }
  | { type: "focus_object"; objectId: string | null; path: string | null }
  | { type: "toggle_sources" }
  | { type: "toggle_activity" };

export function reviewViewReducer(
  state: ReviewViewState,
  action: ReviewViewAction,
): ReviewViewState {
  switch (action.type) {
    case "select_candidate":
      // Changing the inspected revision is a manual navigation.
      return {
        ...state,
        pinnedCandidateId: action.candidateId,
        followTour: false,
      };
    case "set_compare":
      return { ...state, compare: action.compare };
    case "set_diff_mode":
      return { ...state, diffMode: action.diffMode };
    case "select_file":
      return {
        ...state,
        path: action.path,
        followTour: false,
        activeObjectId: null,
      };
    case "toggle_unchanged":
      return { ...state, showUnchanged: !state.showUnchanged };
    case "set_nav_tab":
      return { ...state, navTab: action.tab };
    case "start_follow":
      return {
        ...state,
        followTour: true,
        navTab: "tour",
        pinnedCandidateId: null,
        tourStepId: action.stepId,
        activeObjectId: action.stepId,
        path: action.path ?? state.path,
      };
    case "stop_follow":
      return { ...state, followTour: false };
    case "tour_step":
      return {
        ...state,
        tourStepId: action.stepId,
        activeObjectId: action.stepId,
        path: action.path ?? state.path,
      };
    case "focus_object":
      return {
        ...state,
        activeObjectId: action.objectId,
        path: action.path ?? state.path,
        followTour: false,
      };
    case "toggle_sources":
      return { ...state, sourcesOpen: !state.sourcesOpen };
    case "toggle_activity":
      return { ...state, activityOpen: !state.activityOpen };
  }
}

/** The candidate being inspected: pinned history, else the live current one. */
export function viewedCandidateId(
  review: Review,
  state: ReviewViewState,
): string | null {
  if (
    state.pinnedCandidateId &&
    review.candidates.some((c) => c.id === state.pinnedCandidateId)
  ) {
    return state.pinnedCandidateId;
  }
  return (
    review.current_candidate_id ?? orderedCandidates(review).at(-1)?.id ?? null
  );
}

export function tourStepPath(review: Review, stepId: string): string | null {
  const step = review.tour.find((s) => s.id === stepId);
  return step
    ? firstAnchorOf(step.current.anchors)?.position.path ?? null
    : null;
}
