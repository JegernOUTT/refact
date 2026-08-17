import type { FetchBaseQueryError } from "@reduxjs/toolkit/query";
import { ContextMenu } from "@radix-ui/themes";
import {
  ChevronDown,
  ChevronRight,
  Folder,
  FolderOpen,
  RotateCw,
} from "lucide-react";
import {
  useCallback,
  useEffect,
  useId,
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
  StatusDot,
  Tooltip,
  VirtualList,
  type StatusDotStatus,
} from "../../../components/ui";
import { useAppDispatch, useAppSelector } from "../../../hooks";
import {
  filesApi,
  useGetFilesTreeQuery,
  type FilesTreeEntry,
} from "../../../services/refact/files";
import {
  type PrivacyDestination,
  type ResolvedPrivacyZone,
  type PrivacyZone,
  useGetPrivacyPolicyQuery,
  useUpdatePrivacyPolicyMutation,
} from "../../../services/refact/privacy";
import {
  collapseDirectory,
  expandDirectory,
  isPathWithinWorkspaceRoots,
  resetFileTree,
  selectExpandedDirectories,
  selectFilesPanelSelectedPath,
  selectShowIgnored,
  selectTreePath,
  openFileInFilesPanel,
  toggleDirectory,
} from "./filesPanelSlice";
import { selectFocusedChatWorkspaceRoot } from "../workspaceSlice";
import {
  flattenVisibleTree,
  movePathToPrivacyZone,
  parentDirectoryPath,
  type TreeChildrenByPath,
  type VisibleTreeEntry,
} from "./fileTreeModel";
import { fileTypeIcon } from "./fileTypeIcon";
import styles from "./FilesPanel.module.css";

const VIRTUALIZE_THRESHOLD = 200;

const zoneStatus = (zone: ResolvedPrivacyZone): StatusDotStatus => {
  if (zone.name === "blocked") return "error";
  if (zone.name === "secrets") return "warning";
  if (zone.name === "normal") return "idle";
  return "running";
};

const zoneTooltip = (
  zone: ResolvedPrivacyZone,
  destinations: PrivacyDestination[],
): string => {
  const allowed = zone.send_to.includes("*")
    ? "All destinations"
    : zone.send_to.length === 0
      ? "None"
      : zone.send_to
          .map(
            (id) =>
              destinations.find((destination) => destination.id === id)
                ?.display_name ?? id,
          )
          .join(", ");
  return `Zone: ${zone.name}. Allowed destinations: ${allowed}.`;
};

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
  rowId,
  onActivate,
  privacyZone,
  privacyDestinations,
  privacyZones,
  privacySaving,
  onMoveToZone,
}: {
  entry: VisibleTreeEntry;
  expanded: boolean;
  selected: boolean;
  rowId: string;
  onActivate: (entry: VisibleTreeEntry) => void;
  privacyZone: ResolvedPrivacyZone | null;
  privacyDestinations: PrivacyDestination[];
  privacyZones: PrivacyZone[];
  privacySaving: boolean;
  onMoveToZone: (entry: VisibleTreeEntry, zoneName: string) => void;
}) => {
  const isDirectory = entry.kind === "dir";
  const EntryIcon = isDirectory
    ? expanded
      ? FolderOpen
      : Folder
    : fileTypeIcon(entry.name);
  const movableZones = isDirectory ? [] : privacyZones;

  const row = (
    <button
      aria-expanded={isDirectory ? expanded : undefined}
      aria-selected={selected}
      className={styles.treeRow}
      data-ignored={entry.ignored ? "true" : undefined}
      data-selected={selected ? "true" : undefined}
      onClick={() => onActivate(entry)}
      onMouseDown={(event) => event.preventDefault()}
      id={rowId}
      role="treeitem"
      tabIndex={-1}
      type="button"
    >
      <span aria-hidden="true" className={styles.indentation}>
        {Array.from({ length: entry.depth }, (_, index) => (
          <span className={styles.indent} key={index} />
        ))}
      </span>
      <span
        aria-hidden="true"
        className={styles.treeChevronSlot}
        data-testid="tree-chevron-slot"
      >
        {isDirectory ? (
          <Icon
            icon={expanded ? ChevronDown : ChevronRight}
            size="sm"
            tone="muted"
          />
        ) : null}
      </span>
      <Icon
        icon={EntryIcon}
        size="sm"
        tone={isDirectory ? "accent" : "muted"}
      />
      <span className={styles.treeName}>{entry.name}</span>
      {privacyZone ? (
        <Tooltip
          content={zoneTooltip(privacyZone, privacyDestinations)}
          delayDuration={0}
        >
          <span className={styles.zoneDot}>
            <StatusDot
              aria-hidden="true"
              data-zone={privacyZone.name}
              size="small"
              status={zoneStatus(privacyZone)}
            />
          </span>
        </Tooltip>
      ) : null}
    </button>
  );

  if (!privacyZone || movableZones.length === 0) return row;

  return (
    <ContextMenu.Root>
      <ContextMenu.Trigger>{row}</ContextMenu.Trigger>
      <ContextMenu.Content>
        <ContextMenu.Sub>
          <ContextMenu.SubTrigger>Move to zone</ContextMenu.SubTrigger>
          <ContextMenu.SubContent>
            {movableZones.map((zone) => (
              <ContextMenu.Item
                disabled={privacySaving || zone.name === privacyZone.name}
                key={zone.name}
                onSelect={() => onMoveToZone(entry, zone.name)}
              >
                {zone.name}
              </ContextMenu.Item>
            ))}
          </ContextMenu.SubContent>
        </ContextMenu.Sub>
      </ContextMenu.Content>
    </ContextMenu.Root>
  );
};

export function FileTree() {
  const dispatch = useAppDispatch();
  const treeId = useId();
  const treeRef = useRef<HTMLDivElement>(null);
  const expandedDirectories = useAppSelector(selectExpandedDirectories);
  const selectedPath = useAppSelector(selectFilesPanelSelectedPath);
  const showIgnored = useAppSelector(selectShowIgnored);
  const contextRoot = useAppSelector(selectFocusedChatWorkspaceRoot);
  const privacyPolicyQuery = useGetPrivacyPolicyQuery(undefined);
  const [updatePrivacyPolicy, privacyUpdateState] =
    useUpdatePrivacyPolicyMutation();
  const [loadedContextRoot, setLoadedContextRoot] = useState(contextRoot);
  const [childrenByPath, setChildrenByPath] = useState<TreeChildrenByPath>({});
  const {
    data: root,
    error,
    isFetching,
    refetch,
  } = useGetFilesTreeQuery(contextRoot);
  const contextExpandedDirectories = useMemo(
    () =>
      contextRoot
        ? expandedDirectories.filter((path) =>
            isPathWithinWorkspaceRoots(path, [contextRoot]),
          )
        : expandedDirectories,
    [contextRoot, expandedDirectories],
  );
  const expandedSet = useMemo(
    () => new Set(contextExpandedDirectories),
    [contextExpandedDirectories],
  );
  const visibleEntries = useMemo(
    () =>
      loadedContextRoot === contextRoot
        ? flattenVisibleTree(
            root?.entries ?? [],
            expandedSet,
            childrenByPath,
            showIgnored,
          )
        : [],
    [
      childrenByPath,
      contextRoot,
      expandedSet,
      loadedContextRoot,
      root?.entries,
      showIgnored,
    ],
  );

  const handleDirectoryLoaded = useCallback(
    (path: string, entries: FilesTreeEntry[]) => {
      setChildrenByPath((current) => ({ ...current, [path]: entries }));
    },
    [],
  );

  useEffect(() => {
    if (loadedContextRoot === contextRoot) return;
    setChildrenByPath({});
    dispatch(resetFileTree());
    setLoadedContextRoot(contextRoot);
  }, [contextRoot, dispatch, loadedContextRoot]);

  const activateEntry = useCallback(
    (entry: VisibleTreeEntry) => {
      dispatch(selectTreePath(entry.path));
      if (entry.kind === "dir") dispatch(toggleDirectory(entry.path));
      else dispatch(openFileInFilesPanel({ path: entry.path }));
      treeRef.current?.focus();
    },
    [dispatch],
  );

  const moveEntryToZone = useCallback(
    (entry: VisibleTreeEntry, zoneName: string) => {
      const policy = privacyPolicyQuery.data?.policy;
      if (!policy) return;
      void updatePrivacyPolicy(
        movePathToPrivacyZone(policy, entry.path, zoneName),
      )
        .unwrap()
        .then(() => dispatch(filesApi.util.invalidateTags(["Tree"])))
        .catch(() => undefined);
    },
    [dispatch, privacyPolicyQuery.data?.policy, updatePrivacyPolicy],
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

  const rowId = (path: string) => `${treeId}-item-${encodeURIComponent(path)}`;
  const activeDescendant = visibleEntries.some(
    (entry) => entry.path === selectedPath,
  )
    ? rowId(selectedPath ?? "")
    : undefined;
  const renderEntry = (entry: VisibleTreeEntry) => (
    <TreeRow
      key={entry.path}
      entry={entry}
      expanded={expandedSet.has(entry.path)}
      selected={selectedPath === entry.path}
      rowId={rowId(entry.path)}
      onActivate={activateEntry}
      privacyZone={entry.privacy_zone}
      privacyDestinations={privacyPolicyQuery.data?.destinations ?? []}
      privacyZones={privacyPolicyQuery.data?.policy.zones ?? []}
      privacySaving={privacyUpdateState.isLoading}
      onMoveToZone={moveEntryToZone}
    />
  );

  return (
    <div
      aria-activedescendant={activeDescendant}
      aria-label="Workspace files"
      className={styles.tree}
      onKeyDown={handleKeyDown}
      ref={treeRef}
      role="tree"
      tabIndex={0}
    >
      {contextExpandedDirectories.map((path) => (
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
