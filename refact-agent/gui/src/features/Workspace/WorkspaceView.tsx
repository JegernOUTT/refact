import classNames from "classnames";
import { type DragEvent, useCallback, useEffect, useState } from "react";

import { useAppDispatch, useAppSelector } from "../../hooks";
import {
  selectCurrentThreadId,
  selectOpenThreadIds,
  switchToThread,
} from "../Chat/Thread";
import { collectTabIds } from "../ChatPanes/panesTree";
import {
  hasSplittableTabDrag,
  readTabDragSurfaceKey,
  surfaceKeyFromTabDragPayload,
} from "../ChatPanes/tabDrag";
import {
  usePointerDragHost,
  usePointerDropZone,
} from "../ChatPanes/usePointerDrag";
import { GroupSplitView } from "./GroupSplitView";
import { SurfacePane } from "./SurfacePane";
import { isSplittableSurface, makeSurfaceKey } from "./surfaceKey";
import {
  consolidateWorkspaceTabs,
  openTab,
  selectActiveTabId,
  selectWorkspaceFocusedChatId,
  selectIsTabSplit,
  selectTabs,
  selectWorkspaceGroups,
  splitTab,
} from "./workspaceSlice";
import styles from "./WorkspaceView.module.css";

export function WorkspaceView() {
  const dispatch = useAppDispatch();
  const [unsplitDragActive, setUnsplitDragActive] = useState(false);
  const activeTabId = useAppSelector(selectActiveTabId);
  const currentThreadId = useAppSelector(selectCurrentThreadId);
  const openThreadIds = useAppSelector(selectOpenThreadIds);
  const tabs = useAppSelector(selectTabs);
  const groups = useAppSelector(selectWorkspaceGroups);
  const currentSurfaceKey = currentThreadId
    ? makeSurfaceKey("chat", currentThreadId)
    : null;
  const currentSurfaceKnown = currentSurfaceKey
    ? tabs.includes(currentSurfaceKey) ||
      Object.values(groups).some((group) =>
        group ? collectTabIds(group.root).includes(currentSurfaceKey) : false,
      )
    : false;
  const currentThreadIsOpen = currentThreadId
    ? openThreadIds.includes(currentThreadId)
    : false;
  const focusedChatId = useAppSelector(selectWorkspaceFocusedChatId);
  const focusedChatIsOpen = focusedChatId
    ? openThreadIds.includes(focusedChatId)
    : false;
  const isSplit = useAppSelector((state) =>
    activeTabId ? selectIsTabSplit(state, activeTabId) : false,
  );
  const activeTabCanSplit = activeTabId
    ? isSplittableSurface(activeTabId)
    : false;

  useEffect(() => {
    const workspaceTabs = tabs.filter(isSplittableSurface);
    if (
      workspaceTabs.length > 1 ||
      (workspaceTabs[0] && !groups[workspaceTabs[0]])
    )
      dispatch(consolidateWorkspaceTabs());
  }, [dispatch, groups, tabs]);

  useEffect(() => {
    if (
      currentSurfaceKey === null ||
      activeTabId !== null ||
      currentSurfaceKnown ||
      !currentThreadIsOpen
    )
      return;
    dispatch(openTab(currentSurfaceKey));
  }, [
    activeTabId,
    currentSurfaceKey,
    currentSurfaceKnown,
    currentThreadIsOpen,
    dispatch,
  ]);

  useEffect(() => {
    if (!focusedChatId || !focusedChatIsOpen) return;
    if (currentThreadId === focusedChatId) return;
    dispatch(switchToThread({ id: focusedChatId, openTab: false }));
  }, [focusedChatId, focusedChatIsOpen, currentThreadId, dispatch]);

  const handleUnsplitDragEnter = useCallback(
    (event: DragEvent) => {
      if (!activeTabCanSplit || !hasSplittableTabDrag(event.dataTransfer)) {
        return;
      }
      setUnsplitDragActive(true);
    },
    [activeTabCanSplit],
  );

  const handleUnsplitDragOver = useCallback(
    (event: DragEvent) => {
      if (!activeTabCanSplit || !hasSplittableTabDrag(event.dataTransfer)) {
        return;
      }
      event.preventDefault();
      event.dataTransfer.dropEffect = "move";
      setUnsplitDragActive(true);
    },
    [activeTabCanSplit],
  );

  const handleUnsplitDragLeave = useCallback(
    (event: DragEvent<HTMLElement>) => {
      const nextTarget = event.relatedTarget;
      if (
        nextTarget instanceof Node &&
        event.currentTarget.contains(nextTarget)
      ) {
        return;
      }
      setUnsplitDragActive(false);
    },
    [],
  );

  const handleUnsplitDrop = useCallback(
    (event: DragEvent) => {
      const draggedSurfaceKey = readTabDragSurfaceKey(event.dataTransfer);
      if (
        !activeTabId ||
        !activeTabCanSplit ||
        !draggedSurfaceKey ||
        !isSplittableSurface(draggedSurfaceKey) ||
        draggedSurfaceKey === activeTabId
      ) {
        setUnsplitDragActive(false);
        return;
      }

      event.preventDefault();
      setUnsplitDragActive(false);
      dispatch(
        splitTab({
          tabId: activeTabId,
          dir: "row",
          surfaceKey: draggedSurfaceKey,
        }),
      );
    },
    [activeTabCanSplit, activeTabId, dispatch],
  );

  const handleUnsplitDragEnd = useCallback(() => {
    setUnsplitDragActive(false);
  }, []);

  const pointerDragEnabled = usePointerDragHost();
  const unsplitPointerDrop = usePointerDropZone({
    enabled: pointerDragEnabled && activeTabCanSplit && Boolean(activeTabId),
    accepts: useCallback(
      (payload) => {
        const draggedSurfaceKey = surfaceKeyFromTabDragPayload(payload);
        return (
          Boolean(activeTabId) &&
          draggedSurfaceKey !== null &&
          isSplittableSurface(draggedSurfaceKey) &&
          draggedSurfaceKey !== activeTabId
        );
      },
      [activeTabId],
    ),
    onDrop: useCallback(
      (payload) => {
        const draggedSurfaceKey = surfaceKeyFromTabDragPayload(payload);
        if (!activeTabId || !draggedSurfaceKey) return;
        dispatch(
          splitTab({
            tabId: activeTabId,
            dir: "row",
            surfaceKey: draggedSurfaceKey,
          }),
        );
      },
      [activeTabId, dispatch],
    ),
  });

  const unsplitDropActive = unsplitDragActive || unsplitPointerDrop.isOver;

  return (
    <div className={styles.workspaceView}>
      <div className={styles.mainColumn}>
        <div
          className={classNames(
            styles.body,
            !isSplit && styles.unsplitBody,
            isSplit && styles.splitBody,
          )}
        >
          {activeTabId && activeTabCanSplit && isSplit ? (
            <GroupSplitView tabId={activeTabId} />
          ) : (
            <div
              ref={unsplitPointerDrop.ref}
              className={classNames(
                styles.unsplitSurfaceWrap,
                unsplitDropActive && styles.unsplitSurfaceDragActive,
              )}
              onDragEnter={handleUnsplitDragEnter}
              onDragOver={handleUnsplitDragOver}
              onDragLeave={handleUnsplitDragLeave}
              onDragEnd={handleUnsplitDragEnd}
              onDrop={handleUnsplitDrop}
              data-workspace-unsplit-drop-target={
                activeTabCanSplit ? "true" : undefined
              }
            >
              <SurfacePane surfaceKey={activeTabId} />
              {unsplitDropActive ? (
                <div
                  className={classNames(styles.unsplitDropOverlay, "rf-enter")}
                  aria-hidden="true"
                >
                  <div className={styles.unsplitDropHint}>Drop to split</div>
                </div>
              ) : null}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
