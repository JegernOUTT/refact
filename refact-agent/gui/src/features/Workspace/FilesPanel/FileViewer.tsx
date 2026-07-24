import type { FetchBaseQueryError } from "@reduxjs/toolkit/query";
import { Copy, FileQuestion, RotateCw } from "lucide-react";
import { useCallback, useEffect, useMemo } from "react";

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
import { useReadFileQuery } from "../../../services/refact/files";
import {
  expandDirectory,
  isPathWithinWorkspaceRoots,
  selectFileViewerTargetByPath,
  selectTreePath,
} from "./filesPanelSlice";
import { pathBasename } from "./fileTreeModel";
import { HighlightedFile } from "./HighlightedFile";
import styles from "./FilesPanel.module.css";

const errorStatus = (error: unknown): number | string | null => {
  const candidate = error as FetchBaseQueryError | undefined;
  return candidate?.status ?? null;
};

type Breadcrumb = {
  label: string;
  path: string;
};

const EMPTY_ROOTS: string[] = [];

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
  const workspaceRoots = useAppSelector(
    (state) => state.current_project.workspaceRoots ?? EMPTY_ROOTS,
  );
  const target = storedTarget ?? { path };
  const { data, error, isFetching, refetch } = useReadFileQuery({ path });
  const breadcrumbs = useMemo(
    () => breadcrumbsForPath(path, workspaceRoots),
    [path, workspaceRoots],
  );

  useEffect(() => {
    if (!target.line || !data) return;
    const timer = window.setTimeout(() => {
      document
        .getElementById("files-panel-target-line")
        ?.scrollIntoView({ block: "center" });
    }, 0);
    return () => window.clearTimeout(timer);
  }, [data, target.line]);

  const openBreadcrumb = useCallback(
    (crumb: Breadcrumb, index: number) => {
      if (
        index === breadcrumbs.length - 1 ||
        !isPathWithinWorkspaceRoots(crumb.path, workspaceRoots)
      ) {
        return;
      }
      dispatch(expandDirectory(crumb.path));
      dispatch(selectTreePath(crumb.path));
    },
    [breadcrumbs.length, dispatch, workspaceRoots],
  );

  const blocked = errorStatus(error) === 403;
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

      {isFetching && !data ? (
        <LoadingState
          label={`Loading ${pathBasename(target.path)}`}
          variant="full"
        />
      ) : error ? (
        <ErrorState
          description={
            blocked
              ? "This file is blocked by privacy rules."
              : "The workspace worker could not read this file."
          }
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
      ) : data ? (
        <>
          <div className={styles.fileMeta}>
            <span>{data.language ?? "Plain text"}</span>
            <span>{data.size.toLocaleString()} bytes</span>
          </div>
          {data.truncated ? (
            <div className={styles.truncatedBanner} role="status">
              File truncated at 1 MiB
            </div>
          ) : null}
          <div className={`${styles.codeScroll} scrollX`}>
            <HighlightedFile
              content={data.content}
              language={data.language}
              lineStart={lineStart}
              targetLine={target.line}
            />
          </div>
        </>
      ) : null}
    </section>
  );
}
