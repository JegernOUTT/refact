import { Copy, FileQuestion, Pencil, RotateCw } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";

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
import { HighlightedFile } from "./HighlightedFile";
import { changedLineNumbers } from "./liveFileModel";
import styles from "./FilesPanel.module.css";

type Breadcrumb = {
  label: string;
  path: string;
};

const EMPTY_ROOTS: string[] = [];
const EMPTY_CHUNKS: DiffChunk[] = [];

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
  const conflicted =
    (writeState.error as { status?: number } | undefined)?.status === 409;

  useEffect(() => {
    if (!target.line || !data) return;
    const timer = window.setTimeout(() => {
      document
        .getElementById("files-panel-target-line")
        ?.scrollIntoView({ block: "center" });
    }, 0);
    return () => window.clearTimeout(timer);
  }, [data, target.line]);

  const saveDraft = useCallback(async () => {
    if (draft === null || !data) return;
    const result = await writeFile({
      path,
      content: draft,
      expectedMtimeMs: data.mtime_ms,
    });
    if ("data" in result) {
      setDraft(null);
      void refetch();
    }
  }, [data, draft, path, refetch, writeFile]);

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
          <div className={styles.editorActions}>
            <Button
              disabled={isPlaying || writeState.isLoading}
              onClick={() => void saveDraft()}
              size="sm"
            >
              {writeState.isLoading ? "Saving" : "Save"}
            </Button>
            <Button onClick={() => setDraft(null)} size="sm" variant="plain">
              Cancel
            </Button>
          </div>
        ) : (
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
              onClick={() => setDraft(data?.content ?? "")}
              size="sm"
              variant="plain"
            />
          </Tooltip>
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
        <LoadingState
          label={`Loading ${pathBasename(target.path)}`}
          variant="full"
        />
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
          description={`${pathBasename(
            target.path,
          )} is binary and cannot be previewed (${data.size.toLocaleString()} bytes).`}
          variant="full"
        />
      ) : displayedContent !== null ? (
        <>
          <div className={styles.fileMeta}>
            <span>{data?.language ?? "Plain text"}</span>
            <span>{displayedContent.length.toLocaleString()} bytes</span>
          </div>
          {data?.truncated ? (
            <div className={styles.truncatedBanner} role="status">
              File truncated at 1 MiB
            </div>
          ) : null}
          {conflicted ? (
            <div className={styles.conflictBanner} role="alert">
              This file changed on disk since it was loaded.
              <Button
                onClick={() => {
                  setDraft(null);
                  void refetch();
                }}
                size="sm"
                variant="plain"
              >
                Reload
              </Button>
            </div>
          ) : null}
          {editing ? (
            <textarea
              aria-label={`Edit ${pathBasename(target.path)}`}
              className={styles.editor}
              onChange={(event) => setDraft(event.target.value)}
              readOnly={isPlaying}
              spellCheck={false}
              value={draft}
            />
          ) : (
            <div className={`${styles.codeScroll} scrollX`}>
              <HighlightedFile
                content={displayedContent}
                changedLines={changedLines}
                changeRevision={changeRevision}
                language={data?.language ?? null}
                lineStart={data ? lineStart : 1}
                removedChunks={revealChunks}
                targetLine={target.line}
              />
            </div>
          )}
        </>
      ) : null}
    </section>
  );
}
