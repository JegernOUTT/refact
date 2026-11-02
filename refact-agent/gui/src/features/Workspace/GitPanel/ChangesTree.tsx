import { ContextMenu } from "@radix-ui/themes";
import { ChevronDown, ChevronRight, Folder, FolderOpen } from "lucide-react";
import {
  useCallback,
  useEffect,
  useId,
  useRef,
  type KeyboardEvent,
  type MouseEvent,
  type ReactNode,
} from "react";

import { Checkbox } from "../../../components/Checkbox";
import {
  Icon,
  VirtualList,
  type VirtualListHandle,
} from "../../../components/ui";
import { fileTypeIcon } from "../FilesPanel/fileTypeIcon";
import {
  isCheckable,
  nodeCheckState,
  parentRowIndex,
  type ChangesTreeEntry,
  type ChangesTreeNode,
  type InclusionState,
  type VisibleChangesRow,
} from "./changesTreeModel";
import styles from "./GitDock.module.css";

const VIRTUALIZE_THRESHOLD = 200;

type EntryTone =
  | "modified"
  | "added"
  | "deleted"
  | "renamed"
  | "untracked"
  | "conflict";

function entryTone(entry: ChangesTreeEntry): EntryTone {
  if (entry.kind === "untracked") return "untracked";
  if (entry.kind === "conflict") return "conflict";
  switch (entry.status) {
    case "ADDED":
      return "added";
    case "DELETED":
      return "deleted";
    case "RENAMED":
      return "renamed";
    default:
      return "modified";
  }
}

function nodeLabel(node: ChangesTreeNode): string {
  return node.type === "file" ? node.entry.name : node.label;
}

function nodePathLabel(node: ChangesTreeNode): string {
  if (node.type === "file") return node.entry.path;
  if (node.type === "dir") return node.path;
  return node.label;
}

function fileCountLabel(count: number): string {
  return `${count} ${count === 1 ? "file" : "files"}`;
}

export type ChangesTreeProps = {
  rows: VisibleChangesRow[];
  collapsed: ReadonlySet<string>;
  selectedId: string | null;
  inclusion: InclusionState;
  groupByDirectory: boolean;
  onSelect: (id: string) => void;
  onSetExpanded: (id: string, expanded: boolean) => void;
  onToggleInclude: (node: ChangesTreeNode) => void;
  onShowDiff: (entry: ChangesTreeEntry) => void;
  renderContextMenu: (node: ChangesTreeNode) => ReactNode;
  /** Read-only trees (commit details) hide the include checkboxes. */
  checkable?: boolean;
  label?: string;
  renderEntryMeta?: (entry: ChangesTreeEntry) => ReactNode;
};

type RowProps = {
  row: VisibleChangesRow;
  rowId: string;
  expanded: boolean;
  selected: boolean;
  inclusion: InclusionState;
  groupByDirectory: boolean;
  onSelect: (id: string) => void;
  onSetExpanded: (id: string, expanded: boolean) => void;
  onToggleInclude: (node: ChangesTreeNode) => void;
  onShowDiff: (entry: ChangesTreeEntry) => void;
  renderContextMenu: (node: ChangesTreeNode) => ReactNode;
  checkable: boolean;
  renderEntryMeta?: (entry: ChangesTreeEntry) => ReactNode;
};

function ChangesRow({
  row,
  rowId,
  expanded,
  selected,
  inclusion,
  groupByDirectory,
  onSelect,
  onSetExpanded,
  onToggleInclude,
  onShowDiff,
  renderContextMenu,
  checkable: treeCheckable,
  renderEntryMeta,
}: RowProps) {
  const { node, depth } = row;
  const isFile = node.type === "file";
  const checkable = treeCheckable && isCheckable(node);
  const checkState = checkable ? nodeCheckState(node, inclusion) : false;
  const tone = isFile ? entryTone(node.entry) : undefined;
  const EntryIcon = isFile
    ? fileTypeIcon(node.entry.name)
    : node.type === "dir"
      ? expanded
        ? FolderOpen
        : Folder
      : null;
  const renamedHint =
    isFile && node.entry.status === "RENAMED" && node.entry.oldPath
      ? `renamed from ${node.entry.oldPath}`
      : null;
  const flatDirectory =
    isFile && !groupByDirectory && node.entry.directory
      ? node.entry.directory
      : null;

  const handleChevron = (event: MouseEvent) => {
    event.stopPropagation();
    onSelect(node.id);
    onSetExpanded(node.id, !expanded);
  };

  return (
    <ContextMenu.Root modal={false}>
      <ContextMenu.Trigger>
        <div
          aria-expanded={isFile ? undefined : expanded}
          aria-label={nodePathLabel(node)}
          aria-level={depth + 1}
          aria-selected={selected}
          className={styles.treeRow}
          data-node-id={node.id}
          data-node-type={node.type}
          data-selected={selected ? "true" : undefined}
          id={rowId}
          onClick={() => onSelect(node.id)}
          onContextMenu={() => onSelect(node.id)}
          onDoubleClick={() => {
            if (isFile) onShowDiff(node.entry);
          }}
          onMouseDown={(event) => {
            if (event.detail > 1) event.preventDefault();
          }}
          role="treeitem"
          title={
            renamedHint
              ? `${nodePathLabel(node)} (${renamedHint})`
              : nodePathLabel(node)
          }
        >
          <span aria-hidden="true" className={styles.indentation}>
            {Array.from({ length: depth }, (_, index) => (
              <span className={styles.indent} key={index} />
            ))}
          </span>
          <span className={styles.chevronSlot}>
            {isFile ? null : (
              <button
                aria-hidden="true"
                className={styles.chevronButton}
                onClick={handleChevron}
                tabIndex={-1}
                type="button"
              >
                <Icon
                  icon={expanded ? ChevronDown : ChevronRight}
                  size="sm"
                  tone="muted"
                />
              </button>
            )}
          </span>
          {treeCheckable ? (
            <span
              className={styles.checkSlot}
              onClick={(event) => event.stopPropagation()}
              onDoubleClick={(event) => event.stopPropagation()}
            >
              {checkable ? (
                <Checkbox
                  aria-label={`Include ${nodePathLabel(node)} in commit`}
                  checked={checkState}
                  onCheckedChange={() => {
                    onSelect(node.id);
                    onToggleInclude(node);
                  }}
                  tabIndex={-1}
                />
              ) : null}
            </span>
          ) : null}
          {EntryIcon ? (
            <Icon
              icon={EntryIcon}
              size="sm"
              tone={node.type === "dir" ? "accent" : "muted"}
            />
          ) : null}
          <span
            className={styles.treeName}
            data-group={node.type === "group" ? "true" : undefined}
            data-tone={tone}
          >
            {nodeLabel(node)}
          </span>
          {flatDirectory ? (
            <span className={styles.treeHint}>{flatDirectory}</span>
          ) : null}
          {renamedHint ? (
            <span className={styles.treeHint}>{renamedHint}</span>
          ) : null}
          {isFile ? (
            renderEntryMeta?.(node.entry)
          ) : (
            <span className={styles.treeCount}>
              {fileCountLabel(node.fileCount)}
            </span>
          )}
        </div>
      </ContextMenu.Trigger>
      <ContextMenu.Content size="1">
        {renderContextMenu(node)}
      </ContextMenu.Content>
    </ContextMenu.Root>
  );
}

export function ChangesTree({
  rows,
  collapsed,
  selectedId,
  inclusion,
  groupByDirectory,
  onSelect,
  onSetExpanded,
  onToggleInclude,
  onShowDiff,
  renderContextMenu,
  checkable = true,
  label = "Changes",
  renderEntryMeta,
}: ChangesTreeProps) {
  const treeId = useId();
  const treeRef = useRef<HTMLDivElement>(null);
  const listRef = useRef<VirtualListHandle>(null);
  const virtualized = rows.length > VIRTUALIZE_THRESHOLD;
  const rowId = (id: string) => `${treeId}-row-${encodeURIComponent(id)}`;
  const selectedIndex = rows.findIndex((row) => row.node.id === selectedId);

  useEffect(() => {
    if (selectedIndex < 0) return;
    if (virtualized) {
      listRef.current?.scrollIntoView({ index: selectedIndex });
      return;
    }
    const element = treeRef.current?.querySelector<HTMLElement>(
      `[data-node-id="${CSS.escape(rows[selectedIndex].node.id)}"]`,
    );
    if (typeof element?.scrollIntoView === "function") {
      element.scrollIntoView({ block: "nearest" });
    }
  }, [rows, selectedIndex, virtualized]);

  const openContextMenu = useCallback((id: string) => {
    const element = treeRef.current?.querySelector<HTMLElement>(
      `[data-node-id="${CSS.escape(id)}"]`,
    );
    if (!element) return;
    const rect = element.getBoundingClientRect();
    element.dispatchEvent(
      new window.MouseEvent("contextmenu", {
        bubbles: true,
        cancelable: true,
        clientX: rect.left + rect.height,
        clientY: rect.bottom,
      }),
    );
  }, []);

  const handleKeyDown = useCallback(
    (event: KeyboardEvent<HTMLDivElement>) => {
      if (rows.length === 0) return;
      const index = selectedIndex === -1 ? 0 : selectedIndex;
      const current = rows[index].node;
      const isContainer = current.type !== "file";
      const isExpanded = isContainer && !collapsed.has(current.id);
      let nextIndex = index;
      const modifier = event.ctrlKey || event.metaKey;

      if (modifier && event.key.toLowerCase() === "d") {
        if (current.type === "file") onShowDiff(current.entry);
        event.preventDefault();
        return;
      }
      if (
        event.key === "ContextMenu" ||
        (event.shiftKey && event.key === "F10")
      ) {
        event.preventDefault();
        onSelect(current.id);
        openContextMenu(current.id);
        return;
      }
      if (modifier || event.altKey) return;
      if (
        selectedIndex === -1 &&
        ["ArrowDown", "ArrowUp", "ArrowLeft", "ArrowRight"].includes(event.key)
      ) {
        event.preventDefault();
        onSelect(current.id);
        return;
      }

      switch (event.key) {
        case "ArrowDown":
          nextIndex = Math.min(index + 1, rows.length - 1);
          break;
        case "ArrowUp":
          nextIndex = Math.max(index - 1, 0);
          break;
        case "Home":
          nextIndex = 0;
          break;
        case "End":
          nextIndex = rows.length - 1;
          break;
        case "ArrowRight":
          if (isContainer && !isExpanded) onSetExpanded(current.id, true);
          else if (isContainer)
            nextIndex = Math.min(index + 1, rows.length - 1);
          break;
        case "ArrowLeft":
          if (isExpanded) onSetExpanded(current.id, false);
          else {
            const parent = parentRowIndex(rows, index);
            if (parent >= 0) nextIndex = parent;
          }
          break;
        case " ":
          if (checkable && isCheckable(current)) onToggleInclude(current);
          break;
        case "Enter":
          if (current.type === "file") onShowDiff(current.entry);
          else onSetExpanded(current.id, !isExpanded);
          break;
        default:
          return;
      }
      event.preventDefault();
      if (nextIndex !== index || selectedIndex === -1) {
        onSelect(rows[nextIndex].node.id);
      }
    },
    [
      checkable,
      collapsed,
      onSelect,
      onSetExpanded,
      onShowDiff,
      onToggleInclude,
      openContextMenu,
      rows,
      selectedIndex,
    ],
  );

  const renderRow = (row: VisibleChangesRow) => (
    <ChangesRow
      expanded={row.node.type !== "file" && !collapsed.has(row.node.id)}
      groupByDirectory={groupByDirectory}
      inclusion={inclusion}
      key={row.node.id}
      onSelect={(id) => {
        onSelect(id);
        treeRef.current?.focus();
      }}
      onSetExpanded={onSetExpanded}
      onShowDiff={onShowDiff}
      onToggleInclude={onToggleInclude}
      checkable={checkable}
      renderContextMenu={renderContextMenu}
      renderEntryMeta={renderEntryMeta}
      row={row}
      rowId={rowId(row.node.id)}
      selected={row.node.id === selectedId}
    />
  );

  return (
    <div
      aria-activedescendant={
        selectedIndex >= 0 ? rowId(rows[selectedIndex].node.id) : undefined
      }
      aria-label={label}
      className={styles.tree}
      onKeyDown={handleKeyDown}
      ref={treeRef}
      role="tree"
      tabIndex={0}
    >
      {virtualized ? (
        <VirtualList
          className={styles.virtualTree}
          density="compact"
          getItemKey={(row) => row.node.id}
          height="100%"
          items={rows}
          listRef={listRef}
          renderItem={renderRow}
        />
      ) : (
        <div className={styles.treeRows}>{rows.map(renderRow)}</div>
      )}
    </div>
  );
}
