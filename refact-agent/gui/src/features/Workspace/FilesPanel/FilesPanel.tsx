import {
  ArrowUp,
  Check,
  FileInput,
  FolderOpen,
  MoreHorizontal,
  Undo2,
} from "lucide-react";
import { type FormEvent, useCallback, useEffect, useId, useState } from "react";

import {
  FieldText,
  Icon,
  IconButton,
  Menu,
  Tooltip,
} from "../../../components/ui";
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
          <Menu modal={false}>
            <Tooltip content="More options">
              <Menu.Trigger asChild>
                <IconButton
                  aria-label="More options"
                  icon={MoreHorizontal}
                  size="sm"
                  variant="ghost"
                />
              </Menu.Trigger>
            </Tooltip>
            <Menu.Content align="end">
              <Menu.Item
                role="menuitemcheckbox"
                aria-checked={liveEdits}
                aria-describedby={
                  focusedChatId === null ? `${inputId}-live-edits` : undefined
                }
                disabled={focusedChatId === null}
                onSelect={() => onLiveEditsChange(!liveEdits)}
              >
                <span className={styles.menuCheck}>
                  {liveEdits ? <Icon icon={Check} size="sm" /> : null}
                </span>
                Live edits
              </Menu.Item>
              {focusedChatId === null ? (
                <Menu.Label id={`${inputId}-live-edits`}>
                  Open a chat to enable
                </Menu.Label>
              ) : null}
              <Menu.Item
                role="menuitemcheckbox"
                aria-checked={showIgnored}
                onSelect={() => dispatch(updateShowIgnored(!showIgnored))}
              >
                <span className={styles.menuCheck}>
                  {showIgnored ? <Icon icon={Check} size="sm" /> : null}
                </span>
                Show ignored
              </Menu.Item>
            </Menu.Content>
          </Menu>
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
            <Tooltip content="Open file">
              <IconButton
                aria-label="Open"
                icon={FileInput}
                size="sm"
                variant="ghost"
                type="submit"
              />
            </Tooltip>
            <Tooltip content="Browse folder">
              <IconButton
                aria-label="Browse folder"
                icon={FolderOpen}
                size="sm"
                variant="ghost"
                type="button"
                onClick={onBrowsePath}
              />
            </Tooltip>
          </div>
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
              <Tooltip content="Parent folder">
                <span className={styles.actionHost}>
                  <IconButton
                    aria-label="Parent folder"
                    icon={ArrowUp}
                    size="sm"
                    variant="ghost"
                    disabled={!browseParent || browseParent === browsePath}
                    onClick={() =>
                      setBrowse({
                        contextRoot,
                        path: browseParent ?? browsePath,
                      })
                    }
                  />
                </span>
              </Tooltip>
              <Tooltip content="Current worktree">
                <IconButton
                  aria-label="Current worktree"
                  icon={Undo2}
                  size="sm"
                  variant="ghost"
                  onClick={() => setBrowse(null)}
                />
              </Tooltip>
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
