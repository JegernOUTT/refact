import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getChunks } from "@codemirror/merge";
import { Button } from "../../../components/ui";
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

export type RevealRequest = {
  pane: PaneId;
  line: number;
  token: number;
  path?: string;
  scope?: string;
};

export type DiffPosition = {
  a?: ReturnType<EditorView["scrollSnapshot"]>;
  b: ReturnType<EditorView["scrollSnapshot"]>;
  topA?: number;
  topB: number;
  chunk: number;
  revealToken?: number;
};

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
          };
        },
        write: ({ gutter, width }, view) => {
          for (const [name, value] of [
            ["--rf-review-inline-width", width],
            ["--rf-review-gutter-width", gutter],
          ] as const) {
            const size = `${value}px`;
            if (view.dom.style.getPropertyValue(name) !== size)
              view.dom.style.setProperty(name, size);
          }
        },
      });
    }
  },
);

export type ReviewDiffEditorsProps = {
  navigationKey?: string;
  positions?: Map<string, DiffPosition>;
  autoRevealFirstChange?: boolean;
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
  navigationKey,
  positions,
  autoRevealFirstChange = true,
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
  const [chunkState, setChunkState] = useState({ index: -1, count: 0 });
  const chunkIndex = useRef(-1);
  const binding = useRef<{
    key: string;
    positions?: Map<string, DiffPosition>;
    revealToken?: number;
  }>();
  const controllerRef = useRef<DiffEditorsController | null>(null);
  const cancelRestore = useRef<(() => void) | null>(null);
  const latest = useRef({
    navigationKey,
    positions,
    onSelection,
    highlights,
    reveal,
    inlineHosts,
    onCommentLine,
  });
  latest.current = {
    navigationKey,
    positions,
    onSelection,
    highlights,
    reveal,
    inlineHosts,
    onCommentLine,
  };
  const savePosition = () => {
    const views = controllerRef.current?.views();
    const current = binding.current;
    if (!views || !current || cancelRestore.current) return;
    current.positions?.delete(current.key);
    current.positions?.set(current.key, {
      a: views.a?.scrollSnapshot(),
      b: views.b.scrollSnapshot(),
      topA: views.a?.scrollDOM.scrollTop,
      topB: views.b.scrollDOM.scrollTop,
      chunk: chunkIndex.current,
      revealToken: current.revealToken,
    });
    if (current.positions && current.positions.size > 100) {
      const oldest = current.positions.keys().next().value;
      if (oldest !== undefined) current.positions.delete(oldest);
    }
  };
  const manualScroll = useRef(false);
  const startManualScroll = () => {
    cancelRestore.current?.();
    mirroredScroll.current = new WeakMap();
    manualScroll.current = true;
  };
  const mirroredScroll = useRef(new WeakMap<EditorView, number>());
  const trackScroll = (view: EditorView, side: "a" | "b") => {
    if (cancelRestore.current) return;
    const top = view.scrollDOM.scrollTop;
    const mirrored = mirroredScroll.current.get(view) === top;
    mirroredScroll.current.delete(view);
    if (mirrored) return;
    const views = controllerRef.current?.views();
    const other = side === "a" ? views?.b : views?.a;
    if (other && other.scrollDOM.scrollTop !== top) {
      const before = other.scrollDOM.scrollTop;
      other.scrollDOM.scrollTop = top;
      const applied = other.scrollDOM.scrollTop;
      if (applied !== before) mirroredScroll.current.set(other, applied);
    }
    const chunks = getChunks(view.state)?.chunks ?? [];
    if (chunks.length && manualScroll.current) {
      const top = view.lineBlockAtHeight(
        view.scrollDOM.scrollTop + view.scrollDOM.clientHeight / 2,
      ).from;
      let index = 0;
      for (let i = 0; i < chunks.length; i++) {
        if ((side === "a" ? chunks[i].fromA : chunks[i].fromB) <= top)
          index = i;
      }
      chunkIndex.current = index;
      setChunkState({ index, count: chunks.length });
    }
    saveLatest.current();
  };
  const trackLatest = useRef(trackScroll);
  trackLatest.current = trackScroll;
  const saveLatest = useRef(savePosition);
  saveLatest.current = savePosition;
  const extensions = useMemo(
    () => ({
      a: [
        EditorView.domEventHandlers({
          wheel: () => {
            startManualScroll();
          },
          pointerdown: () => {
            startManualScroll();
          },
          touchstart: () => {
            startManualScroll();
          },
          keydown: (event) => {
            if (
              [
                "ArrowUp",
                "ArrowDown",
                "PageUp",
                "PageDown",
                "Home",
                "End",
                " ",
              ].includes(event.key)
            )
              startManualScroll();
          },
          scroll: (_event, view) => trackLatest.current(view, "a"),
        }),
        inlinePaneSize,
        inlineDiscussionExtension(
          (line) => latest.current.onCommentLine?.("left", line),
        ),
        reviewSideExtension((range) =>
          latest.current.onSelection("left", range),
        ),
      ],
      b: [
        EditorView.domEventHandlers({
          wheel: () => {
            startManualScroll();
          },
          pointerdown: () => {
            startManualScroll();
          },
          touchstart: () => {
            startManualScroll();
          },
          keydown: (event) => {
            if (
              [
                "ArrowUp",
                "ArrowDown",
                "PageUp",
                "PageDown",
                "Home",
                "End",
                " ",
              ].includes(event.key)
            )
              startManualScroll();
          },
          scroll: (_event, view) => trackLatest.current(view, "b"),
        }),
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
  const applyReveal = useCallback((request: RevealRequest) => {
    cancelRestore.current?.();
    manualScroll.current = false;
    const views = controllerRef.current?.views();
    if (!views) return;
    if (binding.current) binding.current.revealToken = request.token;
    const view = request.pane === "left" ? views.a : views.b;
    if (view) {
      const chunks = getChunks(view.state)?.chunks ?? [];
      const position = view.state.doc.line(
        Math.max(1, Math.min(request.line, view.state.doc.lines)),
      ).from;
      let index = 0;
      chunks.forEach((chunk, i) => {
        if ((request.pane === "left" ? chunk.fromA : chunk.fromB) <= position)
          index = i;
      });
      chunkIndex.current = chunks.length ? index : -1;
      setChunkState({ index: chunkIndex.current, count: chunks.length });
      revealLine(view, request.line);
    }
  }, []);
  const onController = useCallback(
    (controller: DiffEditorsController | null) => {
      const previous = controllerRef.current;
      if (previous && binding.current && previous.views().b.dom.isConnected)
        saveLatest.current();
      cancelRestore.current?.();
      controllerRef.current = controller;
      if (!controller) return;
      const key = latest.current.navigationKey ?? "";
      binding.current = {
        key,
        positions: latest.current.positions,
        revealToken: latest.current.reveal?.token,
      };
      const cached = latest.current.positions?.get(key);
      const views = controller.views();
      const chunks = getChunks(views.b.state)?.chunks ?? [];
      chunkIndex.current = cached?.chunk ?? (chunks.length ? 0 : -1);
      setChunkState({ index: chunkIndex.current, count: chunks.length });
      if (views.a) applyHighlights(views.a, latest.current.highlights.left);
      applyHighlights(views.b, latest.current.highlights.right);
      if (views.a)
        applyInlineHosts(views.a, latest.current.inlineHosts?.left ?? []);
      applyInlineHosts(views.b, latest.current.inlineHosts?.right ?? []);
      const request = latest.current.reveal;
      if (request && request.token !== cached?.revealToken) {
        applyReveal(request);
      } else if (cached) {
        if (views.a && cached.a) views.a.dispatch({ effects: cached.a });
        views.b.dispatch({ effects: cached.b });
        let frame = 0;
        let remaining = 12;
        let stable = 0;
        let geometry = "";
        const panes = [views.a, views.b].filter(
          (view): view is EditorView => !!view,
        );
        const interrupt = () => {
          cancel();
          mirroredScroll.current = new WeakMap();
          manualScroll.current = true;
        };
        const cancel = () => {
          cancelAnimationFrame(frame);
          for (const view of panes)
            for (const type of ["wheel", "pointerdown", "touchstart"])
              view.scrollDOM.removeEventListener(type, interrupt, true);
          cancelRestore.current = null;
        };
        for (const view of panes)
          for (const type of ["wheel", "pointerdown", "touchstart"])
            view.scrollDOM.addEventListener(type, interrupt, true);
        cancelRestore.current = cancel;
        const restore = () => {
          if (cancelRestore.current !== cancel) return;
          for (const view of panes) view.lineBlockAtHeight(cached.topB);
          const measured = panes
            .map((view) =>
              [
                view.contentHeight,
                view.scrollDOM.clientHeight,
                view.scrollDOM.clientWidth,
              ].join(":"),
            )
            .join(";");
          const aligned = panes.every(
            (view) => view.scrollDOM.scrollTop === cached.topB,
          );
          stable = aligned && measured === geometry ? stable + 1 : 0;
          geometry = measured;
          for (const view of panes) {
            const before = view.scrollDOM.scrollTop;
            view.scrollDOM.scrollTop = cached.topB;
            if (view.scrollDOM.scrollTop !== before)
              mirroredScroll.current.set(view, view.scrollDOM.scrollTop);
            view.requestMeasure();
          }
          if (stable >= 2 || --remaining === 0) {
            cancel();
            if (stable >= 2) saveLatest.current();
          } else frame = requestAnimationFrame(restore);
        };
        frame = requestAnimationFrame(restore);
      } else if (chunks.length && autoRevealFirstChange) {
        const chunk = chunks[0];
        for (const [view, offset] of [
          [views.a, chunk.fromA],
          [views.b, chunk.fromB],
        ] as const) {
          if (view)
            revealLine(
              view,
              view.state.doc.lineAt(Math.min(offset, view.state.doc.length))
                .number,
            );
        }
      }
    },
    [applyReveal, autoRevealFirstChange],
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
    manualScroll.current = false;
    const views = controllerRef.current?.views();
    if (!views || !reveal || reveal.token === binding.current?.revealToken)
      return;
    applyReveal(reveal);
  }, [reveal, applyReveal]);

  const navigateChunk = (direction: number) => {
    cancelRestore.current?.();
    manualScroll.current = false;
    const views = controllerRef.current?.views();
    if (!views) return;
    const chunks = getChunks(views.b.state)?.chunks ?? [];
    if (!chunks.length) return;
    const index = Math.max(
      0,
      Math.min(chunks.length - 1, chunkIndex.current + direction),
    );
    chunkIndex.current = index;
    saveLatest.current();
    setChunkState({ index, count: chunks.length });
    const chunk = chunks[index];
    for (const [view, offset] of [
      [views.a, chunk.fromA],
      [views.b, chunk.fromB],
    ] as const) {
      if (view)
        revealLine(
          view,
          view.state.doc.lineAt(Math.min(offset, view.state.doc.length)).number,
        );
    }
  };

  return (
    <>
      <div className={styles.fileHeader} role="group" aria-label="Diff hunks">
        <Button
          size="sm"
          variant="ghost"
          disabled={chunkState.index <= 0}
          onClick={() => navigateChunk(-1)}
        >
          Previous change
        </Button>
        <span role="status">
          {chunkState.count ? chunkState.index + 1 : 0} / {chunkState.count}{" "}
          changes
        </span>
        <Button
          size="sm"
          variant="ghost"
          disabled={chunkState.index >= chunkState.count - 1}
          onClick={() => navigateChunk(1)}
        >
          Next change
        </Button>
      </div>
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
    </>
  );
}
