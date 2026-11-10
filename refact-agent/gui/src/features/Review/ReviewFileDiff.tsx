import { useEffect, useMemo } from "react";

import { LoadingState } from "../../components/ui";
import {
  reviewErrorMessage,
  useGetReviewFileQuery,
  type Anchor,
  type Review,
  type ReviewFileSide,
} from "../../services/refact/review";
import type { DiffMode } from "../Workspace/GitPanel/diff/diffEditors";
import {
  ReviewDiffEditors,
  type RevealRequest,
  type DiffPosition,
} from "./diff/ReviewDiffEditors";
import type {
  HighlightKind,
  LineHighlight,
  LineRange,
} from "./diff/reviewHighlights";
import {
  candidateHasPath,
  findCandidate,
  paneFor,
  shortOid,
  type DiffPanes,
  type PaneId,
  type PaneSource,
} from "./model";
import styles from "./Review.module.css";
import type { InlineHost } from "./diff/inlineDiscussions";

export type ReviewFileDiffProps = {
  navigationKey?: string;
  positions?: Map<string, DiffPosition>;
  autoRevealFirstChange?: boolean;
  review: Review;
  panes: DiffPanes;
  diffMode: DiffMode;
  path: string;
  activeObjectId: string | null;
  reveal: RevealRequest | null;
  inlineHosts?: { left: InlineHost[]; right: InlineHost[] };
  preferSingleSided?: boolean;
  onCommentLine?: (pane: PaneId, line: number) => void;
  onLineCounts?: (counts: { left: number; right: number }) => void;
  onSelection: (pane: PaneId, range: LineRange | null) => void;
};

function sideText(side: ReviewFileSide): string | null {
  if (side.kind === "absent") return "";
  if (side.kind === "text") return side.text;
  return null;
}

function collect(
  review: Review,
  panes: DiffPanes,
  path: string,
  activeObjectId: string | null,
): { left: LineHighlight[]; right: LineHighlight[] } {
  const out = { left: [] as LineHighlight[], right: [] as LineHighlight[] };
  const push = (id: string, kind: HighlightKind, anchors: Anchor[]) => {
    for (const anchor of anchors) {
      if (anchor.status !== "current" || anchor.position.path !== path)
        continue;
      const pane = paneFor(panes, anchor.position);
      if (!pane) continue;
      out[pane].push({
        id,
        kind,
        startLine: anchor.position.start_line,
        endLine: anchor.position.end_line,
        active: id === activeObjectId,
      });
    }
  };
  review.annotations.forEach((a) =>
    push(a.id, a.kind === "finding" ? "finding" : "explanation", a.anchors),
  );
  review.threads.forEach((t) => push(t.id, "thread", t.anchors));
  review.tour.forEach((s) => push(s.id, "tour", s.current.anchors));
  return out;
}

function usePaneText(review: Review, source: PaneSource, path: string) {
  const candidate = findCandidate(review, source.candidateId);
  const present = candidate !== null && candidateHasPath(candidate, path);
  const query = useGetReviewFileQuery(
    { reviewId: review.id, candidateId: source.candidateId, path },
    { skip: !present },
  );
  if (!present) {
    return {
      loading: false,
      error: null,
      text: "" as string | null,
      absent: true,
    };
  }
  if (query.currentData) {
    return {
      loading: false,
      error: null,
      text: sideText(query.currentData[source.side]),
      absent: query.currentData[source.side].kind === "absent",
    };
  }
  if (query.error && !query.isFetching) {
    return {
      loading: false,
      error: reviewErrorMessage(query.error),
      text: null,
      absent: false,
    };
  }
  return { loading: true, error: null, text: null, absent: false };
}

function paneLabel(review: Review, source: PaneSource): string {
  const candidate = findCandidate(review, source.candidateId);
  if (!candidate) return source.side;
  const oid = source.side === "base" ? candidate.base_oid : candidate.head_oid;
  const name =
    source.side === "base"
      ? "Base"
      : source.candidateId === review.current_candidate_id
        ? "Head"
        : "Candidate head";
  return `${name} ${shortOid(oid)}`;
}

export function ReviewFileDiff({
  navigationKey,
  positions,
  autoRevealFirstChange = true,
  review,
  panes,
  diffMode,
  path,
  activeObjectId,
  reveal,
  inlineHosts,
  preferSingleSided = false,
  onCommentLine,
  onLineCounts,
  onSelection,
}: ReviewFileDiffProps) {
  const left = usePaneText(review, panes.left, path);
  const right = usePaneText(review, panes.right, path);
  useEffect(() => {
    if (
      left.text !== null &&
      right.text !== null &&
      !left.loading &&
      !right.loading
    ) {
      onLineCounts?.({
        left: left.text.split("\n").length,
        right: right.text.split("\n").length,
      });
    }
  }, [left.text, right.text, left.loading, right.loading, onLineCounts]);
  const highlights = useMemo(
    () => collect(review, panes, path, activeObjectId),
    [review, panes, path, activeObjectId],
  );

  if (left.loading || right.loading) {
    return <LoadingState label="Loading file" />;
  }
  const error = left.error ?? right.error;
  if (error) {
    return (
      <p role="alert" className={styles.unavailable}>
        {error}
      </p>
    );
  }
  if (left.text === null || right.text === null) {
    return (
      <p className={styles.unavailable}>
        Binary or non-UTF-8 content; a text diff is not available for this file.
      </p>
    );
  }

  const baseOnly = preferSingleSided && !left.absent && right.absent;
  const headOnly = preferSingleSided && left.absent && !right.absent;
  return (
    <ReviewDiffEditors
      navigationKey={navigationKey}
      positions={positions}
      autoRevealFirstChange={autoRevealFirstChange}
      mode={baseOnly || headOnly ? "unified" : diffMode}
      left={baseOnly ? "" : left.text}
      right={baseOnly ? left.text : right.text}
      path={path}
      leftLabel={paneLabel(review, panes.left)}
      rightLabel={paneLabel(review, baseOnly ? panes.left : panes.right)}
      highlights={baseOnly ? { left: [], right: highlights.left } : highlights}
      reveal={baseOnly && reveal ? { ...reveal, pane: "right" } : reveal}
      inlineHosts={
        baseOnly ? { left: [], right: inlineHosts?.left ?? [] } : inlineHosts
      }
      onCommentLine={
        baseOnly
          ? (_pane, line) => onCommentLine?.("left", line)
          : onCommentLine
      }
      onSelection={
        baseOnly ? (_pane, range) => onSelection("left", range) : onSelection
      }
      contentSide={baseOnly ? "base" : undefined}
    />
  );
}
