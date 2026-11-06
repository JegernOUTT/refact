import { FolderOpen } from "lucide-react";
import { type FormEvent, useCallback, useEffect, useId, useState } from "react";

import { Button, FieldText, Switch, Tooltip } from "../../../components/ui";
import { useAppDispatch, useAppSelector } from "../../../hooks";
import { filesApi } from "../../../services/refact/files";
import {
  selectFocusedWorkspaceChatId,
  selectFocusedChatWorkspaceRoot,
  selectLiveEditsForChat,
  setLiveEditsForChat,
} from "../workspaceSlice";
import { parentDirectoryPath } from "./fileTreeModel";
import { FileTree } from "./FileTree";
import {
  hydrateShowIgnored,
  loadPersistedShowIgnored,
  openFileInFilesPanel,
  selectShowIgnored,
  updateShowIgnored,
  validateAbsoluteFilePath,
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

  const contextRoot = useAppSelector(selectFocusedChatWorkspaceRoot);
  const [browse, setBrowse] = useState<{
    contextRoot: string;
    path: string;
  } | null>(null);
  const browsePath =
    browse?.contextRoot === contextRoot ? browse.path : undefined;
  const browseParent = browsePath ? parentDirectoryPath(browsePath) : null;
  const onBrowsePath = () => {
    const result = validateAbsoluteFilePath(manualPath, true);
    if (!result.ok) {
      setManualPathError(result.error);
      return;
    }
    setManualPathError(null);
    setBrowse({ contextRoot, path: result.path });
  };

  const inputId = useId();
  const errorId = useId();
  const [manualPath, setManualPath] = useState("");
  const [manualPathError, setManualPathError] = useState<string | null>(null);

  const onOpenPath = useCallback(
    (event: FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      const result = validateAbsoluteFilePath(manualPath);
      if (!result.ok) {
        setManualPathError(result.error);
        return;
      }
      setManualPathError(null);
      dispatch(openFileInFilesPanel({ path: result.path, manual: true }));
      dispatch(
        filesApi.util.invalidateTags([{ type: "File", id: result.path }]),
      );
    },
    [dispatch, manualPath],
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
        <form
          aria-label="Open file by absolute path"
          className={styles.openPathForm}
          noValidate
          onSubmit={onOpenPath}
        >
          <label className={styles.srOnly} htmlFor={inputId}>
            Absolute file path
          </label>
          <div className={styles.openPathRow}>
            <FieldText
              aria-describedby={manualPathError ? errorId : undefined}
              aria-invalid={manualPathError ? true : undefined}
              autoComplete="off"
              className={styles.openPathInput}
              id={inputId}
              onChange={(value) => {
                setManualPath(value);
                if (manualPathError) setManualPathError(null);
              }}
              placeholder="Open absolute path…"
              spellCheck={false}
              value={manualPath}
            />
            <Button leftIcon={FolderOpen} size="sm" type="submit">
              Open
            </Button>
          </div>
          <Button size="sm" type="button" onClick={onBrowsePath}>
            Browse folder
          </Button>
          {manualPathError ? (
            <p className={styles.openPathError} id={errorId} role="alert">
              {manualPathError}
            </p>
          ) : null}
        </form>
        {browsePath ? (
          <div className={styles.openPathForm}>
            <span className="rf-wrap-anywhere">{browsePath}</span>
            <div className={styles.openPathRow}>
              <Button
                size="sm"
                disabled={!browseParent || browseParent === browsePath}
                onClick={() =>
                  setBrowse({
                    contextRoot,
                    path: browseParent ?? browsePath,
                  })
                }
              >
                Parent folder
              </Button>
              <Button size="sm" onClick={() => setBrowse(null)}>
                Current worktree
              </Button>
            </div>
          </div>
        ) : null}
        <FileTree
          key={`${contextRoot}:${browsePath ?? ""}`}
          browsePath={browsePath}
        />
      </aside>
    </div>
  );
}
