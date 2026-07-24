import { useEffect } from "react";

import { Switch } from "../../../components/ui";
import { useAppDispatch, useAppSelector } from "../../../hooks";
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
  const projectRoots = useAppSelector(
    (state) => state.current_project.workspaceRoots,
  );

  useEffect(() => {
    dispatch(hydrateShowIgnored(loadPersistedShowIgnored()));
  }, [dispatch, projectRoots]);

  return (
    <div className={styles.panel} data-testid="files-panel">
      <aside className={styles.explorer} aria-label="File explorer">
        <div className={styles.explorerHeader}>
          <span>Explorer</span>
          <Switch
            checked={showIgnored}
            className={styles.showIgnoredToggle}
            label="Show ignored"
            onCheckedChange={(checked) => dispatch(updateShowIgnored(checked))}
          />
        </div>
        <FileTree />
      </aside>
    </div>
  );
}
