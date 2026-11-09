import classNames from "classnames";
import {
  type CSSProperties,
  type DragEvent,
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";

import { useAppDispatch, useAppSelector } from "../../hooks";
import { useDashboardLayout } from "../Dashboard/hooks/useDashboardLayout";
import {
  collectLeafIds,
  type LeafPane,
  type PaneNode,
  type SplitNode,
} from "../ChatPanes/panesTree";
import {
  addSurfaceToPane,
  focusPane,
  MAX_GROUP_LEAVES,
  resizeGroupSplit,
  selectGroupForTab,
  splitPaneWithSurface,
} from "./workspaceSlice";
import {
  isChatSurface,
  isSplittableSurface,
  type SurfaceKey,
} from "./surfaceKey";
import {
  hasSplittableTabDrag,
  readTabDragSurfaceKey,
  surfaceKeyFromTabDragPayload,
  type TabDragPayload,
} from "../ChatPanes/tabDrag";
import {
  usePointerDragHost,
  usePointerDropZone,
} from "../ChatPanes/usePointerDrag";
import { pointerDragController, type Point } from "../ChatPanes/pointerDrag";
import { resolvePaneDropTarget, type PaneDropTarget } from "./paneDropTarget";
import { SurfacePane } from "./SurfacePane";
import { panePanelId } from "./paneTabIds";
import { useTabDisplay } from "./tabDisplay";
import {
  PaneHeaderActions,
  PaneHeaderProvider,
  PaneSurfaceActivityProvider,
} from "./PaneHeaderContext";
import styles from "./GroupSplitView.module.css";

const MIN_RESIZE_FRACTION = 0.12;

type PaneDropEdge = Exclude<PaneDropTarget, "center">;

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

type PaneSlotStyle = CSSProperties & {
  "--pane-flex": number;
};

type DividerProps = {
  dir: SplitNode["dir"];
  dragging: boolean;
  value: number;
  onPointerDown: (event: ReactPointerEvent<HTMLDivElement>) => void;
  onPointerMove: (event: ReactPointerEvent<HTMLDivElement>) => void;
  onPointerEnd: (event: ReactPointerEvent<HTMLDivElement>) => void;
  onKeyDown: (event: ReactKeyboardEvent<HTMLDivElement>) => void;
};

type SplitViewProps = {
  node: SplitNode;
  tabId: SurfaceKey;
  focusedLeafId: string;
  stacked: boolean;
  enteringLeaves: ReadonlySet<string>;
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
  if (!Number.isFinite(pointerFraction)) return sizes;

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

function PaneDivider({
  dir,
  dragging,
  value,
  onPointerDown,
  onPointerMove,
  onPointerEnd,
  onKeyDown,
}: DividerProps) {
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
      aria-label="Resize workspace panes"
      aria-valuenow={Math.round(value * 100)}
      aria-valuemin={0}
      aria-valuemax={100}
      tabIndex={0}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerEnd}
      onPointerCancel={onPointerEnd}
      onLostPointerCapture={onPointerEnd}
      onKeyDown={onKeyDown}
    >
      <div className={styles.dividerHandle} />
    </div>
  );
}

function LeafView({
  leaf,
  tabId,
  focused,
  entering,
}: {
  leaf: LeafPane;
  tabId: SurfaceKey;
  focused: boolean;
  entering: boolean;
}) {
  const dispatch = useAppDispatch();
  const pointerDragEnabled = usePointerDragHost();
  const resolveDisplay = useTabDisplay();
  const canSplit = useAppSelector((state) => {
    const group = selectGroupForTab(state, tabId);
    return Boolean(
      group && collectLeafIds(group.root).length < MAX_GROUP_LEAVES,
    );
  });
  const [nativeTarget, setNativeTarget] = useState<PaneDropTarget | null>(null);
  const paneRef = useRef<HTMLElement | null>(null);
  const nativeSource = useRef<SurfaceKey | null>(null);
  const pointerSource = useRef<SurfaceKey | null>(null);
  const pointerPoint = useRef<Point>({ x: 0, y: 0 });
  const pointerSnapshot = useSyncExternalStore(
    pointerDragController.subscribe,
    pointerDragController.getSnapshot,
    pointerDragController.getSnapshot,
  );
  if (pointerSnapshot.active) pointerPoint.current = pointerSnapshot.point;
  const surfaceKey =
    leaf.activeTabId ?? (leaf.tabIds.length > 0 ? leaf.tabIds[0] : null);

  const visited = useRef(new Set<SurfaceKey>());
  if (surfaceKey) visited.current.add(surfaceKey);
  for (const key of visited.current) {
    if (!leaf.tabIds.includes(key)) visited.current.delete(key);
  }

  const handleFocusPane = useCallback(() => {
    dispatch(focusPane({ tabId, leafId: leaf.id }));
  }, [dispatch, leaf.id, tabId]);

  const targetAt = useCallback(
    (point: Point, sourceKey?: SurfaceKey | null) => {
      const pane = paneRef.current;
      if (!pane) return null;
      const controls = pane.querySelector<HTMLElement>("[data-pane-controls]");
      const controlRect = controls?.getBoundingClientRect();
      if (
        controls &&
        getComputedStyle(controls).opacity !== "0" &&
        controlRect &&
        controlRect.width > 0 &&
        controlRect.height > 0 &&
        point.x >= controlRect.left &&
        point.x <= controlRect.right &&
        point.y >= controlRect.top &&
        point.y <= controlRect.bottom
      ) {
        return null;
      }
      const rect = pane.getBoundingClientRect();
      const target = resolvePaneDropTarget(rect, point);
      if (target === "center" && sourceKey === surfaceKey) return null;
      return target === "center" || canSplit ? target : null;
    },
    [canSplit, surfaceKey],
  );

  const acceptsSurface = useCallback(
    (key: SurfaceKey | null): key is SurfaceKey =>
      key !== null &&
      isSplittableSurface(key) &&
      (key !== surfaceKey || leaf.tabIds.length > 1),
    [surfaceKey, leaf.tabIds.length],
  );

  const performDrop = useCallback(
    (key: SurfaceKey, target: PaneDropTarget | null) => {
      if (!target || !acceptsSurface(key)) return;
      if (target === "center") {
        dispatch(addSurfaceToPane({ tabId, leafId: leaf.id, surfaceKey: key }));
      } else {
        dispatch(
          splitPaneWithSurface({
            tabId,
            leafId: leaf.id,
            surfaceKey: key,
            dir: paneDropDirections[target],
            placement: paneDropPlacements[target],
          }),
        );
      }
    },
    [acceptsSurface, dispatch, leaf.id, tabId],
  );

  useEffect(() => {
    const clear = () => {
      nativeSource.current = null;
      setNativeTarget(null);
      document.body.removeAttribute("data-rf-native-tab-drag");
    };
    const start = (event: globalThis.DragEvent) => {
      clear();
      const source =
        event.target instanceof Element
          ? event.target.closest("[data-pane-tab-key], [data-tab-surface-key]")
          : null;
      nativeSource.current =
        source?.getAttribute("data-pane-tab-key") ??
        source?.getAttribute("data-tab-surface-key") ??
        null;
      if (nativeSource.current && isSplittableSurface(nativeSource.current)) {
        document.body.setAttribute("data-rf-native-tab-drag", "true");
      }
    };
    const rememberPoint = (event: PointerEvent) => {
      pointerPoint.current = { x: event.clientX, y: event.clientY };
    };
    const keydown = (event: KeyboardEvent) => {
      if (event.key === "Escape") clear();
    };
    window.addEventListener("dragstart", start, true);
    window.addEventListener("dragend", clear, true);
    window.addEventListener("drop", clear, true);
    window.addEventListener("blur", clear);
    window.addEventListener("keydown", keydown, true);
    window.addEventListener("pointerup", rememberPoint, true);
    return () => {
      window.removeEventListener("dragstart", start, true);
      window.removeEventListener("dragend", clear, true);
      window.removeEventListener("drop", clear, true);
      window.removeEventListener("blur", clear);
      window.removeEventListener("keydown", keydown, true);
      window.removeEventListener("pointerup", rememberPoint, true);
      document.body.removeAttribute("data-rf-native-tab-drag");
    };
  }, []);

  const handlePaneDragOver = useCallback(
    (event: DragEvent<HTMLElement>) => {
      const key =
        readTabDragSurfaceKey(event.dataTransfer) ?? nativeSource.current;
      if (
        !(
          hasSplittableTabDrag(event.dataTransfer) ||
          Array.from(event.dataTransfer.types).includes(
            "application/x-refact-surface-tab",
          )
        ) ||
        !acceptsSurface(key)
      ) {
        setNativeTarget(null);
        return;
      }
      const target = targetAt({ x: event.clientX, y: event.clientY }, key);
      setNativeTarget(target);
      if (target) {
        event.preventDefault();
        event.dataTransfer.dropEffect = "move";
      }
    },
    [acceptsSurface, targetAt],
  );

  const handlePaneDragLeave = useCallback((event: DragEvent<HTMLElement>) => {
    if (
      event.relatedTarget instanceof Node &&
      event.currentTarget.contains(event.relatedTarget)
    )
      return;
    setNativeTarget(null);
  }, []);

  const handlePaneDrop = useCallback(
    (event: DragEvent) => {
      setNativeTarget(null);
      const key = readTabDragSurfaceKey(event.dataTransfer);
      if (!acceptsSurface(key)) return;
      event.preventDefault();
      event.stopPropagation();
      performDrop(key, targetAt({ x: event.clientX, y: event.clientY }, key));
    },
    [acceptsSurface, performDrop, targetAt],
  );

  const paneAccepts = useCallback(
    (payload: TabDragPayload) => {
      const draggedSurfaceKey = surfaceKeyFromTabDragPayload(payload);
      pointerSource.current = draggedSurfaceKey;
      return acceptsSurface(draggedSurfaceKey);
    },
    [acceptsSurface],
  );

  const panePointerDrop = usePointerDropZone({
    enabled: pointerDragEnabled,
    accepts: paneAccepts,
    onDrop: useCallback(
      (payload: TabDragPayload) => {
        const draggedSurfaceKey = surfaceKeyFromTabDragPayload(payload);
        if (draggedSurfaceKey && paneAccepts(payload)) {
          performDrop(
            draggedSurfaceKey,
            targetAt(pointerPoint.current, draggedSurfaceKey),
          );
        }
        setNativeTarget(null);
      },
      [paneAccepts, performDrop, targetAt],
    ),
  });

  const previewTarget = pointerSnapshot.active
    ? panePointerDrop.isOver
      ? targetAt(pointerSnapshot.point, pointerSource.current)
      : null
    : nativeTarget;
  const { ref: pointerDropRef } = panePointerDrop;
  const setPaneRef = useCallback(
    (node: HTMLElement | null) => {
      paneRef.current = node;
      pointerDropRef(node);
    },
    [pointerDropRef],
  );

  return (
    <PaneHeaderProvider>
      <section
        ref={setPaneRef}
        className={classNames(
          styles.pane,
          surfaceKey && isChatSurface(surfaceKey) && styles.chatPane,
          entering && styles.paneEntering,
        )}
        aria-label={`Workspace pane ${leaf.id}`}
        data-focused={focused || undefined}
        data-pane-entering={entering || undefined}
        data-workspace-leaf-id={leaf.id}
        onMouseDownCapture={(event) => {
          if (event.button === 0) handleFocusPane();
        }}
        onPointerDownCapture={(event) => {
          if (event.button === 0) handleFocusPane();
        }}
        onClick={handleFocusPane}
        onFocusCapture={handleFocusPane}
        onDragOverCapture={handlePaneDragOver}
        onDragLeave={handlePaneDragLeave}
        onDrop={handlePaneDrop}
      >
        <div data-pane-tab-header={leaf.id} className={styles.tabHeader} />
        <div
          className={styles.paneControls}
          data-pane-controls
          aria-label="Workspace pane controls"
        >
          <PaneHeaderActions className={styles.fileActions} />
        </div>
        {previewTarget ? (
          <div className={styles.previewBody} aria-hidden="true">
            <div
              className={styles.dropPreview}
              data-testid="workspace-pane-drop-preview"
              data-drop-target={previewTarget}
            >
              {previewTarget === "center" ? (
                <span className={styles.joinHint}>Join tabs</span>
              ) : null}
            </div>
          </div>
        ) : null}
        {leaf.tabIds.map((key) => (
          <div
            key={key}
            className={styles.paneBody}
            hidden={key !== surfaceKey}
            role="tabpanel"
            id={panePanelId(leaf.id, key)}
            aria-label={resolveDisplay(key).title}
          >
            {visited.current.has(key) ? (
              <PaneSurfaceActivityProvider
                active={key === surfaceKey}
                focused={focused && key === surfaceKey}
              >
                <SurfacePane
                  surfaceKey={key}
                  focused={focused && key === surfaceKey}
                />
              </PaneSurfaceActivityProvider>
            ) : null}
          </div>
        ))}
        {leaf.tabIds.length === 0 ? (
          <div className={styles.paneBody}>
            <SurfacePane surfaceKey={null} />
          </div>
        ) : null}
      </section>
    </PaneHeaderProvider>
  );
}

function PaneNodeView({
  node,
  tabId,
  focusedLeafId,
  stacked,
  enteringLeaves,
}: {
  node: PaneNode;
  tabId: SurfaceKey;
  focusedLeafId: string;
  stacked: boolean;
  enteringLeaves: ReadonlySet<string>;
}) {
  if (node.kind === "leaf") {
    return (
      <LeafView
        leaf={node}
        tabId={tabId}
        focused={focusedLeafId === node.id}
        entering={enteringLeaves.has(node.id)}
      />
    );
  }

  return (
    <SplitView
      node={node}
      tabId={tabId}
      focusedLeafId={focusedLeafId}
      stacked={stacked}
      enteringLeaves={enteringLeaves}
    />
  );
}

function SplitView({
  node,
  tabId,
  focusedLeafId,
  stacked,
  enteringLeaves,
}: SplitViewProps) {
  const dispatch = useAppDispatch();
  const containerRef = useRef<HTMLDivElement>(null);
  const [draggingDivider, setDraggingDivider] = useState<number | null>(null);
  const sizes = normalizedSizes(node.sizes, node.children.length);

  const resizeSession = useRef<{
    pointerId: number;
    dividerIndex: number;
    element: HTMLDivElement;
    sizes: number[];
  } | null>(null);

  const endResize = useCallback(() => {
    const session = resizeSession.current;
    resizeSession.current = null;
    setDraggingDivider(null);
    if (session?.element.hasPointerCapture(session.pointerId)) {
      session.element.releasePointerCapture(session.pointerId);
    }
  }, []);

  useEffect(() => {
    window.addEventListener("blur", endResize);
    return () => {
      window.removeEventListener("blur", endResize);
      endResize();
    };
  }, [endResize, node.dir, stacked]);

  const handleDividerPointerDown = (
    dividerIndex: number,
    event: ReactPointerEvent<HTMLDivElement>,
  ) => {
    if (event.button !== 0 || resizeSession.current) return;
    event.preventDefault();
    event.stopPropagation();
    event.currentTarget.focus();
    event.currentTarget.setPointerCapture(event.pointerId);
    resizeSession.current = {
      pointerId: event.pointerId,
      dividerIndex,
      element: event.currentTarget,
      sizes,
    };
    setDraggingDivider(dividerIndex);
  };

  const handleDividerPointerMove = (
    event: ReactPointerEvent<HTMLDivElement>,
  ) => {
    const session = resizeSession.current;
    const container = containerRef.current;
    if (!session || session.pointerId !== event.pointerId || !container) return;
    const rect = container.getBoundingClientRect();
    const length = node.dir === "row" ? rect.width : rect.height;
    if (!Number.isFinite(length) || length <= 0) return;
    const start = node.dir === "row" ? rect.left : rect.top;
    const pointer = node.dir === "row" ? event.clientX : event.clientY;
    dispatch(
      resizeGroupSplit({
        tabId,
        splitId: node.id,
        sizes: resizeAtDivider(
          session.sizes,
          session.dividerIndex,
          (pointer - start) / length,
        ),
      }),
    );
  };

  const handleDividerPointerEnd = (
    event: ReactPointerEvent<HTMLDivElement>,
  ) => {
    if (resizeSession.current?.pointerId === event.pointerId) endResize();
  };

  const handleDividerKeyDown = (
    dividerIndex: number,
    event: ReactKeyboardEvent<HTMLDivElement>,
  ) => {
    const decrease = node.dir === "row" ? "ArrowLeft" : "ArrowUp";
    const increase = node.dir === "row" ? "ArrowRight" : "ArrowDown";
    const boundary = sizes
      .slice(0, dividerIndex + 1)
      .reduce((sum, size) => sum + size, 0);
    let next: number;
    if (event.key === decrease) next = boundary - 0.02;
    else if (event.key === increase) next = boundary + 0.02;
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = 1;
    else if (event.key === "Escape" && resizeSession.current) {
      event.preventDefault();
      endResize();
      return;
    } else return;
    event.preventDefault();
    dispatch(
      resizeGroupSplit({
        tabId,
        splitId: node.id,
        sizes: resizeAtDivider(sizes, dividerIndex, next),
      }),
    );
  };

  return (
    <div
      ref={containerRef}
      className={classNames(
        styles.split,
        node.dir === "row" ? styles.row : styles.col,
        stacked && styles.stackedSplit,
      )}
      data-resizing={draggingDivider !== null || undefined}
      data-pane-split-id={node.id}
      data-pane-split-dir={node.dir}
    >
      {node.children.map((child, index) => (
        <div
          key={child.id}
          className={styles.paneSlot}
          data-pane-index={index}
          style={{ "--pane-flex": sizes[index] } as PaneSlotStyle}
        >
          <PaneNodeView
            node={child}
            tabId={tabId}
            focusedLeafId={focusedLeafId}
            stacked={stacked}
            enteringLeaves={enteringLeaves}
          />
          {!stacked && index < node.children.length - 1 ? (
            <PaneDivider
              dir={node.dir}
              dragging={draggingDivider === index}
              value={sizes
                .slice(0, index + 1)
                .reduce((sum, size) => sum + size, 0)}
              onPointerDown={(event) => handleDividerPointerDown(index, event)}
              onPointerMove={handleDividerPointerMove}
              onPointerEnd={handleDividerPointerEnd}
              onKeyDown={(event) => handleDividerKeyDown(index, event)}
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
  const leafSignature = JSON.stringify(group ? collectLeafIds(group.root) : []);
  const previousLeaves = useRef<Set<string>>();
  const enteringLeaves = useMemo(() => {
    const ids = JSON.parse(leafSignature) as string[];
    return new Set(
      previousLeaves.current
        ? ids.filter((id) => !previousLeaves.current?.has(id))
        : [],
    );
  }, [leafSignature]);
  useLayoutEffect(() => {
    previousLeaves.current = new Set(JSON.parse(leafSignature) as string[]);
  }, [leafSignature]);

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
          enteringLeaves={enteringLeaves}
        />
      </div>
    </div>
  );
}
