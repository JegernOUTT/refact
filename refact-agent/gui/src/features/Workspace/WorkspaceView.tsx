import classNames from "classnames";
import { Columns3 } from "lucide-react";
import {
  type DragEvent,
  useCallback,
  useEffect,
  useState,
} from "react";

import { IconButton, Tooltip } from "../../components/ui";
import { useAppDispatch, useAppSelector } from "../../hooks";
import {
  selectCurrentThreadId,
  selectOpenThreadIds,
  switchToThread,
} from "../Chat/Thread";
import { collectTabIds } from "../ChatPanes/panesTree";
import {
  hasTabDragType,
  readTabDragSurfaceKey,
} from "../ChatPanes/tabDrag";
import { GroupSplitView } from "./GroupSplitView";
import { SurfacePane } from "./SurfacePane";
import { isChatSurface, makeSurfaceKey } from "./surfaceKey";
import {
  openTab,
  selectActiveTabId,
  selectFocusedWorkspaceChatId,
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
  const focusedChatId = useAppSelector(selectFocusedWorkspaceChatId);
  const focusedChatIsOpen = focusedChatId
    ? openThreadIds.includes(focusedChatId)
    : false;
  const isSplit = useAppSelector((state) =>
    activeTabId ? selectIsTabSplit(state, activeTabId) : false,
  );
  const activeTabCanSplit = activeTabId ? isChatSurface(activeTabId) : false;

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

  const handleSplitActiveSurface = useCallback(() => {
    if (!activeTabCanSplit || !activeTabId) return;
    dispatch(splitTab({ tabId: activeTabId, dir: "row" }));
  }, [activeTabCanSplit, activeTabId, dispatch]);

  const handleUnsplitDragEnter = useCallback(
    (event: DragEvent) => {
      if (!activeTabCanSplit || !hasTabDragType(event.dataTransfer, "chat")) {
        return;
      }
      setUnsplitDragActive(true);
    },
    [activeTabCanSplit],
  );

  const handleUnsplitDragOver = useCallback(
    (event: DragEvent) => {
      if (!activeTabCanSplit || !hasTabDragType(event.dataTransfer, "chat")) {
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
        !isChatSurface(draggedSurfaceKey) ||
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

  return (
    <div className={styles.workspaceView}>
      <div
        className={classNames(
          styles.body,
          !isSplit && styles.unsplitBody,
          isSplit && styles.splitBody,
          isSplit ? "rf-enter-scale" : "rf-enter",
        )}
      >
        {activeTabId && activeTabCanSplit && isSplit ? (
          <GroupSplitView tabId={activeTabId} />
        ) : (
          <div
            className={classNames(
              styles.unsplitSurfaceWrap,
              unsplitDragActive && styles.unsplitSurfaceDragActive,
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
            {unsplitDragActive ? (
              <div
                className={classNames(styles.unsplitDropOverlay, "rf-enter")}
                aria-hidden="true"
              >
                <div className={styles.unsplitDropHint}>Drop to split</div>
              </div>
            ) : null}
            {activeTabCanSplit ? (
              <div className={styles.unsplitSplitAffordance}>
                <Tooltip content="Split this tab">
                  <IconButton
                    aria-label="Split active tab"
                    className={styles.unsplitSplitButton}
                    icon={Columns3}
                    onClick={handleSplitActiveSurface}
                    size="sm"
                    variant="plain"
                  />
                </Tooltip>
              </div>
            ) : null}
          </div>
        )}
      </div>
    </div>
  );
}
