import classNames from "classnames";
import { Columns3 } from "lucide-react";
import { useCallback, useEffect } from "react";

import { IconButton, Tooltip } from "../../components/ui";
import { useAppDispatch, useAppSelector } from "../../hooks";
import {
  selectCurrentThreadId,
  selectOpenThreadIds,
  switchToThread,
} from "../Chat/Thread";
import { collectTabIds } from "../ChatPanes/panesTree";
import { GroupSplitView } from "./GroupSplitView";
import { SurfacePane } from "./SurfacePane";
import { makeSurfaceKey } from "./surfaceKey";
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
    if (!activeTabId) return;
    dispatch(splitTab({ tabId: activeTabId, dir: "row" }));
  }, [activeTabId, dispatch]);

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
        {activeTabId && isSplit ? (
          <GroupSplitView tabId={activeTabId} />
        ) : (
          <div className={styles.unsplitSurfaceWrap}>
            <SurfacePane surfaceKey={activeTabId} />
            {activeTabId ? (
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
