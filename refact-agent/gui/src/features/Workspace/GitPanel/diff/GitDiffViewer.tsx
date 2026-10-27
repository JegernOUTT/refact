import classNames from "classnames";
import {
  ArrowDown,
  ArrowUp,
  RefreshCw,
  SquareArrowOutUpRight,
} from "lucide-react";
import { useCallback, useMemo, useState, type KeyboardEvent } from "react";

import {
  IconButton,
  SegmentedControl,
  Spinner,
} from "../../../../components/ui";
import { useAppDispatch, useAppSelector } from "../../../../hooks";
import { useOpenFileInApp } from "../../../../hooks/useOpenFileInApp";
import type { GitFileResponse } from "../../../../services/refact/gitRead";
import type { SurfaceKey } from "../../surfaceKey";
import { gitErrorText } from "../gitErrors";
import {
  selectGitDiffViewerPrefs,
  setGitDiffViewerPrefs,
  type GitDiffViewMode,
} from "../gitPanelSlice";
import { DiffEditorsHost } from "./DiffEditorsHost";
import type { DiffEditorsController } from "./diffEditors";
import {
  formatBytes,
  gitRevLabel,
  joinRepoPath,
  type GitDiffDescriptor,
  type GitDiffSide,
} from "./gitDiffDescriptor";
import { useGitDiffSides } from "./useGitDiffSides";
import styles from "./GitDiffViewer.module.css";

export type GitDiffViewerProps = {
  descriptor: GitDiffDescriptor;
  surfaceKey?: SurfaceKey;
};

const MODE_OPTIONS = [
  { value: "side-by-side", label: "Side-by-side" },
  { value: "unified", label: "Unified" },
];

const TRUNCATE_LIMIT_LABEL = "2 MiB";

function sideRevLabel(
  side: GitDiffSide | null,
  descriptor: GitDiffDescriptor,
): string {
  return side ? gitRevLabel(side.rev, descriptor) : "Missing";
}

function sideAriaLabel(
  side: GitDiffSide | null,
  descriptor: GitDiffDescriptor,
  fallbackPath: string,
): string {
  const path = side?.path ?? fallbackPath;
  if (!side) return `Missing version of ${path}`;
  if (side.rev === "WORKTREE") return `Working tree version of ${path}`;
  return `${gitRevLabel(side.rev, descriptor)} version of ${path}`;
}

type DiffContent =
  | { state: "loading" }
  | { state: "error"; message: string }
  | {
      state: "binary";
      left: GitFileResponse | null;
      right: GitFileResponse | null;
    }
  | {
      state: "ready";
      left: string;
      right: string;
      leftMissing: boolean;
      rightMissing: boolean;
      truncated: boolean;
      identical: boolean;
    };

function sideExists(
  side: GitDiffSide | null,
  data: GitFileResponse | undefined,
): boolean {
  return Boolean(side && data?.exists);
}

export function GitDiffViewer({ descriptor, surfaceKey }: GitDiffViewerProps) {
  const dispatch = useAppDispatch();
  const prefs = useAppSelector(selectGitDiffViewerPrefs);
  const { canOpen, openFile } = useOpenFileInApp();
  const { left, right, refreshAll } = useGitDiffSides(descriptor, surfaceKey);
  const [controller, setController] = useState<DiffEditorsController | null>(
    null,
  );

  const content: DiffContent = useMemo(() => {
    const error = left.error ?? right.error;
    if (error) return { state: "error", message: gitErrorText(error).message };
    const leftPending = descriptor.left !== null && !left.data;
    const rightPending = descriptor.right !== null && !right.data;
    if (leftPending || rightPending) {
      return { state: "loading" };
    }
    const leftData = descriptor.left ? left.data ?? null : null;
    const rightData = descriptor.right ? right.data ?? null : null;
    if (leftData?.binary === true || rightData?.binary === true) {
      return { state: "binary", left: leftData, right: rightData };
    }
    const leftMissing = !sideExists(descriptor.left, left.data);
    const rightMissing = !sideExists(descriptor.right, right.data);
    const leftText = leftMissing ? "" : leftData?.content ?? "";
    const rightText = rightMissing ? "" : rightData?.content ?? "";
    return {
      state: "ready",
      left: leftText,
      right: rightText,
      leftMissing,
      rightMissing,
      truncated: leftData?.truncated === true || rightData?.truncated === true,
      identical: !leftMissing && !rightMissing && leftText === rightText,
    };
  }, [
    descriptor.left,
    descriptor.right,
    left.data,
    left.error,
    right.data,
    right.error,
  ]);

  const leftHeaderPath =
    descriptor.left?.path ?? descriptor.old_path ?? descriptor.path;
  const rightHeaderPath = descriptor.right?.path ?? descriptor.path;
  const renamed =
    descriptor.old_path !== null && descriptor.old_path !== descriptor.path;
  const leftLabel = sideAriaLabel(descriptor.left, descriptor, leftHeaderPath);
  const rightLabel = sideAriaLabel(
    descriptor.right,
    descriptor,
    rightHeaderPath,
  );

  const ready = content.state === "ready";
  const chunkCount = ready && controller ? controller.chunkCount() : 0;
  const canJump =
    canOpen &&
    ready &&
    descriptor.right?.rev === "WORKTREE" &&
    !content.rightMissing;

  const setMode = useCallback(
    (mode: string) =>
      dispatch(setGitDiffViewerPrefs({ mode: mode as GitDiffViewMode })),
    [dispatch],
  );
  const toggleCollapse = useCallback(
    () =>
      dispatch(
        setGitDiffViewerPrefs({ collapseUnchanged: !prefs.collapseUnchanged }),
      ),
    [dispatch, prefs.collapseUnchanged],
  );
  const next = useCallback(() => {
    controller?.next();
  }, [controller]);
  const previous = useCallback(() => {
    controller?.previous();
  }, [controller]);
  const jumpToSource = useCallback(() => {
    if (!canJump || !descriptor.right) return;
    openFile({
      path: joinRepoPath(descriptor.root, descriptor.right.path),
      line: controller?.rightLineAtCursor() ?? 1,
      resolved: true,
    });
  }, [canJump, controller, descriptor.right, descriptor.root, openFile]);

  const onKeyDown = useCallback(
    (event: KeyboardEvent<HTMLDivElement>) => {
      if (event.altKey || event.ctrlKey || event.metaKey) return;
      if (event.key === "F7") {
        event.preventDefault();
        if (event.shiftKey) previous();
        else next();
      } else if (event.key === "F4" && !event.shiftKey) {
        event.preventDefault();
        jumpToSource();
      }
    },
    [jumpToSource, next, previous],
  );

  const fetching = left.isFetching || right.isFetching;
  const sideBySide = prefs.mode === "side-by-side";

  return (
    <div
      className={styles.viewer}
      data-testid="git-diff-viewer"
      onKeyDown={onKeyDown}
    >
      <div
        className={styles.toolbar}
        role="toolbar"
        aria-label="Diff viewer toolbar"
      >
        <div className={styles.toolbarGroup}>
          <IconButton
            aria-label="Previous Difference (Shift+F7)"
            title="Previous Difference (Shift+F7)"
            icon={ArrowUp}
            size="sm"
            variant="plain"
            disabled={chunkCount === 0}
            onClick={previous}
          />
          <IconButton
            aria-label="Next Difference (F7)"
            title="Next Difference (F7)"
            icon={ArrowDown}
            size="sm"
            variant="plain"
            disabled={chunkCount === 0}
            onClick={next}
          />
        </div>
        <span className={styles.count} role="status">
          {ready
            ? chunkCount === 1
              ? "1 difference"
              : `${chunkCount} differences`
            : ""}
        </span>
        <span className={styles.toolbarDivider} aria-hidden="true" />
        <SegmentedControl
          aria-label="Diff layout"
          size="sm"
          options={MODE_OPTIONS}
          value={prefs.mode}
          onValueChange={setMode}
        />
        <label className={styles.toggle}>
          <input
            type="checkbox"
            checked={prefs.collapseUnchanged}
            onChange={toggleCollapse}
          />
          Collapse unchanged
        </label>
        <span className={styles.toolbarSpacer} />
        <IconButton
          aria-label="Jump to Source (F4)"
          title="Jump to Source (F4)"
          icon={SquareArrowOutUpRight}
          size="sm"
          variant="plain"
          disabled={!canJump}
          onClick={jumpToSource}
        />
        <IconButton
          aria-label="Refresh"
          title="Refresh"
          icon={RefreshCw}
          size="sm"
          variant="plain"
          loading={fetching && content.state !== "loading"}
          onClick={refreshAll}
        />
      </div>

      {sideBySide ? (
        <div className={styles.paneHeaders}>
          <PaneHeader
            rev={sideRevLabel(descriptor.left, descriptor)}
            path={leftHeaderPath}
          />
          <PaneHeader
            rev={sideRevLabel(descriptor.right, descriptor)}
            path={
              renamed
                ? `${descriptor.old_path} → ${rightHeaderPath}`
                : rightHeaderPath
            }
          />
        </div>
      ) : (
        <div className={styles.paneHeaders}>
          <PaneHeader
            rev={`${sideRevLabel(descriptor.left, descriptor)} → ${sideRevLabel(
              descriptor.right,
              descriptor,
            )}`}
            path={
              renamed
                ? `${descriptor.old_path} → ${rightHeaderPath}`
                : rightHeaderPath
            }
          />
        </div>
      )}

      <DiffBody
        content={content}
        mode={prefs.mode}
        collapseUnchanged={prefs.collapseUnchanged}
        languagePath={descriptor.path}
        leftLabel={leftLabel}
        rightLabel={rightLabel}
        onController={setController}
      />
    </div>
  );
}

function PaneHeader({ rev, path }: { rev: string; path: string }) {
  return (
    <div className={styles.paneHeader}>
      <span className={styles.paneRev}>{rev}</span>
      <span className={styles.panePath} title={path}>
        {path}
      </span>
    </div>
  );
}

type DiffBodyProps = {
  content: DiffContent;
  mode: GitDiffViewMode;
  collapseUnchanged: boolean;
  languagePath: string;
  leftLabel: string;
  rightLabel: string;
  onController: (controller: DiffEditorsController | null) => void;
};

function DiffBody({
  content,
  mode,
  collapseUnchanged,
  languagePath,
  leftLabel,
  rightLabel,
  onController,
}: DiffBodyProps) {
  if (content.state === "loading") {
    return (
      <div className={styles.state}>
        <Spinner label="Loading diff" />
      </div>
    );
  }
  if (content.state === "error") {
    return (
      <div className={styles.state}>
        <p className={styles.errorText} role="alert">
          {content.message}
        </p>
      </div>
    );
  }
  if (content.state === "binary") {
    const leftSize = content.left?.exists
      ? formatBytes(content.left.size)
      : "—";
    const rightSize = content.right?.exists
      ? formatBytes(content.right.size)
      : "—";
    return (
      <div className={styles.state} role="status">
        <p className={styles.stateTitle}>Binary files differ</p>
        <p className={styles.stateText}>
          {leftSize} → {rightSize}
        </p>
      </div>
    );
  }

  return (
    <div className={styles.body}>
      {content.leftMissing ? (
        <div
          className={classNames(styles.banner, styles.bannerInserted)}
          role="status"
        >
          New file
        </div>
      ) : null}
      {content.rightMissing ? (
        <div
          className={classNames(styles.banner, styles.bannerDeleted)}
          role="status"
        >
          Deleted file
        </div>
      ) : null}
      {content.truncated ? (
        <div
          className={classNames(styles.banner, styles.bannerWarning)}
          role="status"
        >
          File is larger than {TRUNCATE_LIMIT_LABEL}; only the first{" "}
          {TRUNCATE_LIMIT_LABEL} are compared.
        </div>
      ) : null}
      {content.identical ? (
        <div
          className={classNames(styles.banner, styles.bannerInfo)}
          role="status"
        >
          Contents are identical
        </div>
      ) : null}
      <DiffEditorsHost
        mode={mode}
        left={content.left}
        right={content.right}
        collapseUnchanged={collapseUnchanged}
        languagePath={languagePath}
        leftLabel={leftLabel}
        rightLabel={rightLabel}
        onController={onController}
      />
    </div>
  );
}

export default GitDiffViewer;
