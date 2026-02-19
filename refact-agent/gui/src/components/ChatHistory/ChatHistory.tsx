import { memo, useState, useCallback, useRef, useEffect, useMemo } from "react";
import { Flex, Box, Text, Spinner, Button } from "@radix-ui/themes";
import { ChatLoading } from "../ChatContent/ChatLoading";
import { ScrollArea } from "../ScrollArea";
import { HistoryItem } from "./HistoryItem";
import { HistoryItemCompact } from "./HistoryItemCompact";
import { TaskItemCompact } from "./TaskItemCompact";
import {
  ChatHistoryItem,
  HistoryTreeNode,
  buildHistoryTree,
  isTaskChatLike,
} from "../../features/History/historySlice";
import type { TaskMeta } from "../../services/refact/tasks";

export type ChatHistoryProps = {
  history: Record<string, ChatHistoryItem>;
  tasks?: TaskMeta[];
  isLoading?: boolean;
  onHistoryItemClick: (id: ChatHistoryItem) => void;
  onDeleteHistoryItem: (id: string) => void;
  onRenameHistoryItem?: (id: string, newTitle: string) => void;
  onOpenChatInTab?: (id: string) => void;
  onTaskClick?: (taskId: string) => void;
  onDeleteTask?: (taskId: string) => void;
  onRenameTask?: (taskId: string, newName: string) => void;
  currentChatId?: string;
  treeView?: boolean;
  compactView?: boolean;
  onLoadMore?: () => void;
  hasMore?: boolean;
  isLoadingMore?: boolean;
  loadMoreError?: string | null;
  onRetryLoadMore?: () => void;
  hasConnectionError?: boolean;
  noScroll?: boolean;
  scrollContainerRef?: React.RefObject<HTMLDivElement>;
};

type TreeNodeProps = {
  node: HistoryTreeNode;
  depth: number;
  onHistoryItemClick: (id: ChatHistoryItem) => void;
  onDeleteHistoryItem: (id: string) => void;
  onRenameHistoryItem?: (id: string, newTitle: string) => void;
  onOpenChatInTab?: (id: string) => void;
  currentChatId?: string;
  expandedIds: Set<string>;
  onToggleExpand: (id: string) => void;
  compactView?: boolean;
};

function getBadgeForNode(
  node: HistoryTreeNode,
  depth: number,
): string | undefined {
  const isTask = !!node.task_id;
  const linkType = node.link_type;
  const isHandoffParent = depth > 0 && !linkType && !isTask;

  if (isTask) {
    return node.task_role === "planner"
      ? "Planner"
      : node.task_role === "agents"
        ? "Agent"
        : undefined;
  }
  if (linkType === "subagent") return "Subagent";
  if (linkType === "handoff") return "Handoff";
  if (linkType === "mode_transition") return "Mode Switch";
  if (linkType === "branch") return "Branched";
  if (isHandoffParent) return "Original";
  return undefined;
}

const TreeNode = memo(
  ({
    node,
    depth,
    onHistoryItemClick,
    onDeleteHistoryItem,
    onRenameHistoryItem,
    onOpenChatInTab,
    currentChatId,
    expandedIds,
    onToggleExpand,
    compactView = false,
  }: TreeNodeProps) => {
    const hasChildren = node.children.length > 0;
    const isExpanded = expandedIds.has(node.id);
    const badge = getBadgeForNode(node, depth);

    return (
      <Box style={{ width: "100%" }}>
        {compactView ? (
          <HistoryItemCompact
            historyItem={node}
            onClick={() => onHistoryItemClick(node)}
            onDelete={onDeleteHistoryItem}
            onRename={onRenameHistoryItem}
            disabled={node.id === currentChatId}
            badge={badge}
            childCount={hasChildren ? node.children.length : undefined}
            isExpanded={isExpanded}
            onToggleExpand={
              hasChildren ? () => onToggleExpand(node.id) : undefined
            }
            isChild={depth > 0}
          />
        ) : (
          <Box style={{ paddingLeft: depth * 16 }}>
            <HistoryItem
              onClick={() => onHistoryItemClick(node)}
              onOpenInTab={onOpenChatInTab}
              onDelete={onDeleteHistoryItem}
              historyItem={node}
              disabled={node.id === currentChatId}
              badge={badge}
              childCount={hasChildren ? node.children.length : undefined}
              isExpanded={isExpanded}
              onToggleExpand={
                hasChildren ? () => onToggleExpand(node.id) : undefined
              }
            />
          </Box>
        )}
        {hasChildren && isExpanded && (
          <Flex direction="column" gap="1" pt="1">
            {node.children.map((child) => (
              <TreeNode
                key={child.id}
                node={child}
                depth={depth + 1}
                onHistoryItemClick={onHistoryItemClick}
                onDeleteHistoryItem={onDeleteHistoryItem}
                onRenameHistoryItem={onRenameHistoryItem}
                onOpenChatInTab={onOpenChatInTab}
                currentChatId={currentChatId}
                expandedIds={expandedIds}
                onToggleExpand={onToggleExpand}
                compactView={compactView}
              />
            ))}
          </Flex>
        )}
      </Box>
    );
  },
);

TreeNode.displayName = "TreeNode";

type UnifiedItem =
  | { type: "chat"; item: ChatHistoryItem }
  | { type: "tree"; item: HistoryTreeNode }
  | { type: "task"; item: TaskMeta };

function getActiveTasks(tasks: TaskMeta[] = []): TaskMeta[] {
  return tasks.filter(
    (t) =>
      t.status === "active" || t.status === "planning" || t.status === "paused",
  );
}

function getUpdatedAt(item: UnifiedItem): string {
  switch (item.type) {
    case "chat":
    case "tree":
      return item.item.updatedAt;
    case "task":
      return item.item.updated_at;
  }
}

function getSortedUnifiedList(
  history: Record<string, ChatHistoryItem>,
  tasks: TaskMeta[] = [],
  useTree: boolean,
  historyTree: HistoryTreeNode[],
): UnifiedItem[] {
  const activeTasks = getActiveTasks(tasks);

  if (useTree) {
    // In tree mode, merge tree root nodes with tasks
    const treeItems: UnifiedItem[] = historyTree.map((item) => ({
      type: "tree" as const,
      item,
    }));

    const taskItems: UnifiedItem[] = activeTasks.map((item) => ({
      type: "task" as const,
      item,
    }));

    return [...treeItems, ...taskItems].sort((a, b) =>
      getUpdatedAt(b).localeCompare(getUpdatedAt(a)),
    );
  }

  // In flat mode, merge chats with tasks
  const chatItems: UnifiedItem[] = Object.values(history)
    .filter((item) => !isTaskChatLike(item))
    .map((item) => ({ type: "chat" as const, item }));

  const taskItems: UnifiedItem[] = activeTasks.map((item) => ({
    type: "task" as const,
    item,
  }));

  return [...chatItems, ...taskItems].sort((a, b) =>
    getUpdatedAt(b).localeCompare(getUpdatedAt(a)),
  );
}

function hasChildChatsInHistory(
  history: Record<string, ChatHistoryItem>,
): boolean {
  return Object.values(history).some((item) => !!item.parent_id);
}

export const ChatHistory = memo(
  ({
    history,
    tasks = [],
    onHistoryItemClick,
    onDeleteHistoryItem,
    onRenameHistoryItem,
    onOpenChatInTab,
    onTaskClick,
    onDeleteTask,
    onRenameTask,
    currentChatId,
    treeView = false,
    compactView = true,
    isLoading = false,
    onLoadMore,
    hasMore = false,
    isLoadingMore = false,
    loadMoreError,
    onRetryLoadMore,
    hasConnectionError = false,
    noScroll = false,
    scrollContainerRef,
  }: ChatHistoryProps) => {
    const historyTree = useMemo(() => buildHistoryTree(history), [history]);
    const hasChildChats = useMemo(
      () => hasChildChatsInHistory(history),
      [history],
    );
    const showTree = treeView || hasChildChats;
    const unifiedList = useMemo(
      () => getSortedUnifiedList(history, tasks, showTree, historyTree),
      [history, tasks, showTree, historyTree],
    );
    const [expandedIds, setExpandedIds] = useState<Set<string>>(new Set());
    const loadMoreRef = useRef<HTMLDivElement>(null);

    const handleToggleExpand = useCallback((id: string) => {
      setExpandedIds((prev) => {
        const next = new Set(prev);
        if (next.has(id)) {
          next.delete(id);
        } else {
          next.add(id);
        }
        return next;
      });
    }, []);

    useEffect(() => {
      if (!onLoadMore || !hasMore || isLoadingMore) return;

      const loadMoreElement = loadMoreRef.current;
      if (!loadMoreElement) return;

      // Find the scroll container - either passed ref or use viewport
      const root = scrollContainerRef?.current ?? null;

      const observer = new IntersectionObserver(
        (entries) => {
          if (entries[0]?.isIntersecting) {
            onLoadMore();
          }
        },
        {
          threshold: 0.1,
          root,
        },
      );

      observer.observe(loadMoreElement);

      return () => {
        observer.disconnect();
      };
    }, [onLoadMore, hasMore, isLoadingMore, scrollContainerRef]);

    const content = (
      <Flex
        justify="center"
        align={unifiedList.length > 0 ? "center" : "start"}
        pl="1"
        pr="1"
        gap="1"
        direction="column"
      >
        {isLoading ? (
          <Box style={{ width: "100%" }}>
            <ChatLoading />
          </Box>
        ) : unifiedList.length !== 0 ? (
          <>
            {unifiedList.map((unified) => {
              if (unified.type === "task") {
                return (
                  <Box
                    key={`task-${unified.item.id}`}
                    style={{ width: "100%" }}
                  >
                    <TaskItemCompact
                      task={unified.item}
                      onClick={() => onTaskClick?.(unified.item.id)}
                      onDelete={(id) => onDeleteTask?.(id)}
                      onRename={onRenameTask}
                      badge="Task"
                    />
                  </Box>
                );
              }
              if (unified.type === "tree") {
                return (
                  <TreeNode
                    key={unified.item.id}
                    node={unified.item}
                    depth={0}
                    onHistoryItemClick={onHistoryItemClick}
                    onDeleteHistoryItem={onDeleteHistoryItem}
                    onRenameHistoryItem={onRenameHistoryItem}
                    onOpenChatInTab={onOpenChatInTab}
                    currentChatId={currentChatId}
                    expandedIds={expandedIds}
                    onToggleExpand={handleToggleExpand}
                    compactView={compactView}
                  />
                );
              }
              // type === "chat"
              return compactView ? (
                <Box key={unified.item.id} style={{ width: "100%" }}>
                  <HistoryItemCompact
                    historyItem={unified.item}
                    onClick={() => onHistoryItemClick(unified.item)}
                    onDelete={onDeleteHistoryItem}
                    onRename={onRenameHistoryItem}
                    disabled={unified.item.id === currentChatId}
                  />
                </Box>
              ) : (
                <Box key={unified.item.id} style={{ width: "100%" }}>
                  <HistoryItem
                    onClick={() => onHistoryItemClick(unified.item)}
                    onOpenInTab={onOpenChatInTab}
                    onDelete={onDeleteHistoryItem}
                    historyItem={unified.item}
                    disabled={unified.item.id === currentChatId}
                  />
                </Box>
              );
            })}
            {loadMoreError && onRetryLoadMore && (
              <Flex
                py="2"
                direction="column"
                align="center"
                gap="2"
                style={{ width: "100%" }}
              >
                <Text size="1" color="red">
                  {loadMoreError}
                </Text>
                <Button size="1" variant="soft" onClick={onRetryLoadMore}>
                  Retry
                </Button>
              </Flex>
            )}
            {hasMore && !loadMoreError && (
              <Box ref={loadMoreRef} py="2" style={{ width: "100%" }}>
                {isLoadingMore ? (
                  <Flex justify="center">
                    <Spinner size="2" />
                  </Flex>
                ) : (
                  <Box style={{ height: 1 }} />
                )}
              </Box>
            )}
          </>
        ) : (
          <Text size="2" color={hasConnectionError ? "red" : "gray"}>
            {hasConnectionError ? "Unable to load" : "No chats yet"}
          </Text>
        )}
      </Flex>
    );

    if (noScroll) {
      return <Box pb="2">{content}</Box>;
    }

    return (
      <Box
        style={{
          overflow: "hidden",
        }}
        pb="2"
        flexGrow="1"
      >
        <ScrollArea scrollbars="vertical">{content}</ScrollArea>
      </Box>
    );
  },
);

ChatHistory.displayName = "ChatHistory";
