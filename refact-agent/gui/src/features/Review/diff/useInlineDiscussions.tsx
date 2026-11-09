import { useRef, type ReactNode } from "react";
import { createPortal } from "react-dom";
import type { Anchor, Review } from "../../../services/refact/review";
import { SharedReviewComposer } from "../ReviewRichBlocks";
import { AnnotationCard, ThreadCard } from "../ReviewObjects";
import { paneFor, type DiffPanes } from "../model";
import type { InlineHost } from "./inlineDiscussions";
import { usePendingDeliveries } from "./usePendingDeliveries";

export function useInlineDiscussions({
  review,
  panes,
  path,
  draftAnchors,
  focusAllowed,
  lineCounts,
  onCloseDraft,
  activeObjectId,
  onFocusObject,
  active = true,
}: {
  review: Review;
  panes: DiffPanes | null;
  path: string | null;
  draftAnchors: Anchor[];
  focusAllowed?: (anchor: Anchor) => boolean;
  lineCounts?: { left: number; right: number };
  onCloseDraft: (anchor: Anchor) => void;
  activeObjectId: string | null;
  onFocusObject: (id: string) => void;
  active?: boolean;
}) {
  const hosts = useRef(new Map<string, HTMLElement>());
  const pending = usePendingDeliveries(
    review.id,
    active &&
      review.threads.some((t) =>
        t.contributions.some((c) => c.actor.kind === "user"),
      ),
  );
  const inlineHosts = { left: [] as InlineHost[], right: [] as InlineHost[] };
  const portals: ReactNode[] = [];
  const placedIds = new Set<string>();
  const live = new Set<string>();
  const add = (id: string, anchors: Anchor[], content: ReactNode) => {
    live.add(id);
    let host = hosts.current.get(id);
    if (!host) {
      host = document.createElement("div");
      host.className = "rf-review-inline-discussion";
      host.dataset.discussionId = id;
      hosts.current.set(id, host);
    }
    const anchor = anchors.find(
      (a) =>
        a.status === "current" &&
        a.position.path === path &&
        panes &&
        paneFor(panes, a.position),
    );
    const pane = anchor && panes ? paneFor(panes, anchor.position) : null;
    if (
      anchor &&
      pane &&
      anchor.position.start_line >= 1 &&
      anchor.position.end_line >= anchor.position.start_line &&
      (!lineCounts || anchor.position.end_line <= lineCounts[pane])
    ) {
      inlineHosts[pane].push({ id, endLine: anchor.position.end_line, host });
      placedIds.add(id);
    }
    portals.push(createPortal(content, host, id));
  };
  const visible = (anchors: Anchor[]) =>
    anchors.some(
      (anchor) =>
        anchor.status === "current" &&
        anchor.position.path === path &&
        panes &&
        paneFor(panes, anchor.position),
    );
  for (const annotation of review.annotations.filter((item) =>
    visible(item.anchors),
  ))
    add(
      `annotation:${annotation.id}`,
      annotation.anchors,
      <AnnotationCard
        review={review}
        annotation={annotation}
        active={activeObjectId === annotation.id}
        onFocus={() => onFocusObject(annotation.id)}
      />,
    );
  for (const thread of review.threads.filter((item) => visible(item.anchors)))
    add(
      `thread:${thread.id}`,
      thread.anchors,
      <ThreadCard
        review={review}
        thread={thread}
        pending={pending}
        active={activeObjectId === thread.id}
        onFocus={() => onFocusObject(thread.id)}
      />,
    );
  for (const draftAnchor of draftAnchors) {
    const id = `draft:${JSON.stringify(draftAnchor.position)}`;
    const fresh = !hosts.current.has(id);
    add(
      id,
      [draftAnchor],
      <SharedReviewComposer
        reviewId={review.id}
        revision={review.revision}
        anchor={draftAnchor}
        onClose={() => onCloseDraft(draftAnchor)}
      />,
    );
    const host = hosts.current.get(id);
    if (fresh && host && active && (focusAllowed?.(draftAnchor) ?? true))
      host.dataset.focusPending = "true";
  }
  for (const id of hosts.current.keys())
    if (!live.has(id)) hosts.current.delete(id);
  return { inlineHosts, portals, placedIds };
}
