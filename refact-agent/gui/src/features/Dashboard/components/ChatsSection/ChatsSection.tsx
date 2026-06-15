import React, {
  useCallback,
  useDeferredValue,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  DashboardFlex,
  DashboardSpinner,
  DashboardText,
  DashboardTextField,
} from "../DashboardPrimitives";
import {
  ChevronDown,
  ChevronUp,
  MessageSquarePlus,
  Search,
} from "lucide-react";
import { CollapsePanel } from "../../../../components/shared/CollapsePanel";
import {
  Button,
  EmptyState,
  ErrorState,
  Icon,
  LoadingState,
} from "../../../../components/ui";
import { Virtuoso } from "react-virtuoso";
import {
  useAppDispatch,
  useAppSelector,
  useLoadMoreHistory,
} from "../../../../hooks";
import {
  buildHistoryTree,
  ChatHistoryItem,
  deleteChatById,
  HistoryTreeNode,
} from "../../../History/historySlice";
import { newChatAction, restoreChat } from "../../../Chat/Thread";
import { push } from "../../../Pages/pagesSlice";
import { RecentItem } from "./RecentItem";
import { getDateGroup } from "./dateUtils";
import type { DashboardBreakpoint } from "../../types";
import styles from "./ChatsSection.module.css";

type ChatsSectionProps = {
  breakpoint: DashboardBreakpoint;
  collapsed: boolean;
  projectLoading: boolean;
  onToggleCollapsed: () => void;
};

const GROUP_ORDER = ["Today", "Yesterday", "Earlier"] as const;

function treeMatchesQuery(node: HistoryTreeNode, query: string): boolean {
  if (node.title.toLowerCase().includes(query)) return true;
  if (node.mode?.toLowerCase().includes(query)) return true;
  return [...node.children, ...node.bubbleChildren].some((child) =>
    treeMatchesQuery(child, query),
  );
}

type FlatItem =
  | { type: "header"; label: string }
  | { type: "node"; node: HistoryTreeNode; depth: number };

function flattenWithExpansion(
  nodes: HistoryTreeNode[],
  expandedIds: Set<string>,
  depth: number,
): FlatItem[] {
  const out: FlatItem[] = [];
  for (const n of nodes) {
    out.push({ type: "node", node: n, depth });
    if (expandedIds.has(n.id) && n.children.length > 0) {
      out.push(...flattenWithExpansion(n.children, expandedIds, depth + 1));
    }
  }
  return out;
}

function buildFlatList(
  tree: HistoryTreeNode[],
  expandedIds: Set<string>,
): FlatItem[] {
  const groups = new Map<string, HistoryTreeNode[]>();
  for (const label of GROUP_ORDER) {
    groups.set(label, []);
  }
  for (const node of tree) {
    const group = getDateGroup(node.updatedAt);
    if (!groups.has(group)) groups.set(group, []);
    const arr = groups.get(group);
    if (arr) arr.push(node);
  }
  const items: FlatItem[] = [];
  for (const [key, nodes] of groups) {
    if (nodes.length > 0) {
      if (key !== "Today") {
        items.push({ type: "header", label: key });
      }
      items.push(...flattenWithExpansion(nodes, expandedIds, 0));
    }
  }
  return items;
}

export const ChatsSection: React.FC<ChatsSectionProps> = ({
  breakpoint,
  collapsed,
  projectLoading,
  onToggleCollapsed,
}) => {
  const dispatch = useAppDispatch();
  const isInitialLoading = useAppSelector((state) => state.history.isLoading);
  const loadError = useAppSelector((state) => state.history.loadError);
  const history = useAppSelector((state) => state.history.chats, {
    devModeChecks: { stabilityCheck: "never" },
  });
  const historyPagination = useAppSelector((state) => state.history.pagination);

  const [searchQuery, setSearchQuery] = useState("");
  const deferredQuery = useDeferredValue(searchQuery);
  const [expandedIds, setExpandedIds] = useState<Set<string>>(new Set());

  const {
    loadMore: loadMoreAsync,
    hasMore,
    isLoading: isLoadingMore,
    error: loadMoreError,
    retry: retryLoadMore,
  } = useLoadMoreHistory();

  const tree = useMemo(() => buildHistoryTree(history), [history]);

  const filteredTree = useMemo(() => {
    if (!deferredQuery.trim()) return tree;
    const q = deferredQuery.toLowerCase();
    return tree.filter((n) => treeMatchesQuery(n, q));
  }, [tree, deferredQuery]);

  const flatItems = useMemo(
    () => buildFlatList(filteredTree, expandedIds),
    [filteredTree, expandedIds],
  );
  const showLoadError = Boolean(loadError) && filteredTree.length === 0;
  const showLoading =
    !showLoadError &&
    filteredTree.length === 0 &&
    (projectLoading || isInitialLoading);

  const handleToggleExpand = useCallback((id: string) => {
    setExpandedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  const handleItemClick = useCallback(
    (node: HistoryTreeNode) => {
      const item = history[node.id] as ChatHistoryItem | undefined;
      if (item) {
        dispatch(restoreChat(item));
      } else {
        const {
          children: _,
          bubbleChildren: _bubbleChildren,
          ...historyItem
        } = node;
        dispatch(restoreChat(historyItem as ChatHistoryItem));
      }
      dispatch(push({ name: "chat" }));
    },
    [dispatch, history],
  );

  const handleDotClick = useCallback(
    (chatId: string) => {
      const item = history[chatId] as ChatHistoryItem | undefined;
      if (item) {
        dispatch(restoreChat(item));
        dispatch(push({ name: "chat" }));
      }
    },
    [dispatch, history],
  );

  const handleDelete = useCallback(
    (id: string) => {
      dispatch(deleteChatById(id));
    },
    [dispatch],
  );

  const handleNewChat = useCallback(() => {
    dispatch(newChatAction());
    dispatch(push({ name: "chat" }));
  }, [dispatch]);

  const endReachedArmedRef = useRef(true);
  const handleAtBottomChange = useCallback((atBottom: boolean) => {
    if (!atBottom) {
      endReachedArmedRef.current = true;
    }
  }, []);
  const handleEndReached = useCallback(() => {
    if (!hasMore || isLoadingMore || !endReachedArmedRef.current) return;
    endReachedArmedRef.current = false;
    void loadMoreAsync();
  }, [hasMore, isLoadingMore, loadMoreAsync]);

  const totalLabel = showLoading
    ? "Loading"
    : historyPagination.totalCount !== null
      ? `${historyPagination.totalCount} total`
      : historyPagination.hasMore
        ? `${filteredTree.length}+ total`
        : `${filteredTree.length} total`;

  return (
    <div className={styles.section} data-collapsed={collapsed || undefined}>
      <div className={styles.header}>
        <div className={styles.headerMain}>
          <Button
            variant="plain"
            size="sm"
            className={styles.headerToggle}
            onClick={onToggleCollapsed}
            aria-expanded={!collapsed}
            rightIcon={collapsed ? ChevronDown : ChevronUp}
          >
            <DashboardText
              size="1"
              weight="bold"
              tone="muted"
              className={styles.label}
            >
              CHATS
            </DashboardText>
          </Button>
          {!collapsed && (
            <DashboardTextField.Root
              size="1"
              placeholder="Search..."
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              className={styles.searchField}
            >
              <DashboardTextField.Slot>
                <Icon icon={Search} size="sm" tone="muted" />
              </DashboardTextField.Slot>
            </DashboardTextField.Root>
          )}
        </div>
        <div className={styles.headerActions}>
          <DashboardText size="1" tone="muted">
            {totalLabel}
          </DashboardText>
          <Button
            variant="ghost"
            size="sm"
            className={styles.newChatButton}
            onClick={handleNewChat}
            leftIcon={MessageSquarePlus}
          >
            New Chat
          </Button>
        </div>
      </div>

      <CollapsePanel collapsed={collapsed} className={styles.bodyPanel}>
        <div className={styles.list}>
          {showLoadError ? (
            <ErrorState
              title="Failed to load chats"
              error={loadError ?? "Refact could not load chat history."}
              className={styles.stateBlock}
            />
          ) : showLoading ? (
            <LoadingState
              label="Loading chats"
              kind="skeleton"
              className={styles.stateBlock}
            />
          ) : (
            <Virtuoso
              data={flatItems}
              endReached={handleEndReached}
              atBottomStateChange={handleAtBottomChange}
              overscan={200}
              className={styles.virtuosoList}
              itemContent={(_index, item) => {
                if (item.type === "header") {
                  return (
                    <div className={styles.groupLabel}>
                      <DashboardText
                        size="1"
                        tone="muted"
                        className={styles.groupLabelText}
                      >
                        {item.label}
                      </DashboardText>
                      <div className={styles.groupDivider} />
                    </div>
                  );
                }
                return (
                  <RecentItem
                    node={item.node}
                    depth={item.depth}
                    breakpoint={breakpoint}
                    isExpanded={expandedIds.has(item.node.id)}
                    onToggleExpand={handleToggleExpand}
                    onClick={() => handleItemClick(item.node)}
                    onDotClick={handleDotClick}
                    onDelete={handleDelete}
                  />
                );
              }}
              components={{
                Footer: () => (
                  <>
                    {isLoadingMore && (
                      <DashboardFlex justify="center" py="2">
                        <DashboardSpinner />
                      </DashboardFlex>
                    )}
                    {loadMoreError && (
                      <DashboardFlex justify="center" py="2">
                        <DashboardText
                          size="1"
                          tone="danger"
                          style={{ cursor: "pointer" }}
                          onClick={retryLoadMore}
                        >
                          Load failed — click to retry
                        </DashboardText>
                      </DashboardFlex>
                    )}
                  </>
                ),
              }}
            />
          )}
          {!showLoadError && !showLoading && filteredTree.length === 0 && (
            <EmptyState
              title={searchQuery ? "No matching chats" : "No chats yet"}
              description={
                searchQuery ? undefined : "Start a new one when you are ready."
              }
              action={
                searchQuery ? undefined : (
                  <Button
                    variant="soft"
                    size="sm"
                    onClick={handleNewChat}
                    leftIcon={MessageSquarePlus}
                  >
                    New Chat
                  </Button>
                )
              }
              className={styles.stateBlock}
            />
          )}
        </div>
      </CollapsePanel>
    </div>
  );
};
