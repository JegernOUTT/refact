import type { FetchBaseQueryError } from "@reduxjs/toolkit/query";
import {
  ChevronDown,
  ChevronRight,
  File,
  Folder,
  FolderOpen,
  RotateCw,
} from "lucide-react";
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";

import {
  Button,
  ErrorState,
  Icon,
  LoadingState,
  VirtualList,
} from "../../../components/ui";
import { useAppDispatch, useAppSelector } from "../../../hooks";
import {
  useGetFilesTreeQuery,
  type FilesTreeEntry,
} from "../../../services/refact/files";
import {
  collapseDirectory,
  expandDirectory,
  selectExpandedDirectories,
  selectFilesPanelSelectedPath,
  selectTreePath,
  openFileInFilesPanel,
  toggleDirectory,
} from "./filesPanelSlice";
import {
  flattenVisibleTree,
  parentDirectoryPath,
  type TreeChildrenByPath,
  type VisibleTreeEntry,
} from "./fileTreeModel";
import styles from "./FilesPanel.module.css";

const VIRTUALIZE_THRESHOLD = 200;

const errorStatus = (error: unknown): number | string | null => {
  const candidate = error as FetchBaseQueryError | undefined;
  return candidate?.status ?? null;
};

const DirectoryLoader = ({
  path,
  onLoaded,
}: {
  path: string;
  onLoaded: (path: string, entries: FilesTreeEntry[]) => void;
}) => {
  const { data, error, refetch } = useGetFilesTreeQuery(path);

  useEffect(() => {
    if (data) onLoaded(path, data.entries);
  }, [data, onLoaded, path]);

  if (!error) return null;

  return (
    <div className={styles.treeLoadError} role="alert">
      <span>
        {errorStatus(error) === 403
          ? "Directory blocked by privacy rules"
          : "Directory could not be loaded"}
      </span>
      <Button onClick={() => void refetch()} size="sm" variant="plain">
        Retry
      </Button>
    </div>
  );
};

const TreeRow = ({
  entry,
  expanded,
  selected,
  onActivate,
}: {
  entry: VisibleTreeEntry;
  expanded: boolean;
  selected: boolean;
  onActivate: (entry: VisibleTreeEntry) => void;
}) => {
  const isDirectory = entry.kind === "dir";
  const EntryIcon = isDirectory ? (expanded ? FolderOpen : Folder) : File;

  return (
    <button
      aria-expanded={isDirectory ? expanded : undefined}
      aria-selected={selected}
      className={styles.treeRow}
      data-selected={selected ? "true" : undefined}
      onClick={() => onActivate(entry)}
      onMouseDown={(event) => event.preventDefault()}
      role="treeitem"
      tabIndex={-1}
      type="button"
    >
      <span aria-hidden="true" className={styles.indentation}>
        {Array.from({ length: entry.depth }, (_, index) => (
          <span className={styles.indent} key={index} />
        ))}
      </span>
      <Icon
        icon={isDirectory ? (expanded ? ChevronDown : ChevronRight) : File}
        size="sm"
        tone="muted"
      />
      <Icon
        icon={EntryIcon}
        size="sm"
        tone={isDirectory ? "accent" : "muted"}
      />
      <span className={styles.treeName}>{entry.name}</span>
    </button>
  );
};

export function FileTree() {
  const dispatch = useAppDispatch();
  const treeRef = useRef<HTMLDivElement>(null);
  const expandedDirectories = useAppSelector(selectExpandedDirectories);
  const selectedPath = useAppSelector(selectFilesPanelSelectedPath);
  const [childrenByPath, setChildrenByPath] = useState<TreeChildrenByPath>({});
  const { data: root, error, isFetching, refetch } = useGetFilesTreeQuery("");
  const expandedSet = useMemo(
    () => new Set(expandedDirectories),
    [expandedDirectories],
  );
  const visibleEntries = useMemo(
    () => flattenVisibleTree(root?.entries ?? [], expandedSet, childrenByPath),
    [childrenByPath, expandedSet, root?.entries],
  );

  const handleDirectoryLoaded = useCallback(
    (path: string, entries: FilesTreeEntry[]) => {
      setChildrenByPath((current) =>
        current[path] ? current : { ...current, [path]: entries },
      );
    },
    [],
  );

  const activateEntry = useCallback(
    (entry: VisibleTreeEntry) => {
      dispatch(selectTreePath(entry.path));
      if (entry.kind === "dir") dispatch(toggleDirectory(entry.path));
      else dispatch(openFileInFilesPanel({ path: entry.path }));
      treeRef.current?.focus();
    },
    [dispatch],
  );

  const handleKeyDown = useCallback(
    (event: KeyboardEvent<HTMLDivElement>) => {
      if (visibleEntries.length === 0) return;
      const currentIndex = visibleEntries.findIndex(
        (entry) => entry.path === selectedPath,
      );
      const index = currentIndex === -1 ? 0 : currentIndex;
      const current = visibleEntries[index];
      let nextIndex = index;

      switch (event.key) {
        case "ArrowDown":
          nextIndex = Math.min(index + 1, visibleEntries.length - 1);
          break;
        case "ArrowUp":
          nextIndex = Math.max(index - 1, 0);
          break;
        case "ArrowRight":
          if (current.kind === "dir" && !expandedSet.has(current.path)) {
            dispatch(expandDirectory(current.path));
          } else if (current.kind === "dir") {
            nextIndex = Math.min(index + 1, visibleEntries.length - 1);
          }
          break;
        case "ArrowLeft":
          if (current.kind === "dir" && expandedSet.has(current.path)) {
            dispatch(collapseDirectory(current.path));
          } else {
            const parent = parentDirectoryPath(current.path);
            const parentIndex = visibleEntries.findIndex(
              (entry) => entry.path === parent,
            );
            if (parentIndex >= 0) nextIndex = parentIndex;
          }
          break;
        case "Enter":
          activateEntry(current);
          break;
        default:
          return;
      }

      event.preventDefault();
      if (nextIndex !== index || currentIndex === -1) {
        dispatch(selectTreePath(visibleEntries[nextIndex].path));
      }
    },
    [activateEntry, dispatch, expandedSet, selectedPath, visibleEntries],
  );

  if (isFetching && !root) {
    return <LoadingState label="Loading workspace files" variant="full" />;
  }

  if (error && !root) {
    const blocked = errorStatus(error) === 403;
    return (
      <ErrorState
        description={
          blocked
            ? "This directory is blocked by privacy rules."
            : "The workspace worker could not load files."
        }
        retry={
          <Button leftIcon={RotateCw} onClick={() => void refetch()} size="sm">
            Retry
          </Button>
        }
        title={blocked ? "Files blocked" : "Files unavailable"}
        variant="full"
      />
    );
  }

  const renderEntry = (entry: VisibleTreeEntry) => (
    <TreeRow
      key={entry.path}
      entry={entry}
      expanded={expandedSet.has(entry.path)}
      selected={selectedPath === entry.path}
      onActivate={activateEntry}
    />
  );

  return (
    <div
      aria-label="Workspace files"
      className={styles.tree}
      onKeyDown={handleKeyDown}
      ref={treeRef}
      role="tree"
      tabIndex={0}
    >
      {expandedDirectories.map((path) => (
        <DirectoryLoader
          key={path}
          path={path}
          onLoaded={handleDirectoryLoaded}
        />
      ))}
      {root?.truncated ? (
        <div className={styles.treeNotice}>Directory list truncated</div>
      ) : null}
      {visibleEntries.length > VIRTUALIZE_THRESHOLD ? (
        <VirtualList
          className={styles.virtualTree}
          getItemKey={(entry) => entry.path}
          height="100%"
          items={visibleEntries}
          renderItem={renderEntry}
        />
      ) : (
        <div className={styles.treeRows}>{visibleEntries.map(renderEntry)}</div>
      )}
    </div>
  );
}
