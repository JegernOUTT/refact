import {
  ArrowUp,
  ArrowDownUp,
  X,
  Check,
  FileInput,
  FolderOpen,
  MoreHorizontal,
  Undo2,
} from "lucide-react";
import { type FormEvent, useCallback, useEffect, useId, useState } from "react";

import {
  Button,
  Dialog,
  FieldText,
  Icon,
  IconButton,
  Menu,
  Tooltip,
} from "../../../components/ui";
import { useAppDispatch, useAppSelector } from "../../../hooks";
import { useIsDocumentVisible } from "../../../hooks/useIsDocumentVisible";
import { getEngineEndpointIdentity } from "../../../services/refact/apiUrl";
import {
  filesApi,
  type FilesSort,
  type FilesDirection,
  type FilesSearchEntry,
} from "../../../services/refact/files";
import {
  selectFocusedWorkspaceChatId,
  selectFocusedChatWorkspaceRoot,
  selectLiveEditsForChat,
  setLiveEditsForChat,
} from "../workspaceSlice";
import { parentDirectoryPath } from "./fileTreeModel";
import { FileTree } from "./FileTree";
import {
  selectTreePath,
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
    project?: boolean;
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
    setPathDialog(null);
    setSearch("");
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
      setPathDialog(null);
      dispatch(openFileInFilesPanel({ path: result.path, manual: true }));
      dispatch(
        filesApi.util.invalidateTags([{ type: "File", id: result.path }]),
      );
    },
    [dispatch, manualPath],
  );

  const [pathDialog, setPathDialog] = useState<"open" | "browse" | null>(null);
  const [search, setSearch] = useState("");
  const [debounced, setDebounced] = useState("");
  const [sort, setSort] = useState<FilesSort>("name");
  const [direction, setDirection] = useState<FilesDirection>("asc");
  const [searchRoot, setSearchRoot] = useState("");
  const [activeResult, setActiveResult] = useState(-1);
  const config = useAppSelector((state) => state.config);
  const projectIdentity = JSON.stringify([
    projectRoots,
    getEngineEndpointIdentity(config),
    config.apiKey,
  ]);
  const [queryIdentity, setQueryIdentity] = useState(projectIdentity);
  useEffect(() => {
    setSearch("");
    setDebounced("");
    setSearchRoot("");
    setBrowse(null);
    setPathDialog(null);
    setQueryIdentity(projectIdentity);
  }, [projectIdentity]);
  useEffect(() => {
    setActiveResult(-1);
    const timer = setTimeout(() => setDebounced(search.trim()), 250);
    return () => clearTimeout(timer);
  }, [search]);
  const searchActive =
    Boolean(debounced) &&
    search.trim() === debounced &&
    queryIdentity === projectIdentity;
  const visible = useIsDocumentVisible(() => {
    if (searchActive) void searchQuery.refetch();
  });
  const searchQuery = filesApi.useSearchFilesQuery(
    {
      q: debounced,
      root: projectRoots?.includes(searchRoot) ? searchRoot : undefined,
      limit: 200,
      sort,
      direction,
      show_ignored: showIgnored,
      projectIdentity,
    },
    {
      skip: !searchActive,
      pollingInterval: visible && searchActive ? 5_000 : 0,
      skipPollingIfUnfocused: true,
      refetchOnFocus: true,
      refetchOnReconnect: true,
      refetchOnMountOrArgChange: true,
    },
  );
  const results =
    search.trim() === debounced && queryIdentity === projectIdentity
      ? searchQuery.currentData
      : undefined;
  const activateResult = (entry: FilesSearchEntry) => {
    dispatch(selectTreePath(entry.path));
    if (entry.kind === "dir") {
      setBrowse({ contextRoot, path: entry.path, project: true });
      setSearch("");
    } else dispatch(openFileInFilesPanel({ path: entry.path }));
  };

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
                onSelect={() => {
                  setManualPathError(null);
                  setPathDialog("open");
                }}
              >
                Open file by path…
              </Menu.Item>
              <Menu.Item
                onSelect={() => {
                  setManualPathError(null);
                  setPathDialog("browse");
                }}
              >
                Browse folder…
              </Menu.Item>
              <Menu.Separator />
              {(["name", "type", "size"] as const).map((value) => (
                <Menu.Item
                  key={value}
                  role="menuitemradio"
                  aria-checked={sort === value}
                  onSelect={() => setSort(value)}
                >
                  <span className={styles.menuCheck}>
                    {sort === value ? <Icon icon={Check} size="sm" /> : null}
                  </span>
                  Sort by {value}
                </Menu.Item>
              ))}
              <Menu.Item
                onSelect={() =>
                  setDirection(direction === "asc" ? "desc" : "asc")
                }
              >
                <Icon icon={ArrowDownUp} size="sm" />{" "}
                {direction === "asc" ? "Ascending" : "Descending"}
              </Menu.Item>
              <Menu.Separator />
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
        <div className={styles.openPathForm}>
          <div className={styles.openPathRow}>
            <FieldText
              aria-label="Search project files and folders"
              placeholder="Search files and folders…"
              autoComplete="off"
              className={styles.openPathInput}
              value={search}
              onChange={setSearch}
              role="combobox"
              aria-expanded={Boolean(search.trim())}
              aria-controls={`${inputId}-results`}
              aria-autocomplete="list"
              aria-activedescendant={
                results?.entries[activeResult]
                  ? `${inputId}-result-${String(activeResult)}`
                  : undefined
              }
              onKeyDown={(event) => {
                if (event.key === "Escape" && search) {
                  event.preventDefault();
                  event.stopPropagation();
                  setSearch("");
                }
                if (event.key === "ArrowDown" || event.key === "ArrowUp") {
                  event.preventDefault();
                  const length = results?.entries.length ?? 0;
                  setActiveResult((index) =>
                    length
                      ? Math.max(
                          0,
                          Math.min(
                            length - 1,
                            index + (event.key === "ArrowDown" ? 1 : -1),
                          ),
                        )
                      : -1,
                  );
                }
                if (event.key === "Enter" && results?.entries[activeResult]) {
                  event.preventDefault();
                  activateResult(results.entries[activeResult]);
                }
              }}
            />
            {search ? (
              <IconButton
                aria-label="Clear search"
                icon={X}
                size="sm"
                variant="ghost"
                onClick={() => setSearch("")}
              />
            ) : null}
          </div>
          {(projectRoots?.length ?? 0) > 1 ? (
            <select
              aria-label="Search scope"
              value={searchRoot}
              onChange={(event) => setSearchRoot(event.target.value)}
              className={styles.searchScope}
            >
              <option value="">All project roots</option>
              {projectRoots?.map((root) => (
                <option key={root} value={root}>
                  {root}
                </option>
              ))}
            </select>
          ) : null}
        </div>
        <Dialog
          open={pathDialog !== null}
          onOpenChange={(open) => {
            if (!open) setPathDialog(null);
          }}
        >
          <Dialog.Content maxWidth="480px">
            <Dialog.Title>
              {pathDialog === "browse" ? "Browse folder" : "Open file by path"}
            </Dialog.Title>
            <Dialog.Description>
              Enter an absolute path. This does not change project search roots.
            </Dialog.Description>
            <form
              aria-label="Open file by absolute path"
              className={styles.openPathForm}
              noValidate
              onSubmit={(event) => {
                if (pathDialog === "browse") {
                  event.preventDefault();
                  onBrowsePath();
                } else onOpenPath(event);
              }}
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
                <IconButton
                  aria-label={
                    pathDialog === "browse" ? "Browse folder" : "Open"
                  }
                  icon={pathDialog === "browse" ? FolderOpen : FileInput}
                  size="sm"
                  type="submit"
                />
              </div>
              {manualPathError ? (
                <p className={styles.openPathError} id={errorId} role="alert">
                  {manualPathError}
                </p>
              ) : null}
            </form>
          </Dialog.Content>
        </Dialog>
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
                        project: browse?.project,
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
        {search.trim() ? (
          <div className={styles.tree}>
            <div className={styles.searchStatus} role="status">
              {!results
                ? searchQuery.isError
                  ? "Search unavailable. Try again."
                  : "Searching project…"
                : `${String(results.entries.length)} results`}
              {results && (!results.complete || results.index_state !== "ready")
                ? ` · Index ${results.index_state}; results may be incomplete`
                : ""}
              {results && searchQuery.isError
                ? " · Refresh failed; showing previous results"
                : ""}
              {results?.truncated
                ? " · Results truncated; refine your search"
                : ""}
              {results?.complete && results.entries.length === 0
                ? " · No matches"
                : ""}
            </div>
            {searchActive &&
            (searchQuery.isError ||
              (results &&
                (!results.complete || results.index_state !== "ready"))) ? (
              <Button
                size="sm"
                variant="ghost"
                disabled={!visible || searchQuery.isFetching}
                onClick={() => void searchQuery.refetch()}
              >
                {searchQuery.isError ? "Retry search" : "Refresh search"}
              </Button>
            ) : null}
            <div
              role="listbox"
              aria-label="Search results"
              id={`${inputId}-results`}
            >
              {results?.entries.map((entry, index) => (
                <button
                  key={entry.path}
                  id={`${inputId}-result-${String(index)}`}
                  type="button"
                  role="option"
                  aria-selected={activeResult === index}
                  className={styles.searchResult}
                  data-selected={activeResult === index || undefined}
                  onFocus={() => setActiveResult(index)}
                  onClick={() => activateResult(entry)}
                  title={entry.path}
                >
                  <span className={styles.treeName}>{entry.name}</span>
                  <span className={styles.searchDetail}>
                    {entry.relative_path}
                  </span>
                  <span className={styles.searchDetail}>
                    {entry.root} · {entry.kind === "dir" ? "Folder" : "File"}
                    {entry.size !== null
                      ? ` · ${String(entry.size)} bytes`
                      : ""}
                    {entry.ignored ? " · Ignored" : ""}
                    {` · ${entry.privacy_zone.name}`}
                  </span>
                </button>
              ))}
            </div>
          </div>
        ) : (
          <FileTree
            key={`${contextRoot}:${browsePath ?? ""}`}
            browsePath={browsePath}
            projectBrowse={browse?.project}
            sort={sort}
            direction={direction}
          />
        )}
      </aside>
    </div>
  );
}
