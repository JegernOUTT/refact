import type { EditorView } from "@codemirror/view";
import { Code2, Copy, Eye, FileQuestion, Pencil, RotateCw } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import {
  Button,
  EmptyState,
  ErrorState,
  IconButton,
  LoadingState,
  Tooltip,
} from "../../../components/ui";
import {
  useAppDispatch,
  useAppSelector,
  useCopyToClipboard,
} from "../../../hooks";
import type { DiffChunk } from "../../../services/refact";
import {
  useReadFileQuery,
  useWriteFileMutation,
} from "../../../services/refact/files";
import {
  selectFocusedChatWorktreeRoot,
  selectFocusedWorkspaceChatId,
  setDockOpen,
  setDockSection,
} from "../workspaceSlice";
import { CodeMirrorEditor } from "./CodeMirrorEditor";
import { EditPlayerControls } from "./EditPlayerControls";
import {
  expandDirectory,
  isPathWithinWorkspaceRoots,
  selectActiveEditPlayerStep,
  selectFileViewerTargetByPath,
  selectIsEditPlaying,
  selectLiveFileUpdate,
  selectTreePath,
} from "./filesPanelSlice";
import { pathBasename } from "./fileTreeModel";
import {
  errorDetail,
  isAccessDenied,
  isPrivacyBlocked,
} from "./filesPanelErrors";
import { changedLineNumbers } from "./liveFileModel";
import { MarkdownFilePreview } from "./MarkdownFilePreview";
import styles from "./FilesPanel.module.css";

type Breadcrumb = {
  label: string;
  path: string;
};

type DiscardIntent = "cancel" | "reload" | null;

const EMPTY_ROOTS: string[] = [];
const EMPTY_CHUNKS: DiffChunk[] = [];
const MARKDOWN_LANGUAGES = new Set(["markdown", "md", "mdx"]);

const normalizeBreadcrumbPath = (path: string): string => {
  const normalized = path.replace(/\\/g, "/");
  if (/^\/+$/u.test(normalized)) return "/";
  if (/^[A-Za-z]:\/+$/u.test(normalized)) {
    return `${normalized.slice(0, 2)}/`;
  }
  return normalized.replace(/\/+$/u, "");
};

const breadcrumbsForPath = (
  path: string,
  workspaceRoots: string[],
): Breadcrumb[] => {
  const normalizedPath = normalizeBreadcrumbPath(path);
  const workspaceRoot = workspaceRoots
    .map(normalizeBreadcrumbPath)
    .filter((root) =>
      isPathWithinWorkspaceRoots(normalizedPath, root ? [root] : []),
    )
    .sort((left, right) => right.length - left.length)[0];

  if (!workspaceRoot) {
    return [{ label: pathBasename(normalizedPath), path: normalizedPath }];
  }

  const relativePath = normalizedPath
    .slice(workspaceRoot.length)
    .replace(/^\/+/, "");
  const segments = relativePath.split("/").filter(Boolean);
  const rootLabel = pathBasename(workspaceRoot) || workspaceRoot;
  return [
    { label: rootLabel, path: workspaceRoot },
    ...segments.map((label, index) => {
      const suffix = segments.slice(0, index + 1).join("/");
      const crumbPath = workspaceRoot.endsWith("/")
        ? `${workspaceRoot}${suffix}`
        : `${workspaceRoot}/${suffix}`;
      return {
        label,
        path: crumbPath,
      };
    }),
  ];
};

export function FileViewer({ path }: { path: string }) {
  const dispatch = useAppDispatch();
  const copyToClipboard = useCopyToClipboard();
  const storedTarget = useAppSelector((state) =>
    selectFileViewerTargetByPath(state, path),
  );
  const chatId = useAppSelector(selectFocusedWorkspaceChatId);
  const liveUpdate = useAppSelector((state) =>
    selectLiveFileUpdate(state, chatId, path),
  );
  const configuredWorkspaceRoots = useAppSelector(
    (state) => state.current_project.workspaceRoots ?? EMPTY_ROOTS,
  );
  const worktreeRoot = useAppSelector(selectFocusedChatWorktreeRoot);
  const workspaceRoots = useMemo(
    () => (worktreeRoot ? [worktreeRoot] : configuredWorkspaceRoots),
    [configuredWorkspaceRoots, worktreeRoot],
  );
  const isPlaying = useAppSelector(selectIsEditPlaying);
  const activeStep = useAppSelector(selectActiveEditPlayerStep);
  const playbackStep = activeStep?.path === path ? activeStep : undefined;
  const [draft, setDraft] = useState<string | null>(null);
  const [pendingCaret, setPendingCaret] = useState<number | null>(null);
  const [showSource, setShowSource] = useState(false);
  const [discardIntent, setDiscardIntent] = useState<DiscardIntent>(null);
  const viewRef = useRef<EditorView | null>(null);
  const [writeFile, writeState] = useWriteFileMutation();
  const target = storedTarget ?? { path };
  const { data, error, isFetching, refetch } = useReadFileQuery({
    path,
    chatId: chatId ?? undefined,
    revision:
      liveUpdate?.operation === "write" ? liveUpdate.revision : undefined,
  });
  const breadcrumbs = useMemo(
    () => breadcrumbsForPath(path, workspaceRoots),
    [path, workspaceRoots],
  );
  const unavailable =
    liveUpdate?.operation === "remove" || liveUpdate?.operation === "rename";
  const displayedContent = unavailable ? null : data?.content ?? null;
  const displayedByteLength = useMemo(
    () =>
      displayedContent === null
        ? 0
        : new TextEncoder().encode(displayedContent).length,
    [displayedContent],
  );
  const revealChunks = useMemo(
    () => playbackStep?.chunks ?? liveUpdate?.chunks ?? EMPTY_CHUNKS,
    [liveUpdate, playbackStep],
  );
  const changedLines = useMemo(
    () => changedLineNumbers(revealChunks),
    [revealChunks],
  );
  const changeRevision = playbackStep
    ? `${playbackStep.id}`
    : liveUpdate?.authoritative
      ? liveUpdate.revision
      : undefined;
  const editable =
    !!data &&
    !data.binary &&
    !data.truncated &&
    data.line_start === null &&
    data.line_end === null &&
    !unavailable;
  const editing = draft !== null;
  const isDirty = editing && draft !== (data?.content ?? "");
  const isMarkdown = MARKDOWN_LANGUAGES.has(
    (data?.language ?? "").toLowerCase(),
  );
  const showPreview = isMarkdown && !editing && !showSource;
  const conflicted =
    (writeState.error as { status?: number } | undefined)?.status === 409;
  const basename = pathBasename(target.path);

  useEffect(() => {
    setDraft(null);
    setPendingCaret(null);
    setShowSource(false);
    setDiscardIntent(null);
    viewRef.current = null;
  }, [path]);

  useEffect(() => {
    if (!editing) return;
    const view = viewRef.current;
    if (!view) return;
    view.focus();
    if (pendingCaret === null) return;
    const anchor = Math.max(0, Math.min(pendingCaret, view.state.doc.length));
    view.dispatch({ selection: { anchor } });
    setPendingCaret(null);
  }, [editing, pendingCaret]);

  const handleReady = useCallback((view: EditorView) => {
    viewRef.current = view;
  }, []);

  const requestEdit = useCallback(
    (offset: number | null) => {
      if (!editable || isPlaying) return;
      setDraft(data.content);
      setPendingCaret(offset);
      setDiscardIntent(null);
    },
    [data?.content, editable, isPlaying],
  );

  const cancelEdit = useCallback(() => {
    setDraft(null);
    setPendingCaret(null);
    setDiscardIntent(null);
  }, []);

  const requestCancel = useCallback(() => {
    if (isDirty) {
      setDiscardIntent("cancel");
      return;
    }
    cancelEdit();
  }, [cancelEdit, isDirty]);

  const reloadFromDisk = useCallback(() => {
    cancelEdit();
    void refetch();
  }, [cancelEdit, refetch]);

  const requestReload = useCallback(() => {
    if (isDirty) {
      setDiscardIntent("reload");
      return;
    }
    reloadFromDisk();
  }, [isDirty, reloadFromDisk]);

  const confirmDiscard = useCallback(() => {
    if (discardIntent === "reload") {
      reloadFromDisk();
      return;
    }
    cancelEdit();
  }, [cancelEdit, discardIntent, reloadFromDisk]);

  const saveDraft = useCallback(async () => {
    if (draft === null || !data) return;
    if (!editable || isPlaying || writeState.isLoading) return;
    const result = await writeFile({
      path,
      content: draft,
      expectedMtimeMs: data.mtime_ms,
    });
    if ("data" in result) {
      setDraft(null);
      setPendingCaret(null);
      setDiscardIntent(null);
      void refetch();
    }
  }, [
    data,
    draft,
    editable,
    isPlaying,
    path,
    refetch,
    writeFile,
    writeState.isLoading,
  ]);

  const openBreadcrumb = useCallback(
    (crumb: Breadcrumb, index: number) => {
      if (
        index === breadcrumbs.length - 1 ||
        !isPathWithinWorkspaceRoots(crumb.path, workspaceRoots)
      ) {
        return;
      }
      dispatch(setDockOpen(true));
      dispatch(setDockSection("files"));
      dispatch(expandDirectory(crumb.path));
      dispatch(selectTreePath(crumb.path));
    },
    [breadcrumbs.length, dispatch, workspaceRoots],
  );

  const blocked = isPrivacyBlocked(error);
  const unreadableDescription = blocked
    ? "This file is blocked by privacy rules."
    : isAccessDenied(error)
      ? errorDetail(error) ??
        "This file is outside the directories the workspace worker may read."
      : "The workspace worker could not read this file.";
  const lineStart = data?.line_start ?? 1;

  return (
    <section className={styles.viewer} aria-label="File viewer">
      <header className={styles.viewerHeader}>
        <nav aria-label="File path" className={styles.breadcrumbs}>
          {breadcrumbs.map((crumb, index) => (
            <span className={styles.breadcrumbPart} key={crumb.path}>
              {index > 0 ? <span className={styles.separator}>/</span> : null}
              <button
                className={styles.breadcrumb}
                disabled={
                  index === breadcrumbs.length - 1 ||
                  !isPathWithinWorkspaceRoots(crumb.path, workspaceRoots)
                }
                onClick={() => openBreadcrumb(crumb, index)}
                type="button"
              >
                {crumb.label}
              </button>
            </span>
          ))}
        </nav>
        <EditPlayerControls />
        {editing ? (
          discardIntent !== null ? (
            <div className={styles.editorActions}>
              <span className={styles.discardPrompt} role="alert">
                Discard changes?
              </span>
              <Button onClick={confirmDiscard} size="sm" variant="plain">
                Discard
              </Button>
              <Button onClick={() => setDiscardIntent(null)} size="sm">
                Keep editing
              </Button>
            </div>
          ) : (
            <div className={styles.editorActions}>
              <Button
                disabled={isPlaying || writeState.isLoading}
                onClick={() => void saveDraft()}
                size="sm"
              >
                {writeState.isLoading ? "Saving" : "Save"}
              </Button>
              <Button onClick={requestCancel} size="sm" variant="plain">
                Cancel
              </Button>
            </div>
          )
        ) : (
          <>
            {isMarkdown ? (
              <Tooltip
                content={
                  showSource ? "Show rendered Markdown" : "Show Markdown source"
                }
              >
                <IconButton
                  aria-label={
                    showSource
                      ? "Show rendered Markdown"
                      : "Show Markdown source"
                  }
                  icon={showSource ? Eye : Code2}
                  onClick={() => setShowSource((previous) => !previous)}
                  size="sm"
                  variant="plain"
                />
              </Tooltip>
            ) : null}
            <Tooltip
              content={
                isPlaying
                  ? "Editing is locked while edits are playing"
                  : editable
                    ? "Edit this file"
                    : "This file cannot be edited here"
              }
            >
              <IconButton
                aria-label="Edit this file"
                disabled={!editable || isPlaying}
                icon={Pencil}
                onClick={() => requestEdit(null)}
                size="sm"
                variant="plain"
              />
            </Tooltip>
          </>
        )}
        <Tooltip content="Copy file path">
          <IconButton
            aria-label="Copy file path"
            icon={Copy}
            onClick={() => copyToClipboard(target.path)}
            size="sm"
            variant="plain"
          />
        </Tooltip>
      </header>

      {unavailable ? (
        <ErrorState
          description={
            liveUpdate.operation === "rename" && liveUpdate.renamedTo
              ? `This file was renamed to ${liveUpdate.renamedTo}.`
              : "This file was deleted from the workspace."
          }
          title={
            liveUpdate.operation === "rename" ? "File renamed" : "File deleted"
          }
          variant="full"
        />
      ) : isFetching && !data ? (
        <LoadingState label={`Loading ${basename}`} variant="full" />
      ) : error && displayedContent === null ? (
        <ErrorState
          description={unreadableDescription}
          retry={
            <Button
              leftIcon={RotateCw}
              onClick={() => void refetch()}
              size="sm"
            >
              Retry
            </Button>
          }
          title={blocked ? "File blocked" : "File unavailable"}
          variant="full"
        />
      ) : data?.binary ? (
        <EmptyState
          icon={FileQuestion}
          title="Binary file"
          description={`${basename} is binary and cannot be previewed (${data.size.toLocaleString()} bytes).`}
          variant="full"
        />
      ) : displayedContent !== null ? (
        <>
          <div className={styles.fileMeta}>
            <span>{data?.language ?? "Plain text"}</span>
            <span>{displayedByteLength.toLocaleString()} bytes</span>
          </div>
          {data?.truncated ? (
            <div className={styles.truncatedBanner} role="status">
              File truncated at 1 MiB
            </div>
          ) : null}
          {conflicted ? (
            <div className={styles.conflictBanner} role="alert">
              This file changed on disk since it was loaded.
              <Button onClick={requestReload} size="sm" variant="plain">
                Reload
              </Button>
            </div>
          ) : null}
          {showPreview ? (
            <MarkdownFilePreview
              content={displayedContent}
              onRequestEdit={
                editable && !isPlaying ? () => requestEdit(null) : undefined
              }
            />
          ) : (
            <CodeMirrorEditor
              ariaLabel={
                editing ? `Edit ${basename}` : `${basename} file contents`
              }
              changedLines={changedLines}
              changeRevision={changeRevision}
              language={data?.language ?? null}
              lineStart={data ? lineStart : 1}
              onCancel={editing ? requestCancel : undefined}
              onChange={editing ? setDraft : undefined}
              onReady={handleReady}
              onRequestEdit={(offset) => requestEdit(offset)}
              onSave={
                editing && !isPlaying && !writeState.isLoading
                  ? () => void saveDraft()
                  : undefined
              }
              readOnly={!editing || isPlaying}
              removedChunks={revealChunks}
              targetLine={target.line}
              value={editing ? draft : displayedContent}
            />
          )}
        </>
      ) : null}
    </section>
  );
}
