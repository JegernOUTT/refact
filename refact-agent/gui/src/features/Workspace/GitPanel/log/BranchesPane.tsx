import { ContextMenu } from "@radix-ui/themes";
import {
  ChevronDown,
  ChevronRight,
  Cloud,
  Ellipsis,
  Folder,
  FolderOpen,
  GitBranch,
  Tag,
  Target,
  type LucideIcon,
} from "lucide-react";
import {
  useCallback,
  useId,
  useMemo,
  useState,
  type KeyboardEvent,
} from "react";

import { Icon, IconButton, Menu } from "../../../../components/ui";
import type { GitBranchesRoot } from "../../../../services/refact/gitRead";
import {
  ContextMenuActions,
  MenuActions,
  type GitMenuAction,
} from "../branches/menuActions";
import branchStyles from "../branches/Branches.module.css";
import {
  HEAD_NODE_ID,
  TAGS_GROUP_ID,
  buildBranchTree,
  flattenBranchTree,
  refNodeId,
  type BranchRef,
  type BranchTreeNode,
  type VisibleBranchRow,
} from "./branchesTreeModel";
import type { BranchFilter } from "./logFilters";
import styles from "./GitLog.module.css";

export type BranchActionsProvider = (ref: BranchRef) => GitMenuAction[];

export type BranchesPaneProps = {
  data: GitBranchesRoot | undefined;
  filter: BranchFilter;
  onNavigate: (filter: BranchFilter) => void;
  /** Actions for a ref (context menu and hover "…" menu); omitted means none. */
  branchActions?: BranchActionsProvider;
};

const DEFAULT_COLLAPSED: ReadonlySet<string> = new Set([TAGS_GROUP_ID]);

function nodeIcon(node: BranchTreeNode, expanded: boolean): LucideIcon {
  if (node.type === "head") return Target;
  if (node.type === "group") {
    if (node.id === TAGS_GROUP_ID) return Tag;
    if (node.id === "group:remote") return Cloud;
    return expanded ? FolderOpen : Folder;
  }
  if (node.ref.kind === "tag") return Tag;
  return GitBranch;
}

function selectedNodeId(filter: BranchFilter, data?: GitBranchesRoot) {
  if (filter.kind === "ref") {
    if (data?.branches.some((branch) => branch.name === filter.name)) {
      return refNodeId("local", filter.name);
    }
    if (data?.tags?.some((tag) => tag.name === filter.name)) {
      return refNodeId("tag", filter.name);
    }
    return refNodeId("remote", filter.name);
  }
  return filter.kind === "head" ? HEAD_NODE_ID : null;
}

function trackingHint(ref: BranchRef): string | null {
  if (ref.upstreamGone) return "gone";
  const parts: string[] = [];
  if (ref.ahead) parts.push(`↑${ref.ahead}`);
  if (ref.behind) parts.push(`↓${ref.behind}`);
  return parts.length > 0 ? parts.join(" ") : null;
}

function rowTitle(node: BranchTreeNode): string {
  if (node.type === "head")
    return node.detail ? `HEAD (${node.detail})` : "HEAD";
  if (node.type === "group") return node.label;
  const upstream = node.ref.upstream ? ` → ${node.ref.upstream}` : "";
  return `${node.ref.name}${upstream}`;
}

type RowProps = {
  row: VisibleBranchRow;
  rowId: string;
  expanded: boolean;
  selected: boolean;
  active: boolean;
  onActivate: (node: BranchTreeNode) => void;
  onToggle: (id: string) => void;
  branchActions?: BranchActionsProvider;
  menuOpen: boolean;
  onMenuOpenChange: (open: boolean) => void;
};

function BranchRow({
  row,
  rowId,
  expanded,
  selected,
  active,
  onActivate,
  onToggle,
  branchActions,
  menuOpen,
  onMenuOpenChange,
}: RowProps) {
  const { node, depth } = row;
  const isGroup = node.type === "group";
  const hint = node.type === "ref" ? trackingHint(node.ref) : null;
  const actions =
    node.type === "ref" && branchActions ? branchActions(node.ref) : null;
  const content = (
    <div
      aria-expanded={isGroup ? expanded : undefined}
      aria-label={node.type === "ref" ? node.ref.name : undefined}
      aria-level={depth + 1}
      aria-selected={selected}
      className={
        actions
          ? `${styles.treeRow} ${branchStyles.rowActionsHost}`
          : styles.treeRow
      }
      data-active={active ? "true" : undefined}
      data-node-id={node.id}
      data-selected={selected ? "true" : undefined}
      id={rowId}
      onClick={() => onActivate(node)}
      role="treeitem"
      title={rowTitle(node)}
    >
      <span aria-hidden="true" className={styles.indentation}>
        {Array.from({ length: depth }, (_, index) => (
          <span className={styles.indent} key={index} />
        ))}
      </span>
      <span className={styles.chevronSlot}>
        {isGroup ? (
          <button
            aria-hidden="true"
            className={styles.chevronButton}
            onClick={(event) => {
              event.stopPropagation();
              onToggle(node.id);
            }}
            tabIndex={-1}
            type="button"
          >
            <Icon
              icon={expanded ? ChevronDown : ChevronRight}
              size="sm"
              tone="muted"
            />
          </button>
        ) : null}
      </span>
      <Icon
        icon={nodeIcon(node, expanded)}
        size="sm"
        tone={
          node.type === "head" || (node.type === "ref" && node.ref.current)
            ? "accent"
            : "muted"
        }
      />
      <span
        className={styles.treeName}
        data-current={
          node.type === "ref" && node.ref.current ? "true" : undefined
        }
      >
        {node.type === "ref" ? node.ref.label : node.label}
      </span>
      {node.type === "head" && node.detail ? (
        <span className={styles.treeHint}>{node.detail}</span>
      ) : null}
      {hint ? <span className={styles.treeHint}>{hint}</span> : null}
      {actions && node.type === "ref" ? (
        <Menu onOpenChange={onMenuOpenChange} open={menuOpen}>
          <Menu.Trigger asChild>
            <IconButton
              aria-label={`Actions for ${node.ref.name}`}
              className={branchStyles.rowActions}
              icon={Ellipsis}
              onClick={(event) => event.stopPropagation()}
              size="sm"
              tabIndex={-1}
              variant="ghost"
            />
          </Menu.Trigger>
          <Menu.Content align="end">
            <MenuActions actions={actions} />
          </Menu.Content>
        </Menu>
      ) : null}
    </div>
  );

  if (!actions) return content;
  return (
    <ContextMenu.Root modal={false}>
      <ContextMenu.Trigger>{content}</ContextMenu.Trigger>
      <ContextMenu.Content size="1">
        <ContextMenuActions actions={actions} />
      </ContextMenu.Content>
    </ContextMenu.Root>
  );
}

export function BranchesPane({
  data,
  filter,
  onNavigate,
  branchActions,
}: BranchesPaneProps) {
  const treeId = useId();
  const [search, setSearch] = useState("");
  const [menuFor, setMenuFor] = useState<string | null>(null);
  const [collapsed, setCollapsed] = useState(DEFAULT_COLLAPSED);
  const [cursor, setCursor] = useState<string | null>(null);
  const tree = useMemo(() => (data ? buildBranchTree(data) : []), [data]);
  const rows = useMemo(
    () => flattenBranchTree(tree, collapsed, search),
    [collapsed, search, tree],
  );
  const selectedId = selectedNodeId(filter, data);
  const activeId = cursor ?? selectedId;
  const activeIndex = rows.findIndex((row) => row.node.id === activeId);
  const rowId = (id: string) => `${treeId}-${encodeURIComponent(id)}`;

  const toggle = useCallback((id: string) => {
    setCollapsed((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  const activate = useCallback(
    (node: BranchTreeNode) => {
      setCursor(node.id);
      if (node.type === "group") toggle(node.id);
      else if (node.type === "head") onNavigate({ kind: "all" });
      else onNavigate({ kind: "ref", name: node.ref.name });
    },
    [onNavigate, toggle],
  );

  const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (rows.length === 0) return;
    const index = Math.max(activeIndex, 0);
    const current = rows[index].node;
    const isGroup = current.type === "group";
    const expanded = isGroup && !collapsed.has(current.id);
    let next = index;
    if (
      event.key === "ContextMenu" ||
      (event.key === "F10" && event.shiftKey)
    ) {
      if (current.type === "ref" && branchActions) {
        event.preventDefault();
        setMenuFor(current.id);
      }
      return;
    }
    switch (event.key) {
      case "ArrowDown":
        next = Math.min(index + 1, rows.length - 1);
        break;
      case "ArrowUp":
        next = Math.max(index - 1, 0);
        break;
      case "Home":
        next = 0;
        break;
      case "End":
        next = rows.length - 1;
        break;
      case "ArrowRight":
        if (isGroup && !expanded) toggle(current.id);
        break;
      case "ArrowLeft":
        if (expanded) toggle(current.id);
        else {
          const parent = rows.findIndex(
            (row) => row.node.id === rows[index].parentId,
          );
          if (parent >= 0) next = parent;
        }
        break;
      case "Enter":
      case " ":
        activate(current);
        break;
      default:
        return;
    }
    event.preventDefault();
    setCursor(rows[next].node.id);
  };

  return (
    <div className={styles.branchesPane}>
      <input
        aria-label="Search branches"
        className={styles.searchInput}
        onChange={(event) => setSearch(event.currentTarget.value)}
        placeholder="Search branches"
        type="search"
        value={search}
      />
      <div
        aria-activedescendant={
          activeIndex >= 0 ? rowId(rows[activeIndex].node.id) : undefined
        }
        aria-label="Branches"
        className={styles.tree}
        onKeyDown={handleKeyDown}
        role="tree"
        tabIndex={0}
      >
        {rows.map((row) => (
          <BranchRow
            expanded={row.node.type === "group" && !collapsed.has(row.node.id)}
            key={row.node.id}
            onActivate={activate}
            branchActions={branchActions}
            menuOpen={menuFor === row.node.id}
            onMenuOpenChange={(open) => setMenuFor(open ? row.node.id : null)}
            onToggle={toggle}
            row={row}
            rowId={rowId(row.node.id)}
            active={row.node.id === activeId}
            selected={row.node.id === selectedId}
          />
        ))}
      </div>
    </div>
  );
}
