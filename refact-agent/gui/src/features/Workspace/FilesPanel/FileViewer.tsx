import { createPortal } from "react-dom";
import type { EditorView } from "@codemirror/view";
import {
  Code2,
  Copy,
  Eye,
  FileQuestion,
  Info,
  Lock,
  Pencil,
  RotateCw,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import {
  Button,
  EmptyState,
  ErrorState,
  IconButton,
  LoadingState,
  Popover,
  Tooltip,
} from "../../../components/ui";
import {
  useAppDispatch,
  useAppSelector,
  useCopyToClipboard,
} from "../../../hooks";
import type { DiffChunk } from "../../../services/refact";
import {
  MAX_PREVIEW_IMAGE_BYTES,
  imageDataUrl,
  useReadFileQuery,
  useWriteFileMutation,
} from "../../../services/refact/files";
import {
  selectActiveTabId,
  splitTab,
  selectFocusedChatWorktreeRoot,
  selectFocusedWorkspaceChatId,
  setDockOpen,
  setDockSection,
} from "../workspaceSlice";
import {
  usePaneHeaderTarget,
  usePaneSurfaceActive,
} from "../PaneHeaderContext";
import { useSurfaceSessionState } from "../surfaceSessionState";
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
import { HtmlFilePreview } from "./HtmlFilePreview";
import { ImageFilePreview } from "./ImageFilePreview";
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
const IMAGE_PATH_PATTERN = /\.(png|jpe?g|gif|webp)$/iu;

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
  const activeTabId = useAppSelector(selectActiveTabId);
  const headerTarget = usePaneHeaderTarget();
  const surfaceActive = usePaneSurfaceActive();
  const compact = headerTarget !== undefined;
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
  const [draft, setDraft] = useSurfaceSessionState<string | null>(
    path,
    "draft",
    null,
  );
  const [editBase, setEditBase] = useSurfaceSessionState<{
    mtimeMs: number;
    digest?: string;
  } | null>(path, "editBase", null);
  const [pendingCaret, setPendingCaret] = useSurfaceSessionState<number | null>(
    path,
    "caret",
    null,
  );
  const [showSource, setShowSource] = useSurfaceSessionState(
    path,
    "source",
    false,
  );
  const [discardIntent, setDiscardIntent] = useState<DiscardIntent>(null);
  const viewRef = useRef<EditorView | null>(null);
  const [writeFile, writeState] = useWriteFileMutation();
  const target = storedTarget ?? { path };
  const manual =
    storedTarget?.manual === true ||
    (!storedTarget &&
      workspaceRoots.length > 0 &&
      !isPathWithinWorkspaceRoots(path, workspaceRoots));
  const { data, error, isFetching, refetch } = useReadFileQuery(
    manual
      ? { path, manual: true }
      : {
          path,
          chatId: chatId ?? undefined,
          revision:
            liveUpdate?.operation === "write" ? liveUpdate.revision : undefined,
        },
  );
  const breadcrumbs = useMemo(
    () => breadcrumbsForPath(path, workspaceRoots),
    [path, workspaceRoots],
  );
  const unavailable =
    !manual &&
    (liveUpdate?.operation === "remove" || liveUpdate?.operation === "rename");
  const imageSrc = useMemo(
    () => imageDataUrl(data?.image_mime, data?.image_data),
    [data?.image_mime, data?.image_data],
  );
  const unsupportedImage = !!data?.image_mime && imageSrc === null;
  const canEdit = manual ? data?.can_edit === true : data?.can_edit !== false;
  const displayedContent = unavailable ? null : data?.content ?? null;
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
    canEdit &&
    !data.binary &&
    !data.image_mime &&
    !data.truncated &&
    data.line_start === null &&
    data.line_end === null &&
    !unavailable;
  const editing = draft !== null;
  const isDirty = editing && draft !== (data?.content ?? "");
  const isMarkdown = MARKDOWN_LANGUAGES.has(
    (data?.language ?? "").toLowerCase(),
  );
  const isHtml =
    /\.html?$/iu.test(path) || (data?.language ?? "").toLowerCase() === "html";
  const previewFormat = isHtml ? "HTML" : isMarkdown ? "Markdown" : null;
  const showPreview = previewFormat !== null && !editing && !showSource;
  const sourceToggleLabel = showSource
    ? `Show rendered ${previewFormat ?? ""}`
    : `Show ${previewFormat ?? ""} source`;
  const conflicted =
    (writeState.error as { status?: number } | undefined)?.status === 409;
  const basename = pathBasename(target.path);

  const caretRef = useRef(pendingCaret);
  caretRef.current = pendingCaret;

  useEffect(() => {
    if (!editing || !surfaceActive) return;
    const view = viewRef.current;
    if (!view) return;
    view.focus();
    const caret = caretRef.current;
    if (caret === null) return;
    const anchor = Math.max(0, Math.min(caret, view.state.doc.length));
    view.dispatch({ selection: { anchor } });
  }, [editing, surfaceActive]);

  const handleReady = useCallback(
    (view: EditorView) => {
      viewRef.current = view;
      const caret = pendingCaret;
      if (caret !== null)
        view.dispatch({
          selection: { anchor: Math.min(caret, view.state.doc.length) },
        });
      const listener = () => setPendingCaret(view.state.selection.main.head);
      view.dom.addEventListener("keyup", listener);
      view.dom.addEventListener("mouseup", listener);
    },
    [pendingCaret, setPendingCaret],
  );

  const requestEdit = useCallback(
    (offset: number | null) => {
      if (!editable || isPlaying) return;
      setDraft(data.content);
      setEditBase({ mtimeMs: data.mtime_ms, digest: data.digest });
      setPendingCaret(offset);
      setDiscardIntent(null);
    },
    [data, editable, isPlaying, setDraft, setEditBase, setPendingCaret],
  );

  const cancelEdit = useCallback(() => {
    setDraft(null);
    setEditBase(null);
    setPendingCaret(null);
    setDiscardIntent(null);
  }, [setDraft, setEditBase, setPendingCaret]);

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
    if (draft === null || !data || !editBase) return;
    if (!editable || isPlaying || writeState.isLoading) return;
    const result = await writeFile({
      path,
      content: draft,
      expectedMtimeMs: editBase.mtimeMs,
      expectedDigest: editBase.digest,
    });
    if ("data" in result) {
      setDraft(null);
      setEditBase(null);
      setPendingCaret(null);
      setDiscardIntent(null);
      void refetch();
    }
  }, [
    data,
    draft,
    editBase,
    editable,
    isPlaying,
    path,
    refetch,
    writeFile,
    writeState.isLoading,
    setDraft,
    setEditBase,
    setPendingCaret,
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
    : manual
      ? errorDetail(error) ?? "This file could not be opened."
      : isAccessDenied(error)
        ? errorDetail(error) ??
          "This file is outside the directories the workspace worker may read."
        : "The workspace worker could not read this file.";
  const lineStart = data?.line_start ?? 1;

  const controls = (
    <div className={styles.viewerControls}>
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
          {previewFormat !== null ? (
            <Tooltip content={sourceToggleLabel}>
              <IconButton
                aria-label={sourceToggleLabel}
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
                : data && !canEdit
                  ? "This file is read-only"
                  : editable
                    ? "Edit this file"
                    : "This file cannot be edited here"
            }
          >
            <IconButton
              aria-label="Edit this file"
              className={
                !editable || isPlaying ? styles.unavailableEdit : undefined
              }
              disabled={!editable || isPlaying}
              icon={Pencil}
              onClick={() => requestEdit(null)}
              size="sm"
              variant="plain"
            />
          </Tooltip>
        </>
      )}
      {data && !canEdit ? (
        <span
          className={styles.readOnlyIndicator}
          role="status"
          aria-label="Read-only"
          title="Read-only"
        >
          <Lock aria-hidden="true" />
          <span className={styles.srOnly}>Read-only</span>
        </span>
      ) : null}
      <Popover>
        <Popover.Trigger asChild>
          <IconButton
            aria-label="File information"
            icon={Info}
            size="sm"
            variant="plain"
          />
        </Popover.Trigger>
        <Popover.Content align="end" aria-label="File information">
          <div className={styles.fileInfo}>
            <div className={styles.fullPath}>{target.path}</div>
            <nav aria-label="File path" className={styles.breadcrumbs}>
              {breadcrumbs.map((crumb, index) => (
                <span className={styles.breadcrumbPart} key={crumb.path}>
                  {index > 0 ? (
                    <span className={styles.separator}>/</span>
                  ) : null}
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
            {data ? (
              <div className={styles.fileInfoMeta}>
                <span>{data.image_mime ?? data.language ?? "Plain text"}</span>
                <span>{data.size.toLocaleString()} bytes</span>
                {!canEdit ? <span>Read-only</span> : null}
              </div>
            ) : null}
            <div className={styles.editorActions}>
              <Tooltip content="Copy file path">
                <IconButton
                  aria-label="Copy file path"
                  icon={Copy}
                  onClick={() => copyToClipboard(target.path)}
                  size="sm"
                  variant="plain"
                />
              </Tooltip>
              <Button
                leftIcon={RotateCw}
                onClick={requestReload}
                size="sm"
                variant="plain"
              >
                Reload
              </Button>
            </div>
            <Button
              onClick={() =>
                dispatch(
                  splitTab({
                    tabId: activeTabId ?? `file:${path}`,
                    dir: "row",
                  }),
                )
              }
              size="sm"
              variant="plain"
            >
              Split pane right
            </Button>
          </div>
        </Popover.Content>
      </Popover>
    </div>
  );

  return (
    <section className={styles.viewer} aria-label="File viewer">
      {compact ? (
        headerTarget && surfaceActive ? (
          createPortal(controls, headerTarget)
        ) : null
      ) : (
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
          {controls}
        </header>
      )}

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
      ) : imageSrc !== null && data ? (
        <>
          <ImageFilePreview
            alt={basename}
            key={path}
            sessionPath={path}
            src={imageSrc}
          />
        </>
      ) : data && (unsupportedImage || data.binary) ? (
        <EmptyState
          icon={FileQuestion}
          title={
            IMAGE_PATH_PATTERN.test(basename) &&
            data.size > MAX_PREVIEW_IMAGE_BYTES
              ? "Image too large"
              : "Binary file"
          }
          description={
            IMAGE_PATH_PATTERN.test(basename) &&
            data.size > MAX_PREVIEW_IMAGE_BYTES
              ? `${basename} is too large to preview (${data.size.toLocaleString()} bytes). Image previews are limited to 10 MiB.`
              : unsupportedImage
                ? `${basename} uses an image format that cannot be previewed safely (${data.size.toLocaleString()} bytes).`
                : `${basename} is binary and cannot be previewed (${data.size.toLocaleString()} bytes).`
          }
          variant="full"
        />
      ) : displayedContent !== null ? (
        <>
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
          {showPreview && isHtml ? (
            <HtmlFilePreview content={displayedContent} />
          ) : showPreview ? (
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
