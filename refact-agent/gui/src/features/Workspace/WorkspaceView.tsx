import classNames from "classnames";
import { Columns3 } from "lucide-react";
import { useCallback, useEffect } from "react";

import { IconButton, Tooltip } from "../../components/ui";
import { useAppDispatch, useAppSelector } from "../../hooks";
import { selectCurrentThreadId } from "../Chat/Thread";
import { collectTabIds } from "../ChatPanes/panesTree";
import { GroupSplitView } from "./GroupSplitView";
import { SurfacePane } from "./SurfacePane";
import { makeSurfaceKey } from "./surfaceKey";
import {
  openTab,
  selectActiveTabId,
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
  const isSplit = useAppSelector((state) =>
    activeTabId ? selectIsTabSplit(state, activeTabId) : false,
  );

  useEffect(() => {
    if (
      currentSurfaceKey === null ||
      activeTabId !== null ||
      currentSurfaceKnown
    )
      return;
    dispatch(openTab(currentSurfaceKey));
  }, [activeTabId, currentSurfaceKey, currentSurfaceKnown, dispatch]);

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
