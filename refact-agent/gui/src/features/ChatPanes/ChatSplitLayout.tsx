import classNames from "classnames";
import {
  CSSProperties,
  MouseEvent as ReactMouseEvent,
  useCallback,
  useRef,
  useState,
} from "react";

import { useAppDispatch, useAppSelector } from "../../hooks";
import { useDashboardLayout } from "../Dashboard/hooks/useDashboardLayout";
import type { PaneNode, SplitNode } from "./panesTree";
import { resizeSplit, selectPaneRoot } from "./panesSlice";
import { ChatPane } from "./ChatPane";
import styles from "./ChatSplitLayout.module.css";

const MIN_RESIZE_FRACTION = 0.12;

type PaneSlotStyle = CSSProperties & {
  "--pane-flex": number;
};

type SplitViewProps = {
  node: SplitNode;
  stacked: boolean;
};

type DividerProps = {
  dir: SplitNode["dir"];
  dragging: boolean;
  onMouseDown: (event: ReactMouseEvent<HTMLDivElement>) => void;
};

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}

function normalizedSizes(sizes: number[], count: number): number[] {
  if (count === 0) return [];
  if (sizes.length !== count) {
    return Array.from({ length: count }, () => 1 / count);
  }

  const positive = sizes.map((size) =>
    Number.isFinite(size) && size > 0 ? size : 0,
  );
  const total = positive.reduce((sum, size) => sum + size, 0);
  if (total <= 0) return Array.from({ length: count }, () => 1 / count);
  return positive.map((size) => size / total);
}

function resizeAtDivider(
  sizes: number[],
  dividerIndex: number,
  pointerFraction: number,
): number[] {
  const next = [...sizes];
  const before = next
    .slice(0, dividerIndex)
    .reduce((sum, size) => sum + size, 0);
  const pairTotal = next[dividerIndex] + next[dividerIndex + 1];
  if (pairTotal <= 0) return sizes;

  const minimum = Math.min(MIN_RESIZE_FRACTION, pairTotal / 2);
  const boundary = clamp(
    pointerFraction,
    before + minimum,
    before + pairTotal - minimum,
  );

  next[dividerIndex] = boundary - before;
  next[dividerIndex + 1] = before + pairTotal - boundary;
  return normalizedSizes(next, next.length);
}

function PaneDivider({ dir, dragging, onMouseDown }: DividerProps) {
  const vertical = dir === "row";

  return (
    <div
      className={classNames(
        styles.divider,
        vertical ? styles.verticalDivider : styles.horizontalDivider,
      )}
      data-testid={
        vertical ? "pane-vertical-divider" : "pane-horizontal-divider"
      }
      data-dragging={dragging || undefined}
      role="separator"
      aria-orientation={vertical ? "vertical" : "horizontal"}
      onMouseDown={onMouseDown}
    >
      <div className={styles.dividerHandle} />
    </div>
  );
}

function PaneNodeView({ node, stacked }: { node: PaneNode; stacked: boolean }) {
  if (node.kind === "leaf") {
    return <ChatPane leafId={node.id} />;
  }

  return <SplitView node={node} stacked={stacked} />;
}

function SplitView({ node, stacked }: SplitViewProps) {
  const dispatch = useAppDispatch();
  const containerRef = useRef<HTMLDivElement>(null);
  const [draggingDivider, setDraggingDivider] = useState<number | null>(null);
  const sizes = normalizedSizes(node.sizes, node.children.length);

  const handleDividerMouseDown = useCallback(
    (dividerIndex: number, event: ReactMouseEvent<HTMLDivElement>) => {
      event.preventDefault();
      const container = containerRef.current;
      if (!container) return;

      setDraggingDivider(dividerIndex);
      document.body.style.cursor =
        node.dir === "row" ? "col-resize" : "row-resize";
      document.body.style.userSelect = "none";

      const handleMouseMove = (moveEvent: MouseEvent) => {
        const rect = container.getBoundingClientRect();
        const length = node.dir === "row" ? rect.width : rect.height;
        if (!Number.isFinite(length) || length <= 0) return;

        const start = node.dir === "row" ? rect.left : rect.top;
        const pointer =
          node.dir === "row" ? moveEvent.clientX : moveEvent.clientY;
        const nextSizes = resizeAtDivider(
          sizes,
          dividerIndex,
          (pointer - start) / length,
        );

        dispatch(resizeSplit({ splitId: node.id, sizes: nextSizes }));
      };

      const handleMouseUp = () => {
        setDraggingDivider(null);
        document.body.style.cursor = "";
        document.body.style.userSelect = "";
        window.removeEventListener("mousemove", handleMouseMove);
        window.removeEventListener("mouseup", handleMouseUp);
      };

      window.addEventListener("mousemove", handleMouseMove);
      window.addEventListener("mouseup", handleMouseUp);
    },
    [dispatch, node.dir, node.id, sizes],
  );

  return (
    <div
      ref={containerRef}
      className={classNames(
        styles.split,
        node.dir === "row" ? styles.row : styles.col,
        stacked && styles.stackedSplit,
      )}
      data-pane-split-id={node.id}
      data-pane-split-dir={node.dir}
    >
      {node.children.map((child, index) => (
        <div
          key={child.id}
          className={styles.paneSlot}
          style={{ "--pane-flex": sizes[index] } as PaneSlotStyle}
        >
          <PaneNodeView node={child} stacked={stacked} />
          {!stacked && index < node.children.length - 1 ? (
            <PaneDivider
              dir={node.dir}
              dragging={draggingDivider === index}
              onMouseDown={(event) => handleDividerMouseDown(index, event)}
            />
          ) : null}
        </div>
      ))}
    </div>
  );
}

export function ChatSplitLayout() {
  const containerRef = useRef<HTMLDivElement>(null);
  const root = useAppSelector(selectPaneRoot);
  const breakpoint = useDashboardLayout(containerRef);
  const stacked = breakpoint !== "wide";

  return (
    <div
      ref={containerRef}
      className={classNames(styles.layout, stacked && styles.stackedLayout)}
      data-breakpoint={breakpoint}
      data-stacked={stacked || undefined}
    >
      <PaneNodeView node={root} stacked={stacked} />
    </div>
  );
}
