import React, { useCallback, useDeferredValue, useMemo, useState } from "react";
import { Flex, Skeleton, Spinner, Text, TextField } from "@radix-ui/themes";
import { MagnifyingGlassIcon, ChevronDownIcon, ChevronUpIcon, PlusIcon } from "@radix-ui/react-icons";
import { Virtuoso } from "react-virtuoso";
import { useAppDispatch, useAppSelector, useLoadMoreHistory } from "../../../../hooks";
import {
  buildHistoryTree,
  ChatHistoryItem,
  deleteChatById,
  HistoryTreeNode,
  updateChatTitleById,
} from "../../../History/historySlice";
import { newChatAction, restoreChat } from "../../../Chat/Thread";
import { push } from "../../../Pages/pagesSlice";
import { RecentItem, getDateGroup } from "./RecentItem";
import type { DashboardBreakpoint } from "../../types";
import styles from "./ChatsSection.module.css";

type ChatsSectionProps = {
  breakpoint: DashboardBreakpoint;
  expanded: boolean;
  onToggleExpand: () => void;
};

const GROUP_ORDER = ["Today", "Yesterday", "Earlier"] as const;

const DOT_LEGEND: { color: string; label: string }[] = [
  { color: "var(--blue-8)", label: "Chat" },
  { color: "var(--green-8)", label: "Subagent / Handoff" },
  { color: "var(--amber-8)", label: "Fork / Branch" },
  { color: "var(--blue-9)", label: "Active" },
  { color: "var(--green-9)", label: "Done" },
];

function treeMatchesQuery(node: HistoryTreeNode, query: string): boolean {
  if (node.title.toLowerCase().includes(query)) return true;
  if (node.mode?.toLowerCase().includes(query)) return true;
  return node.children.some((child) => treeMatchesQuery(child, query));
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
      items.push({ type: "header", label: key });
      items.push(...flattenWithExpansion(nodes, expandedIds, 0));
    }
  }
  return items;
}

export const ChatsSection: React.FC<ChatsSectionProps> = ({
  breakpoint,
  expanded,
  onToggleExpand,
}) => {
  const dispatch = useAppDispatch();
  const isInitialLoading = useAppSelector((state) => state.history.isLoading);
  const history = useAppSelector((state) => state.history.chats, {
    devModeChecks: { stabilityCheck: "never" },
  });

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
        const { children: _, ...historyItem } = node;
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

  const handleRename = useCallback(
    (id: string, newTitle: string) => {
      dispatch(updateChatTitleById({ chatId: id, newTitle }));
    },
    [dispatch],
  );

  const handleNewChat = useCallback(() => {
    dispatch(newChatAction());
    dispatch(push({ name: "chat" }));
  }, [dispatch]);

  const handleEndReached = useCallback(() => {
    if (hasMore && !isLoadingMore) {
      void loadMoreAsync();
    }
  }, [hasMore, isLoadingMore, loadMoreAsync]);

  return (
    <div className={styles.section}>
      <div className={styles.header}>
        <button
          type="button"
          className={styles.headerToggle}
          onClick={onToggleExpand}
          aria-expanded={expanded}
        >
          <Text size="1" weight="bold" color="gray" className={styles.label}>
            CHATS
          </Text>
          <Flex align="center" gap="1">
            {!expanded && (
              <Text size="1" color="gray">
                {filteredTree.length} total
              </Text>
            )}
            {expanded ? (
              <ChevronUpIcon width={12} height={12} color="var(--gray-9)" />
            ) : (
              <ChevronDownIcon width={12} height={12} color="var(--gray-9)" />
            )}
          </Flex>
        </button>
        <button
          type="button"
          className={styles.newChatButton}
          onClick={handleNewChat}
        >
          <PlusIcon width={12} height={12} />
          <Text size="1">New Chat</Text>
        </button>
      </div>

      {breakpoint !== "narrow" && (
        <div className={styles.legend}>
          {DOT_LEGEND.map((item) => (
            <div key={item.label} className={styles.legendItem}>
              <div className={styles.legendDot} style={{ background: item.color }} />
              <Text size="1" color="gray">{item.label}</Text>
            </div>
          ))}
        </div>
      )}

      {expanded && (
        <div className={styles.controls}>
          <TextField.Root
            size="1"
            placeholder="Search..."
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
          >
            <TextField.Slot>
              <MagnifyingGlassIcon width={12} height={12} />
            </TextField.Slot>
          </TextField.Root>
        </div>
      )}

      <div className={styles.list}>
        {isInitialLoading && filteredTree.length === 0 ? (
          <Flex direction="column" gap="1" p="1">
            {Array.from({ length: 8 }, (_, i) => (
              <Flex key={i} align="center" gap="2" py="1" px="2">
                <Skeleton><div style={{ width: 8, height: 8, borderRadius: "50%" }} /></Skeleton>
                <Skeleton><Text size="2" style={{ width: `${120 + (i % 3) * 40}px` }}>&nbsp;</Text></Skeleton>
                <div style={{ flex: 1 }} />
                <Skeleton><Text size="1" style={{ width: 40 }}>&nbsp;</Text></Skeleton>
              </Flex>
            ))}
          </Flex>
        ) : (
          <Virtuoso
            data={flatItems}
            endReached={handleEndReached}
            overscan={200}
            className={styles.virtuosoList}
            itemContent={(_index, item) => {
              if (item.type === "header") {
                return (
                  <div className={styles.groupLabel}>
                    <Text size="1" color="gray" className={styles.groupLabelText}>
                      {item.label}
                    </Text>
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
                  onRename={handleRename}
                />
              );
            }}
            components={{
              Footer: () => (
                <>
                  {isLoadingMore && (
                    <Flex justify="center" py="2">
                      <Spinner size="2" />
                    </Flex>
                  )}
                  {loadMoreError && (
                    <Flex justify="center" py="2">
                      <Text size="1" color="red" style={{ cursor: "pointer" }} onClick={retryLoadMore}>
                        Load failed — click to retry
                      </Text>
                    </Flex>
                  )}
                </>
              ),
            }}
          />
        )}
        {!isInitialLoading && filteredTree.length === 0 && (
          <Text size="2" color="gray" style={{ padding: "var(--space-4)", textAlign: "center" }}>
            {searchQuery ? "No matching chats" : "No chats yet — start a new one!"}
          </Text>
        )}
      </div>
    </div>
  );
};
