import { useCallback, useEffect, useMemo, useRef } from "react";
import { EditorView, ViewPlugin, type ViewUpdate } from "@codemirror/view";

import { DiffEditorsHost } from "../../Workspace/GitPanel/diff/DiffEditorsHost";
import {
  type DiffEditorsController,
  type DiffMode,
} from "../../Workspace/GitPanel/diff/diffEditors";
import type { PaneId } from "../model";
import {
  applyHighlights,
  revealLine,
  reviewSideExtension,
  type LineHighlight,
  type LineRange,
} from "./reviewHighlights";
import styles from "../Review.module.css";
import {
  applyInlineHosts,
  inlineDiscussionExtension,
  type InlineHost,
} from "./inlineDiscussions";

export type RevealRequest = { pane: PaneId; line: number; token: number };

// cm-content follows the longest unwrapped line, not the visible pane. Keep
// discussion layout independent of that width without changing code scrolling.
const inlinePaneSize = ViewPlugin.fromClass(
  class {
    constructor(readonly view: EditorView) {
      this.measure();
    }

    update(update: ViewUpdate) {
      if (update.geometryChanged) this.measure();
    }

    measure() {
      this.view.requestMeasure({
        key: this,
        read: (view) => {
          const gutter =
            view.scrollDOM.querySelector<HTMLElement>(".cm-gutters")
              ?.offsetWidth ?? 0;
          return {
            gutter,
            width: Math.max(0, view.scrollDOM.clientWidth - gutter),
            height: view.contentHeight,
          };
        },
        write: ({ gutter, width, height }, view) => {
          for (const [name, value] of [
            ["--rf-review-inline-width", width],
            ["--rf-review-gutter-width", gutter],
          ] as const) {
            const size = `${value}px`;
            if (view.dom.style.getPropertyValue(name) !== size)
              view.dom.style.setProperty(name, size);
          }
          // Widget heights are included; retain both pane measurements so a
          // shorter pane cannot clip the taller one. The flex parent caps it.
          if (height > 0) {
            view.dom.dataset.reviewContentHeight = String(height);
            const area = view.dom.closest<HTMLElement>(`.${styles.diffArea}`);
            if (area) {
              const heights = Array.from(
                area.querySelectorAll<HTMLElement>(".cm-editor"),
                (editor) => Number(editor.dataset.reviewContentHeight) || 0,
              );
              const contentHeight = Math.ceil(Math.max(...heights));
              const size = `${contentHeight}px`;
              if (area.style.height !== size) area.style.height = size;
              const canvas = area.closest<HTMLElement>(`.${styles.canvas}`);
              if (canvas && canvas.clientHeight > 0) {
                area.dataset.fit = String(
                  contentHeight <= canvas.clientHeight * 0.65,
                );
              }
              area.dataset.measured = "true";
            }
          }
        },
      });
    }
  },
);

export type ReviewDiffEditorsProps = {
  mode: DiffMode;
  left: string;
  right: string;
  path: string;
  leftLabel: string;
  rightLabel: string;
  highlights: { left: LineHighlight[]; right: LineHighlight[] };
  reveal: RevealRequest | null;
  inlineHosts?: { left: InlineHost[]; right: InlineHost[] };
  onCommentLine?: (pane: PaneId, line: number) => void;
  contentSide?: "base";
  onSelection: (pane: PaneId, range: LineRange | null) => void;
};

export function ReviewDiffEditors({
  mode,
  left,
  right,
  path,
  leftLabel,
  rightLabel,
  highlights,
  reveal,
  inlineHosts,
  onCommentLine,
  contentSide,
  onSelection,
}: ReviewDiffEditorsProps) {
  const controllerRef = useRef<DiffEditorsController | null>(null);
  const latest = useRef({
    onSelection,
    highlights,
    reveal,
    inlineHosts,
    onCommentLine,
  });
  latest.current = {
    onSelection,
    highlights,
    reveal,
    inlineHosts,
    onCommentLine,
  };
  const extensions = useMemo(
    () => ({
      a: [
        inlinePaneSize,
        inlineDiscussionExtension(
          (line) => latest.current.onCommentLine?.("left", line),
        ),
        reviewSideExtension((range) =>
          latest.current.onSelection("left", range),
        ),
      ],
      b: [
        inlinePaneSize,
        inlineDiscussionExtension(
          (line) => latest.current.onCommentLine?.("right", line),
        ),
        reviewSideExtension((range) =>
          latest.current.onSelection("right", range),
        ),
      ],
    }),
    [],
  );
  const onController = useCallback(
    (controller: DiffEditorsController | null) => {
      controllerRef.current = controller;
      if (!controller) return;
      const views = controller.views();
      if (views.a) applyHighlights(views.a, latest.current.highlights.left);
      applyHighlights(views.b, latest.current.highlights.right);
      if (views.a)
        applyInlineHosts(views.a, latest.current.inlineHosts?.left ?? []);
      applyInlineHosts(views.b, latest.current.inlineHosts?.right ?? []);
      const request = latest.current.reveal;
      if (request) {
        const view = request.pane === "left" ? views.a : views.b;
        if (view) revealLine(view, request.line);
      }
    },
    [],
  );

  useEffect(() => {
    const views = controllerRef.current?.views();
    if (!views) return;
    if (views.a) applyHighlights(views.a, highlights.left);
    applyHighlights(views.b, highlights.right);
  }, [highlights]);

  useEffect(() => {
    const views = controllerRef.current?.views();
    if (!views) return;
    if (views.a) applyInlineHosts(views.a, inlineHosts?.left ?? []);
    applyInlineHosts(views.b, inlineHosts?.right ?? []);
  }, [inlineHosts]);

  useEffect(() => {
    const views = controllerRef.current?.views();
    if (!views || !reveal) return;
    const view = reveal.pane === "left" ? views.a : views.b;
    if (view) revealLine(view, reveal.line);
  }, [reveal]);

  return (
    <DiffEditorsHost
      mode={mode}
      left={left}
      right={right}
      languagePath={path}
      leftLabel={leftLabel}
      rightLabel={rightLabel}
      collapseUnchanged={false}
      extensions={extensions}
      onController={onController}
      className={
        contentSide === "base"
          ? `${styles.diffHost} ${styles.deletedOnly}`
          : styles.diffHost
      }
      testId="review-diff-editors"
    />
  );
}
