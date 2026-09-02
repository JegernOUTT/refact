import { useCallback, useEffect } from "react";

import { Switch, Tooltip } from "../../../components/ui";
import { useAppDispatch, useAppSelector } from "../../../hooks";
import {
  selectFocusedWorkspaceChatId,
  selectLiveEditsForChat,
  setLiveEditsForChat,
} from "../workspaceSlice";
import { FileTree } from "./FileTree";
import {
  hydrateShowIgnored,
  loadPersistedShowIgnored,
  selectShowIgnored,
  updateShowIgnored,
} from "./filesPanelSlice";
import styles from "./FilesPanel.module.css";

export function FilesPanel() {
  const dispatch = useAppDispatch();
  const showIgnored = useAppSelector(selectShowIgnored);
  const focusedChatId = useAppSelector(selectFocusedWorkspaceChatId);
  const liveEdits = useAppSelector((state) =>
    focusedChatId ? selectLiveEditsForChat(state, focusedChatId) : false,
  );
  const projectRoots = useAppSelector(
    (state) => state.current_project.workspaceRoots,
  );

  useEffect(() => {
    dispatch(hydrateShowIgnored(loadPersistedShowIgnored()));
  }, [dispatch, projectRoots]);

  const onLiveEditsChange = useCallback(
    (enabled: boolean) => {
      if (!focusedChatId) return;
      dispatch(setLiveEditsForChat({ chatId: focusedChatId, enabled }));
    },
    [dispatch, focusedChatId],
  );

  return (
    <div className={styles.panel} data-testid="files-panel">
      <aside className={styles.explorer} aria-label="File explorer">
        <div className={styles.explorerHeader}>
          <span>Explorer</span>
          <div className={styles.headerToggles}>
            <Tooltip>
              <Tooltip.Trigger asChild>
                <Switch
                  checked={liveEdits}
                  className={styles.liveEditsToggle}
                  disabled={focusedChatId === null}
                  label="Live edits"
                  onCheckedChange={onLiveEditsChange}
                />
              </Tooltip.Trigger>
              <Tooltip.Content side="bottom">
                {focusedChatId === null
                  ? "Open a chat to enable"
                  : "Stream agent edits into the viewer"}
              </Tooltip.Content>
            </Tooltip>
            <Switch
              checked={showIgnored}
              className={styles.showIgnoredToggle}
              label="Show ignored"
              onCheckedChange={(checked) =>
                dispatch(updateShowIgnored(checked))
              }
            />
          </div>
        </div>
        <FileTree />
      </aside>
    </div>
  );
}
