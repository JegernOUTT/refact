import {
  createContext,
  useContext,
  useEffect,
  useMemo,
  useReducer,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { Columns2, MessageSquarePlus } from "lucide-react";

import {
  Badge,
  Button,
  IconButton,
  Tooltip,
  SegmentedControl,
  Select,
} from "../../components/ui";
import {
  buildMutation,
  isReviewConflict,
  isAnchor,
  reviewErrorMessage,
  useMutateReviewMutation,
  useReportReviewSelectionMutation,
  useResolveAnchorMutation,
  useLazyGetReviewHandoffQuery,
  type Anchor,
  type Evidence,
  type DocumentTarget,
  type Review,
} from "../../services/refact/review";
import { useInlineDiscussions } from "./diff/useInlineDiscussions";
import { Drawer, SourcesAndEvidence, WorkerActivity } from "./ReviewDrawers";
import { ReviewFileDiff } from "./ReviewFileDiff";
import { ReviewDocument } from "./ReviewDocument";
import { ReviewHeader } from "./ReviewHeader";
import {
  ReviewDocumentState,
  RichDiscussionContext,
  RichNavigationContext,
  RichFileNavigationContext,
} from "./ReviewRichBlocks";
import { ThreadCard, ReviewObjects } from "./ReviewObjects";
import { ReviewToolbar } from "./ReviewToolbar";
import { ReviewHandoff } from "./ReviewHandoff";
import type { RevealRequest } from "./diff/ReviewDiffEditors";
import type { LineRange } from "./diff/reviewHighlights";
import { FILE_STATUS_LETTER, formatTime, isRunActive, label } from "./labels";
import {
  diffPanes,
  findCandidate,
  firstAnchorOf,
  objectAnchor,
  paneFor,
  newRequestKey,
  orderedCandidates,
  previousCandidate,
  reattachOperation,
  reviewFiles,
  shortOid,
  stalledTargets,
  type PaneId,
  type StalledTarget,
} from "./model";
import {
  initialViewState,
  reviewViewReducer,
  tourStepPath,
  viewedCandidateId,
} from "./viewState";
import { useReviewReadOnly } from "./readOnly";
import styles from "./Review.module.css";

type Selection = { pane: PaneId; range: LineRange } | null;

export const StackHeaderContext = createContext<{
  controls: ReactNode;
  status: ReactNode;
  pending?: { token: number; detail: unknown };
} | null>(null);

export const ReviewSurfaceActiveContext = createContext(true);
export const ReviewAcceptedNavigationContext = createContext<
  (reviewId: string) => void
>(() => undefined);

const SELECTION_REPORT_DELAY_MS = 400;

function targetLink(reference: string): { href: string; label: string } | null {
  try {
    const url = new URL(reference);
    if (url.protocol !== "https:" && url.protocol !== "http:") return null;
    const last = url.pathname.split("/").filter(Boolean).at(-1);
    return {
      href: url.href,
      label: `${url.hostname}${last && /^\d+$/.test(last) ? ` #${last}` : ""}`,
    };
  } catch {
    return null;
  }
}

export function ReviewCanvas({
  review,
  active: memberActive = true,
}: {
  review: Review;
  active?: boolean;
}) {
  const acceptNavigation = useContext(ReviewAcceptedNavigationContext);
  const navigationToggleRef = useRef<HTMLDivElement>(null);
  const closeNavigation = () => {
    const toggle = navigationToggleRef.current;
    if (
      toggle &&
      toggle.parentElement?.contains(document.activeElement) &&
      getComputedStyle(toggle).display !== "none"
    )
      toggle.querySelector("button")?.focus();
    setNavigationOpen(false);
  };
  const surfaceActive = useContext(ReviewSurfaceActiveContext);
  const active = memberActive && surfaceActive;
  const [navigationOpen, setNavigationOpen] = useState(false);
  const navigationId = `review-${review.id}-navigation`;
  const [view, dispatch] = useReducer(reviewViewReducer, {
    ...initialViewState,
    navTab: "tour",
  });
  const readOnly = useReviewReadOnly();
  const stackHeader = useContext(StackHeaderContext);
  const [documentTarget, setDocumentTarget] = useState<{
    target: DocumentTarget;
    token: number;
    threadId?: string;
  }>();
  const consumedNavigation = useRef<unknown>(null);
  const pendingNavigation = useRef<unknown>(null);
  const [selection, setSelection] = useState<Selection>(null);
  const [layoutChosen, setLayoutChosen] = useState(false);
  const canvasRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || layoutChosen || typeof ResizeObserver === "undefined")
      return;
    const observer = new ResizeObserver(([entry]) => {
      const width = entry.contentRect.width;
      if (width <= 0) return;
      dispatch({
        type: "set_diff_mode",
        diffMode: width <= 640 ? "unified" : "side-by-side",
      });
    });
    observer.observe(canvas);
    return () => observer.disconnect();
  }, [layoutChosen]);
  const [draftAnchors, setDraftAnchors] = useState<Anchor[]>([]);
  const draftFocus = useRef(new WeakMap<Anchor, boolean>());
  const setDraftAnchor = (anchor: Anchor) =>
    setDraftAnchors((current) =>
      current.some(
        (a) => JSON.stringify(a.position) === JSON.stringify(anchor.position),
      )
        ? current
        : [...current, anchor],
    );
  const [reattach, setReattach] = useState<StalledTarget | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [reveal, setReveal] = useState<RevealRequest | null>(null);
  const revealToken = useRef(0);

  const [resolveAnchor, resolveState] = useResolveAnchorMutation();
  const [mutateReview, mutateState] = useMutateReviewMutation();
  const [reportSelection] = useReportReviewSelectionMutation();
  const [loadHandoff] = useLazyGetReviewHandoffQuery();

  const candidateId = viewedCandidateId(review, view);
  const candidate = findCandidate(review, candidateId);
  const isHistorical =
    candidate !== null && candidate.id !== review.current_candidate_id;
  const previous = candidate ? previousCandidate(review, candidate.id) : null;
  const panes = useMemo(
    () => (candidate ? diffPanes(candidate, previous, view.compare) : null),
    [candidate, previous, view.compare],
  );
  const selectionSource = selection && panes ? panes[selection.pane] : null;
  const candidates = orderedCandidates(review);
  const reference = review.target_reference;
  const link = reference ? targetLink(reference) : null;

  const files = useMemo(
    () =>
      candidate
        ? reviewFiles(review, candidate, view.compare, view.showUnchanged)
        : [],
    [review, candidate, view.compare, view.showUnchanged],
  );
  const path =
    view.path &&
    (files.some((f) => f.path === view.path) ||
      candidate?.files.some((f) => f.path === view.path))
      ? view.path
      : files.at(0)?.path ?? null;

  const stalled = useMemo(() => stalledTargets(review), [review]);
  const activeRuns = review.runs.filter((r) => isRunActive(r.status)).length;

  // Following the tour tracks the newest step as the agent publishes it;
  // this only scrolls the diff and never moves keyboard focus.
  const lastStepId = review.tour.at(-1)?.id ?? null;
  useEffect(() => {
    if (!view.followTour || !lastStepId || lastStepId === view.tourStepId) {
      return;
    }
    dispatch({
      type: "tour_step",
      stepId: lastStepId,
      path: tourStepPath(review, lastStepId),
    });
  }, [view.followTour, lastStepId, view.tourStepId, review]);

  const activeAnchor = view.activeObjectId
    ? objectAnchor(review, view.activeObjectId)
    : null;
  const revealPane =
    activeAnchor?.status === "current" &&
    activeAnchor.position.path === path &&
    panes
      ? paneFor(panes, activeAnchor.position)
      : null;
  const revealLineNumber = activeAnchor?.position.start_line ?? 0;
  const currentDiffMode = useRef(view.diffMode);
  currentDiffMode.current = view.diffMode;
  useEffect(() => {
    if (!revealPane) return;
    if (revealPane === "left" && currentDiffMode.current === "unified") {
      dispatch({ type: "set_diff_mode", diffMode: "side-by-side" });
    }
    revealToken.current += 1;
    setReveal({
      pane: revealPane,
      line: revealLineNumber,
      token: revealToken.current,
    });
  }, [view.activeObjectId, revealPane, revealLineNumber]);

  // Report the viewed candidate/selection as ephemeral chat context.
  const viewedId = candidate?.id ?? null;
  useEffect(() => {
    if (!viewedId || !active || readOnly) return;
    const timer = window.setTimeout(() => {
      void reportSelection({
        reviewId: review.id,
        candidate_id: selectionSource?.candidateId ?? viewedId,
        action: selection ? "select_range" : "view_file",
        path,
        side: selectionSource?.side ?? null,
        start_line: selection?.range.startLine ?? null,
        end_line: selection?.range.endLine ?? null,
        object_id: view.activeObjectId,
        request_key: newRequestKey(),
      });
    }, SELECTION_REPORT_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, [
    viewedId,
    active,
    readOnly,
    path,
    selection,
    selectionSource,
    view.activeObjectId,
    review.id,
    reportSelection,
  ]);

  useEffect(() => {
    setSelection(null);
  }, [viewedId, path, view.compare]);

  const revealEvidence = (evidence: Evidence) => {
    if (evidence.source.kind !== "source") return;
    dispatch({ type: "set_nav_tab", tab: "files" });
    dispatch({ type: "stop_follow" });
    const anchor = evidence.source.anchor;
    if (anchor?.status === "current") {
      revealExactAnchor(anchor);
      return;
    }
    const candidateForEvidence =
      anchor?.position.candidate_id ?? evidence.candidate_id;
    if (candidateForEvidence !== candidate?.id) {
      dispatch({
        type: "select_candidate",
        candidateId:
          candidateForEvidence === review.current_candidate_id
            ? null
            : candidateForEvidence,
      });
    }
    dispatch({ type: "select_file", path: evidence.source.path });
  };

  const revealExactAnchor = (anchor: Anchor) => {
    if (anchor.status !== "current") return;
    dispatch({ type: "set_nav_tab", tab: "files" });
    dispatch({ type: "stop_follow" });
    const exactPane = panes ? paneFor(panes, anchor.position) : null;
    if (!exactPane) {
      dispatch({
        type: "select_candidate",
        candidateId:
          anchor.position.candidate_id === review.current_candidate_id
            ? null
            : anchor.position.candidate_id,
      });
      dispatch({ type: "set_compare", compare: "full" });
    }
    const pane =
      exactPane ?? (anchor.position.side === "base" ? "left" : "right");
    if (pane === "left" && view.diffMode === "unified")
      dispatch({ type: "set_diff_mode", diffMode: "side-by-side" });
    dispatch({ type: "select_file", path: anchor.position.path });
    revealToken.current += 1;
    setReveal({
      pane,
      line: anchor.position.start_line,
      token: revealToken.current,
    });
  };

  useEffect(() => {
    const receive = (event: Event) => {
      if (!(event instanceof CustomEvent)) return;
      const detail: unknown = event.detail;
      if (consumedNavigation.current === detail) return;
      if (
        typeof detail !== "object" ||
        detail === null ||
        !("reviewId" in detail) ||
        detail.reviewId !== review.id
      )
        return;
      pendingNavigation.current = detail;
      if ("threadId" in detail && typeof detail.threadId === "string") {
        const thread = review.threads.find(
          (item) => item.id === detail.threadId,
        );
        if (!thread) return;
        if (thread.target) {
          const target = thread.target;
          if (
            !review.tour.some(
              (step) =>
                step.id === target.step_id &&
                [step.current, ...step.history].some(
                  (version) => version.version === target.version,
                ),
            )
          )
            return;
          revealToken.current += 1;
          setDocumentTarget({
            target,
            threadId: thread.id,
            token: revealToken.current,
          });
          dispatch({ type: "stop_follow" });
          dispatch({ type: "set_nav_tab", tab: "tour" });
          dispatch({
            type: "tour_step",
            stepId: thread.target.step_id,
            path: null,
          });
        } else {
          const anchor = firstAnchorOf(thread.anchors);
          if (!anchor || anchor.status !== "current") return;
          dispatch({ type: "set_nav_tab", tab: "files" });
          revealExactAnchor(anchor);
        }
        dispatch({ type: "focus_object", objectId: thread.id, path: null });
      } else if ("anchor" in detail && isAnchor(detail.anchor)) {
        if (detail.anchor.status !== "current") return;
        const position = detail.anchor.position;
        if (
          !review.candidates.some(
            (item) =>
              item.id === position.candidate_id &&
              item.files.some((file) => file.path === position.path),
          )
        )
          return;
        dispatch({ type: "set_nav_tab", tab: "files" });
        revealExactAnchor(detail.anchor);
      } else if (
        "candidateId" in detail &&
        typeof detail.candidateId === "string" &&
        "path" in detail &&
        typeof detail.path === "string"
      ) {
        const path = detail.path;
        if (
          !review.candidates.some(
            (item) =>
              item.id === detail.candidateId &&
              item.files.some((file) => file.path === path),
          )
        )
          return;
        dispatch({ type: "set_nav_tab", tab: "files" });
        dispatch({ type: "select_candidate", candidateId: detail.candidateId });
        dispatch({ type: "set_compare", compare: "full" });
        dispatch({ type: "select_file", path });
      } else {
        return;
      }
      acceptNavigation(review.id);
      consumedNavigation.current = detail;
      pendingNavigation.current = null;
    };
    if (stackHeader?.pending)
      receive(
        new CustomEvent("refact:review-reveal", {
          detail: stackHeader.pending.detail,
        }),
      );
    if (pendingNavigation.current)
      receive(
        new CustomEvent("refact:review-reveal", {
          detail: pendingNavigation.current,
        }),
      );
    window.addEventListener("refact:review-reveal", receive);
    return () => window.removeEventListener("refact:review-reveal", receive);
  });

  const selectStep = (stepId: string) => {
    closeNavigation();
    dispatch({ type: "stop_follow" });
    dispatch({
      type: "tour_step",
      stepId,
      path: tourStepPath(review, stepId),
    });
  };

  const anchorSelection = async (
    selected = selection,
  ): Promise<Anchor | null> => {
    const source = selected && panes ? panes[selected.pane] : null;
    if (!selected || !source || !path) return null;
    setActionError(null);
    const result = await resolveAnchor({
      reviewId: review.id,
      candidate_id: source.candidateId,
      path,
      side: source.side,
      start_line: selected.range.startLine,
      end_line: selected.range.endLine,
    });
    if ("error" in result) {
      setActionError(reviewErrorMessage(result.error));
      return null;
    }
    return result.data;
  };

  const navigationIdentity = `${candidateId}:${path}:${view.compare}:${view.diffMode}`;
  const focusNavigation = useRef({ active, epoch: 0 });
  if (focusNavigation.current.active !== active) {
    focusNavigation.current = {
      active,
      epoch: focusNavigation.current.epoch + 1,
    };
  }
  const currentNavigation = useRef(navigationIdentity);
  const [lineCounts, setLineCounts] = useState<{
    identity: string;
    left: number;
    right: number;
  } | null>(null);
  currentNavigation.current = navigationIdentity;
  const openComment = async (selected = selection) => {
    if (readOnly) return;
    dispatch({ type: "stop_follow" });
    const requestedFrom = currentNavigation.current;
    const requestedEpoch = focusNavigation.current.epoch;
    const anchor = await anchorSelection(selected);
    if (anchor) {
      draftFocus.current.set(
        anchor,
        requestedFrom === currentNavigation.current &&
          focusNavigation.current.active &&
          requestedEpoch === focusNavigation.current.epoch,
      );
      setDraftAnchor(anchor);
    }
  };
  const startComment = () => openComment();

  const { inlineHosts, portals, placedIds } = useInlineDiscussions({
    review,
    active,
    panes,
    path,
    draftAnchors,
    lineCounts:
      lineCounts?.identity === navigationIdentity ? lineCounts : undefined,
    focusAllowed: (anchor) => draftFocus.current.get(anchor) ?? false,
    onCloseDraft: (anchor: Anchor) =>
      setDraftAnchors((current) => current.filter((a) => a !== anchor)),
    activeObjectId: view.activeObjectId,
    onFocusObject: (objectId) => {
      const anchor = objectAnchor(review, objectId);
      if (anchor) revealExactAnchor(anchor);
      dispatch({ type: "focus_object", objectId, path });
    },
  });

  const applyReattach = async () => {
    if (!reattach || readOnly) return;
    const anchor = await anchorSelection();
    if (!anchor) return;
    const operation = reattachOperation(review, reattach, anchor);
    if (!operation) {
      setActionError("That item no longer exists.");
      setReattach(null);
      return;
    }
    const result = await mutateReview({
      reviewId: review.id,
      mutation: buildMutation(review, newRequestKey(), operation),
    });
    if ("error" in result) {
      setActionError(
        isReviewConflict(result.error)
          ? "The review changed; check the item and reattach again."
          : reviewErrorMessage(result.error),
      );
      return;
    }
    setReattach(null);
  };

  const selectionLabel =
    selection && selectionSource
      ? `${label(selectionSource.side)} L${selection.range.startLine}${
          selection.range.endLine !== selection.range.startLine
            ? `–${selection.range.endLine}`
            : ""
        }`
      : null;

  return (
    <ReviewDocumentState>
      <RichFileNavigationContext.Provider
        value={(candidateId, path) => {
          dispatch({ type: "set_nav_tab", tab: "files" });
          dispatch({
            type: "select_candidate",
            candidateId:
              candidateId === review.current_candidate_id ? null : candidateId,
          });
          dispatch({ type: "set_compare", compare: "full" });
          dispatch({ type: "select_file", path });
        }}
      >
        <RichNavigationContext.Provider
          value={(anchor) => {
            dispatch({ type: "set_nav_tab", tab: "files" });
            revealExactAnchor(anchor);
          }}
        >
          <RichDiscussionContext.Provider
            value={(id) => {
              const thread = review.threads.find((item) => item.id === id);
              return thread ? (
                <ThreadCard
                  review={review}
                  thread={thread}
                  active={false}
                  pending={new Set()}
                  onFocus={() =>
                    window.dispatchEvent(
                      new CustomEvent("refact:review-reveal", {
                        detail: { reviewId: review.id, threadId: id },
                      }),
                    )
                  }
                />
              ) : null;
            }}
          >
            <div
              ref={canvasRef}
              className={styles.canvas}
              data-read-only={readOnly}
              aria-label="Review canvas"
              role="region"
            >
              <ReviewHeader
                stackControls={stackHeader?.controls}
                identity={
                  stackHeader ? null : (
                    <span title={review.target_reference}>
                      {review.title || "Review"}
                    </span>
                  )
                }
                status={
                  <>
                    {stackHeader?.status}
                    {isHistorical && (
                      <span>
                        Historical {shortOid(candidate.head_oid)} ·{" "}
                        {formatTime(candidate.created_at_ms)}{" "}
                        <Button
                          size="sm"
                          variant="ghost"
                          onClick={() =>
                            dispatch({
                              type: "select_candidate",
                              candidateId: null,
                            })
                          }
                        >
                          Back to current
                        </Button>
                      </span>
                    )}
                  </>
                }
                reviewControls={
                  <div className={styles.unifiedControls}>
                    <SegmentedControl
                      size="sm"
                      value={view.navTab}
                      onValueChange={(value) =>
                        dispatch({
                          type: "set_nav_tab",
                          tab: value === "tour" ? "tour" : "files",
                        })
                      }
                      options={[
                        { value: "tour", label: "Tour" },
                        { value: "files", label: "Review" },
                      ]}
                    />
                    <ReviewToolbar
                      compact
                      title={review.title}
                      reference={
                        !stackHeader &&
                        reference &&
                        (link ? (
                          <a
                            className={styles.reference}
                            href={link.href}
                            title={reference}
                            target="_blank"
                            rel="noreferrer"
                          >
                            {link.label}
                          </a>
                        ) : (
                          <span className={styles.reference} title={reference}>
                            {reference}
                          </span>
                        ))
                      }
                      candidate={candidate}
                      candidates={candidates}
                      currentCandidateId={review.current_candidate_id}
                      compare={view.compare}
                      sinceLastAvailable={previous !== null}
                      diffMode={view.diffMode}
                      followTour={view.followTour}
                      hasTour={review.tour.length > 0}
                      showUnchanged={view.showUnchanged}
                      sourcesOpen={view.sourcesOpen}
                      workersOpen={view.activityOpen}
                      sourcesId={`review-${review.id}-sources`}
                      workersId={`review-${review.id}-workers`}
                      activeWorkers={activeRuns}
                      onCandidate={(value) =>
                        dispatch({
                          type: "select_candidate",
                          candidateId:
                            value === review.current_candidate_id
                              ? null
                              : value,
                        })
                      }
                      onCompare={(compare) =>
                        dispatch({ type: "set_compare", compare })
                      }
                      onDiffMode={(diffMode) => {
                        setLayoutChosen(true);
                        dispatch({ type: "set_diff_mode", diffMode });
                      }}
                      onFollow={() => {
                        if (view.followTour) {
                          dispatch({ type: "stop_follow" });
                          return;
                        }
                        const stepId =
                          view.tourStepId ?? review.tour.at(0)?.id ?? null;
                        dispatch({
                          type: "start_follow",
                          stepId,
                          path: stepId ? tourStepPath(review, stepId) : null,
                        });
                      }}
                      onUnchanged={() => dispatch({ type: "toggle_unchanged" })}
                      onSources={() => dispatch({ type: "toggle_sources" })}
                      onWorkers={() => dispatch({ type: "toggle_activity" })}
                      handoff={
                        <ReviewHandoff
                          reviewId={review.id}
                          revision={review.revision}
                          disabled={readOnly}
                          load={() =>
                            loadHandoff({ reviewId: review.id }).unwrap()
                          }
                        />
                      }
                    />
                  </div>
                }
              />

              {stalled.length > 0 && (
                <div className={styles.notice} role="status">
                  <span className={styles.grow}>
                    {stalled.length} item(s) lost their code location.
                    {reattach
                      ? ` Select lines in the diff, then reattach “${reattach.label}”.`
                      : ""}
                  </span>
                  {reattach ? (
                    <>
                      <Button
                        type="button"
                        size="sm"
                        variant="primary"
                        disabled={!selection || readOnly}
                        loading={
                          resolveState.isLoading || mutateState.isLoading
                        }
                        onClick={() => void applyReattach()}
                      >
                        Reattach to selection
                      </Button>
                      <Button
                        type="button"
                        size="sm"
                        variant="ghost"
                        onClick={() => setReattach(null)}
                      >
                        Cancel
                      </Button>
                    </>
                  ) : (
                    <Select
                      value=""
                      onValueChange={(value) =>
                        setReattach(
                          stalled.find(
                            (t) => `${t.kind}:${t.id}:${t.index}` === value,
                          ) ?? null,
                        )
                      }
                    >
                      <Select.Trigger
                        aria-label="Reattach a stalled item"
                        placeholder="Reattach…"
                      />
                      <Select.Content>
                        {stalled.map((t) => (
                          <Select.Item
                            key={`${t.kind}:${t.id}:${t.index}`}
                            value={`${t.kind}:${t.id}:${t.index}`}
                          >
                            {`${label(t.kind)}: ${t.label} — ${
                              t.anchor.stalled_reason ?? "location unavailable"
                            }`}
                          </Select.Item>
                        ))}
                      </Select.Content>
                    </Select>
                  )}
                </div>
              )}

              <div className={styles.body}>
                <nav
                  className={styles.nav}
                  aria-label="Review navigation"
                  data-open={navigationOpen}
                >
                  <div
                    className={styles.navigationToggle}
                    ref={navigationToggleRef}
                  >
                    <Button
                      size="sm"
                      variant="ghost"
                      aria-expanded={navigationOpen}
                      aria-controls={navigationId}
                      onClick={() => setNavigationOpen((open) => !open)}
                    >
                      {navigationOpen ? "Hide" : "Browse"}{" "}
                      {view.navTab === "tour" ? "topics" : "files"}
                    </Button>
                    <span
                      className={styles.navigationSelection}
                      title={
                        view.navTab === "tour"
                          ? review.tour.find(
                              (step) => step.id === view.tourStepId,
                            )?.current.title
                          : path ?? undefined
                      }
                    >
                      {view.navTab === "tour"
                        ? review.tour.find(
                            (step) => step.id === view.tourStepId,
                          )?.current.title
                        : path}
                    </span>
                  </div>
                  <div className={styles.navigationLists} id={navigationId}>
                    {view.navTab === "files" ? (
                      <>
                        <ul
                          className={styles.navList}
                          aria-label="Changed files"
                        >
                          {files.map((f) => (
                            <li key={f.path}>
                              <button
                                type="button"
                                className={styles.navItem}
                                aria-current={f.path === path}
                                aria-label={f.path}
                                title={f.path}
                                onClick={() => {
                                  closeNavigation();
                                  dispatch({
                                    type: "select_file",
                                    path: f.path,
                                  });
                                }}
                              >
                                <span
                                  className={styles.statusLetter}
                                  data-status={f.status}
                                  aria-label={label(f.status)}
                                >
                                  {FILE_STATUS_LETTER[f.status]}
                                </span>
                                <span className={styles.fileIdentity}>
                                  <span className={styles.fileBasename}>
                                    {f.path.split("/").at(-1)}
                                  </span>
                                  {f.path.includes("/") && (
                                    <span className={styles.fileParent}>
                                      {f.path.split("/").length > 3 ? "…/" : ""}
                                      {f.path
                                        .split("/")
                                        .slice(-3, -1)
                                        .join("/")}
                                    </span>
                                  )}
                                </span>
                                {f.findingCount > 0 && (
                                  <Badge tone="warning" size="xs">
                                    {f.findingCount}
                                  </Badge>
                                )}
                                {f.threadCount > 0 && (
                                  <Badge tone="accent" size="xs">
                                    {f.threadCount}
                                  </Badge>
                                )}
                              </button>
                            </li>
                          ))}
                          {files.length === 0 && (
                            <li className={styles.meta}>No changed files.</li>
                          )}
                        </ul>
                      </>
                    ) : (
                      <>
                        <ol className={styles.navList} aria-label="Tour steps">
                          {review.tour.map((step, index) => (
                            <li key={step.id}>
                              <button
                                type="button"
                                className={styles.navItem}
                                aria-current={step.id === view.tourStepId}
                                title={step.current.title}
                                onClick={() => selectStep(step.id)}
                              >
                                <span className={styles.statusLetter}>
                                  {index + 1}
                                </span>
                                <span className={styles.topicTitle}>
                                  {step.current.title}
                                </span>
                                {step.current.version > 1 && (
                                  <Badge tone="muted" size="xs">
                                    v{step.current.version}
                                  </Badge>
                                )}
                              </button>
                            </li>
                          ))}
                          {review.tour.length === 0 && (
                            <li className={styles.meta}>No tour yet.</li>
                          )}
                        </ol>
                      </>
                    )}
                  </div>
                </nav>

                <div className={styles.main}>
                  <div
                    className={styles.documentPane}
                    hidden={view.navTab !== "tour"}
                  >
                    <ReviewDocument
                      review={review}
                      revealTarget={documentTarget}
                      stepId={view.tourStepId}
                      onManualScroll={() => dispatch({ type: "stop_follow" })}
                      follow={view.followTour}
                      active={active && view.navTab === "tour"}
                      onRevealAnchor={revealExactAnchor}
                    />
                  </div>
                  <div
                    className={styles.reviewPane}
                    hidden={view.navTab !== "files"}
                  >
                    {path && candidate ? (
                      <>
                        <div className={styles.fileHeader}>
                          <span className={styles.filePath} title={path}>
                            {path}
                          </span>
                          {view.diffMode === "unified" &&
                            inlineHosts.left.length > 0 && (
                              <Tooltip>
                                <Tooltip.Trigger asChild>
                                  <IconButton
                                    icon={Columns2}
                                    aria-label={`Show ${inlineHosts.left.length} left-side discussions in Split`}
                                    size="sm"
                                    variant="soft"
                                    onClick={() => {
                                      dispatch({ type: "stop_follow" });
                                      setLayoutChosen(true);
                                      dispatch({
                                        type: "set_diff_mode",
                                        diffMode: "side-by-side",
                                      });
                                    }}
                                  />
                                </Tooltip.Trigger>
                                <Tooltip.Content>
                                  Show left-side discussions in Split
                                </Tooltip.Content>
                              </Tooltip>
                            )}
                          {selectionLabel && !reattach && !readOnly && (
                            <Tooltip>
                              <Tooltip.Trigger asChild>
                                <IconButton
                                  icon={MessageSquarePlus}
                                  aria-label={`Comment on ${selectionLabel}`}
                                  size="sm"
                                  variant="soft"
                                  loading={resolveState.isLoading}
                                  onClick={() => void startComment()}
                                />
                              </Tooltip.Trigger>
                              <Tooltip.Content>
                                Comment on {selectionLabel}
                              </Tooltip.Content>
                            </Tooltip>
                          )}
                        </div>
                        {actionError && (
                          <p role="alert" className={styles.errorText}>
                            {actionError}
                          </p>
                        )}
                        <div className={styles.diffArea}>
                          {panes && (
                            <ReviewFileDiff
                              review={review}
                              panes={panes}
                              diffMode={view.diffMode}
                              preferSingleSided={!layoutChosen}
                              path={path}
                              activeObjectId={view.activeObjectId}
                              reveal={reveal}
                              inlineHosts={inlineHosts}
                              onLineCounts={(counts) =>
                                setLineCounts((current) =>
                                  current?.identity === navigationIdentity &&
                                  current.left === counts.left &&
                                  current.right === counts.right
                                    ? current
                                    : {
                                        identity: navigationIdentity,
                                        ...counts,
                                      },
                                )
                              }
                              onCommentLine={(pane, line) => {
                                if (readOnly) return;
                                dispatch({ type: "stop_follow" });
                                const selected = {
                                  pane,
                                  range: { startLine: line, endLine: line },
                                };
                                setSelection(selected);
                                void openComment(selected);
                              }}
                              onSelection={(pane, range) =>
                                setSelection(range ? { pane, range } : null)
                              }
                            />
                          )}
                        </div>
                        <ReviewObjects
                          review={review}
                          fallbackOnly
                          placedIds={placedIds}
                          candidateId={candidate.id}
                          path={path}
                          activeObjectId={view.activeObjectId}
                          onFocusObject={(objectId) => {
                            const anchor = objectAnchor(review, objectId);
                            if (anchor) revealExactAnchor(anchor);
                            dispatch({ type: "focus_object", objectId, path });
                          }}
                        />
                      </>
                    ) : (
                      <p className={styles.unavailable}>
                        {candidate
                          ? "Select a file to review."
                          : "Waiting for the first candidate capture."}
                      </p>
                    )}
                  </div>
                </div>
              </div>

              {candidate && (
                <Drawer
                  title="Sources & Evidence"
                  placement="inspector"
                  chrome="external"
                  id={`review-${review.id}-sources`}
                  open={view.sourcesOpen}
                  onToggle={() => dispatch({ type: "toggle_sources" })}
                >
                  <SourcesAndEvidence
                    review={review}
                    candidateId={candidate.id}
                    selectedAnnotationId={
                      review.annotations.some(
                        (a) => a.id === view.activeObjectId,
                      )
                        ? view.activeObjectId
                        : null
                    }
                    onRevealSource={revealEvidence}
                  />
                </Drawer>
              )}
              {portals}
              <Drawer
                title={`Workers${
                  activeRuns > 0 ? ` (${activeRuns} active)` : ""
                }`}
                chrome="external"
                id={`review-${review.id}-workers`}
                open={view.activityOpen}
                onToggle={() => dispatch({ type: "toggle_activity" })}
              >
                <WorkerActivity review={review} />
              </Drawer>
            </div>
          </RichDiscussionContext.Provider>
        </RichNavigationContext.Provider>
      </RichFileNavigationContext.Provider>
    </ReviewDocumentState>
  );
}
