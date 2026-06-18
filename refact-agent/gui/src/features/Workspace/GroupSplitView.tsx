import classNames from "classnames";
import { Columns3, Rows3, X } from "lucide-react";
import {
  type CSSProperties,
  type DragEvent,
  type MouseEvent as ReactMouseEvent,
  useCallback,
  useRef,
  useState,
} from "react";

import { IconButton, Tooltip } from "../../components/ui";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { useDashboardLayout } from "../Dashboard/hooks/useDashboardLayout";
import type { LeafPane, PaneNode, SplitNode } from "../ChatPanes/panesTree";
import {
  addSurfaceToPane,
  closePane,
  focusPane,
  resizeGroupSplit,
  selectGroupForTab,
  splitPaneWithSurface,
  splitTab,
} from "./workspaceSlice";
import { isChatSurface, type SurfaceKey } from "./surfaceKey";
import {
  hasTabDragType,
  readTabDragSurfaceKey,
} from "../ChatPanes/tabDrag";
import { SurfacePane } from "./SurfacePane";
import styles from "./GroupSplitView.module.css";

const MIN_RESIZE_FRACTION = 0.12;

type PaneDropEdge = "left" | "right" | "top" | "bottom";

const paneDropEdges: PaneDropEdge[] = ["left", "right", "top", "bottom"];

const paneDropDirections: Record<PaneDropEdge, "row" | "col"> = {
  left: "row",
  right: "row",
  top: "col",
  bottom: "col",
};

const paneDropPlacements: Record<PaneDropEdge, "before" | "after"> = {
  left: "before",
  right: "after",
  top: "before",
  bottom: "after",
};

const paneDropEdgeClasses: Record<PaneDropEdge, string> = {
  left: styles.edgeDropLeft,
  right: styles.edgeDropRight,
  top: styles.edgeDropTop,
  bottom: styles.edgeDropBottom,
};

type PaneSlotStyle = CSSProperties & {
  "--pane-flex": number;
};

type DividerProps = {
  dir: SplitNode["dir"];
  dragging: boolean;
  onMouseDown: (event: ReactMouseEvent<HTMLDivElement>) => void;
};

type SplitViewProps = {
  node: SplitNode;
  tabId: SurfaceKey;
  focusedLeafId: string;
  stacked: boolean;
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

function hasChatTabDrag(dataTransfer: DataTransfer): boolean {
  return hasTabDragType(dataTransfer, "chat");
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
        vertical ? "workspace-vertical-divider" : "workspace-horizontal-divider"
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

function PaneHeader({ leaf, tabId }: { leaf: LeafPane; tabId: SurfaceKey }) {
  const dispatch = useAppDispatch();
  const canSplit = Boolean(leaf.activeTabId ?? leaf.tabIds[0]);

  const handleSplitRight = useCallback(() => {
    dispatch(focusPane({ tabId, leafId: leaf.id }));
    dispatch(splitTab({ tabId, dir: "row" }));
  }, [dispatch, leaf.id, tabId]);

  const handleSplitDown = useCallback(() => {
    dispatch(focusPane({ tabId, leafId: leaf.id }));
    dispatch(splitTab({ tabId, dir: "col" }));
  }, [dispatch, leaf.id, tabId]);

  const handleClose = useCallback(() => {
    dispatch(closePane({ tabId, leafId: leaf.id }));
  }, [dispatch, leaf.id, tabId]);

  return (
    <div className={styles.paneHeader} aria-label="Workspace pane controls">
      <div className={styles.paneTitle}>Pane</div>
      <div className={styles.paneActions}>
        <Tooltip content="Split Right">
          <IconButton
            aria-label="Split Right"
            className={styles.paneAction}
            disabled={!canSplit}
            icon={Columns3}
            onClick={handleSplitRight}
            size="sm"
            variant="plain"
          />
        </Tooltip>
        <Tooltip content="Split Down">
          <IconButton
            aria-label="Split Down"
            className={styles.paneAction}
            disabled={!canSplit}
            icon={Rows3}
            onClick={handleSplitDown}
            size="sm"
            variant="plain"
          />
        </Tooltip>
        <Tooltip content="Close Pane">
          <IconButton
            aria-label="Close Pane"
            className={styles.paneAction}
            icon={X}
            onClick={handleClose}
            size="sm"
            variant="plain"
          />
        </Tooltip>
      </div>
    </div>
  );
}

function LeafView({
  leaf,
  tabId,
  focused,
}: {
  leaf: LeafPane;
  tabId: SurfaceKey;
  focused: boolean;
}) {
  const dispatch = useAppDispatch();
  const [surfaceDragActive, setSurfaceDragActive] = useState(false);
  const surfaceKey =
    leaf.activeTabId ?? (leaf.tabIds.length > 0 ? leaf.tabIds[0] : null);

  const handleFocusPane = useCallback(() => {
    dispatch(focusPane({ tabId, leafId: leaf.id }));
  }, [dispatch, leaf.id, tabId]);

  const handlePaneDragEnter = useCallback((event: DragEvent) => {
    if (!hasChatTabDrag(event.dataTransfer)) return;
    setSurfaceDragActive(true);
  }, []);

  const handlePaneDragOver = useCallback((event: DragEvent) => {
    if (!hasChatTabDrag(event.dataTransfer)) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "move";
    setSurfaceDragActive(true);
  }, []);

  const handlePaneDragLeave = useCallback((event: DragEvent<HTMLElement>) => {
    const nextTarget = event.relatedTarget;
    if (
      nextTarget instanceof Node &&
      event.currentTarget.contains(nextTarget)
    ) {
      return;
    }
    setSurfaceDragActive(false);
  }, []);

  const handlePaneDrop = useCallback(
    (event: DragEvent) => {
      const draggedSurfaceKey = readTabDragSurfaceKey(event.dataTransfer);
      if (!draggedSurfaceKey || !isChatSurface(draggedSurfaceKey)) {
        setSurfaceDragActive(false);
        return;
      }
      if (draggedSurfaceKey === surfaceKey) {
        setSurfaceDragActive(false);
        return;
      }

      event.preventDefault();
      event.stopPropagation();
      setSurfaceDragActive(false);
      dispatch(
        addSurfaceToPane({
          tabId,
          leafId: leaf.id,
          surfaceKey: draggedSurfaceKey,
        }),
      );
    },
    [dispatch, leaf.id, surfaceKey, tabId],
  );

  const handlePaneDragEnd = useCallback(() => {
    setSurfaceDragActive(false);
  }, []);

  const handleEdgeDragOver = useCallback((event: DragEvent) => {
    event.preventDefault();
    event.stopPropagation();
    event.dataTransfer.dropEffect = "move";
  }, []);

  const handleEdgeDrop = useCallback(
    (edge: PaneDropEdge, event: DragEvent) => {
      event.preventDefault();
      event.stopPropagation();
      setSurfaceDragActive(false);
      const draggedSurfaceKey = readTabDragSurfaceKey(event.dataTransfer);
      if (!draggedSurfaceKey || !isChatSurface(draggedSurfaceKey)) return;
      if (draggedSurfaceKey === surfaceKey) return;
      dispatch(
        splitPaneWithSurface({
          tabId,
          leafId: leaf.id,
          surfaceKey: draggedSurfaceKey,
          dir: paneDropDirections[edge],
          placement: paneDropPlacements[edge],
        }),
      );
    },
    [dispatch, leaf.id, surfaceKey, tabId],
  );

  return (
    <section
      className={classNames(styles.pane, focused && styles.focused)}
      aria-label={`Workspace pane ${leaf.id}`}
      data-focused={focused || undefined}
      data-workspace-leaf-id={leaf.id}
      onMouseDownCapture={handleFocusPane}
      onPointerDownCapture={handleFocusPane}
      onClick={handleFocusPane}
      onFocusCapture={handleFocusPane}
      onDragEnter={handlePaneDragEnter}
      onDragOver={handlePaneDragOver}
      onDragLeave={handlePaneDragLeave}
      onDragEnd={handlePaneDragEnd}
      onDrop={handlePaneDrop}
    >
      <PaneHeader leaf={leaf} tabId={tabId} />
      {surfaceDragActive ? (
        <div
          className={classNames(styles.edgeDropZones, "rf-enter")}
          aria-hidden="true"
        >
          {paneDropEdges.map((edge) => (
            <div
              key={edge}
              className={classNames(
                styles.edgeDropZone,
                paneDropEdgeClasses[edge],
              )}
              data-testid={`workspace-pane-edge-drop-${leaf.id}-${edge}`}
              onDragOver={handleEdgeDragOver}
              onDrop={(event) => handleEdgeDrop(edge, event)}
            />
          ))}
        </div>
      ) : null}
      <div className={styles.paneBody}>
        <SurfacePane surfaceKey={surfaceKey} />
      </div>
    </section>
  );
}

function PaneNodeView({
  node,
  tabId,
  focusedLeafId,
  stacked,
}: {
  node: PaneNode;
  tabId: SurfaceKey;
  focusedLeafId: string;
  stacked: boolean;
}) {
  if (node.kind === "leaf") {
    return (
      <LeafView leaf={node} tabId={tabId} focused={focusedLeafId === node.id} />
    );
  }

  return (
    <SplitView
      node={node}
      tabId={tabId}
      focusedLeafId={focusedLeafId}
      stacked={stacked}
    />
  );
}

function SplitView({ node, tabId, focusedLeafId, stacked }: SplitViewProps) {
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

        dispatch(
          resizeGroupSplit({ tabId, splitId: node.id, sizes: nextSizes }),
        );
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
    [dispatch, node.dir, node.id, sizes, tabId],
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
          className={classNames(styles.paneSlot, "rf-grow-in")}
          data-pane-index={index}
          style={{ "--pane-flex": sizes[index] } as PaneSlotStyle}
        >
          <PaneNodeView
            node={child}
            tabId={tabId}
            focusedLeafId={focusedLeafId}
            stacked={stacked}
          />
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

export function GroupSplitView({ tabId }: { tabId: SurfaceKey }) {
  const containerRef = useRef<HTMLDivElement>(null);
  const group = useAppSelector((state) => selectGroupForTab(state, tabId));
  const breakpoint = useDashboardLayout(containerRef);
  const stacked = breakpoint !== "wide";

  if (!group) return null;

  return (
    <div
      ref={containerRef}
      className={classNames(styles.layout, stacked && styles.stackedLayout)}
      data-breakpoint={breakpoint}
      data-stacked={stacked || undefined}
      data-workspace-group-tab-id={tabId}
    >
      <div className={styles.tree}>
        <PaneNodeView
          node={group.root}
          tabId={tabId}
          focusedLeafId={group.focusedLeafId}
          stacked={stacked}
        />
      </div>
    </div>
  );
}
