import { skipToken, type FetchBaseQueryError } from "@reduxjs/toolkit/query";
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
  selectFileViewerTarget,
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

const breadcrumbsForPath = (path: string): Breadcrumb[] => {
  const normalized = path.replace(/\\/g, "/");
  const rootPrefix = normalized.startsWith("/") ? "/" : "";
  const segments = normalized.split("/").filter(Boolean);
  return segments.map((label, index) => ({
    label,
    path: rootPrefix + segments.slice(0, index + 1).join("/"),
  }));
};

export function FileViewer() {
  const dispatch = useAppDispatch();
  const copyToClipboard = useCopyToClipboard();
  const target = useAppSelector(selectFileViewerTarget);
  const request = target ? { path: target.path } : skipToken;
  const { data, error, isFetching, refetch } = useReadFileQuery(request);
  const breadcrumbs = useMemo(
    () => (target ? breadcrumbsForPath(target.path) : []),
    [target],
  );

  useEffect(() => {
    if (!target?.line || !data) return;
    const timer = window.setTimeout(() => {
      document
        .getElementById("files-panel-target-line")
        ?.scrollIntoView({ block: "center" });
    }, 0);
    return () => window.clearTimeout(timer);
  }, [data, target?.line]);

  const openBreadcrumb = useCallback(
    (crumb: Breadcrumb, index: number) => {
      if (index === breadcrumbs.length - 1) return;
      dispatch(expandDirectory(crumb.path));
      dispatch(selectTreePath(crumb.path));
    },
    [breadcrumbs.length, dispatch],
  );

  if (!target) {
    return (
      <EmptyState
        icon={FileQuestion}
        title="Select a file"
        description="Choose a file from the explorer to view it here."
        variant="full"
      />
    );
  }

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
                disabled={index === breadcrumbs.length - 1}
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
          description={` is binary and cannot be previewed ( bytes).`}
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
