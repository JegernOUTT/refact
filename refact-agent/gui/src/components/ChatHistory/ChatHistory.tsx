import { memo, useState, useCallback, useRef, useEffect, useMemo } from "react";
import { Flex, Box, Text, Spinner, Button } from "@radix-ui/themes";
import { ChatLoading } from "../ChatContent/ChatLoading";
import { ScrollArea } from "../ScrollArea";
import { HistoryItem } from "./HistoryItem";
import {
  ChatHistoryItem,
  HistoryTreeNode,
  buildHistoryTree,
} from "../../features/History/historySlice";

export type ChatHistoryProps = {
  history: Record<string, ChatHistoryItem>;
  isLoading?: boolean;
  onHistoryItemClick: (id: ChatHistoryItem) => void;
  onDeleteHistoryItem: (id: string) => void;
  onOpenChatInTab?: (id: string) => void;
  currentChatId?: string;
  treeView?: boolean;
  onLoadMore?: () => void;
  hasMore?: boolean;
  isLoadingMore?: boolean;
  loadMoreError?: string | null;
  onRetryLoadMore?: () => void;
  hasConnectionError?: boolean;
};

type TreeNodeProps = {
  node: HistoryTreeNode;
  depth: number;
  onHistoryItemClick: (id: ChatHistoryItem) => void;
  onDeleteHistoryItem: (id: string) => void;
  onOpenChatInTab?: (id: string) => void;
  currentChatId?: string;
  expandedIds: Set<string>;
  onToggleExpand: (id: string) => void;
};

const TreeNode = memo(
  ({
    node,
    depth,
    onHistoryItemClick,
    onDeleteHistoryItem,
    onOpenChatInTab,
    currentChatId,
    expandedIds,
    onToggleExpand,
  }: TreeNodeProps) => {
    const hasChildren = node.children.length > 0;
    const isExpanded = expandedIds.has(node.id);
    const isTask = !!node.task_id;
    const linkType = node.link_type;

    const isHandoffParent = depth > 0 && !linkType && !isTask;

    const getBadge = () => {
      if (isTask) {
        return node.task_role === "planner"
          ? "Planner"
          : node.task_role === "agents"
            ? "Agent"
            : undefined;
      }
      if (linkType === "subagent") return "Subagent";
      if (linkType === "handoff") return "Handoff";
      if (isHandoffParent) return "Original";
      return undefined;
    };

    return (
      <Box style={{ width: "100%", paddingLeft: depth * 16 }}>
        <HistoryItem
          onClick={() => onHistoryItemClick(node)}
          onOpenInTab={onOpenChatInTab}
          onDelete={onDeleteHistoryItem}
          historyItem={node}
          disabled={node.id === currentChatId}
          badge={getBadge()}
          childCount={hasChildren ? node.children.length : undefined}
          isExpanded={isExpanded}
          onToggleExpand={
            hasChildren ? () => onToggleExpand(node.id) : undefined
          }
        />
        {hasChildren && isExpanded && (
          <Flex direction="column" gap="1" pt="1">
            {node.children.map((child) => (
              <TreeNode
                key={child.id}
                node={child}
                depth={depth + 1}
                onHistoryItemClick={onHistoryItemClick}
                onDeleteHistoryItem={onDeleteHistoryItem}
                onOpenChatInTab={onOpenChatInTab}
                currentChatId={currentChatId}
                expandedIds={expandedIds}
                onToggleExpand={onToggleExpand}
              />
            ))}
          </Flex>
        )}
      </Box>
    );
  },
);

TreeNode.displayName = "TreeNode";

function getSortedHistory(
  history: Record<string, ChatHistoryItem>,
): ChatHistoryItem[] {
  return Object.values(history)
    .filter((item) => !item.task_id)
    .sort((a, b) => b.updatedAt.localeCompare(a.updatedAt));
}

export const ChatHistory = memo(
  ({
    history,
    onHistoryItemClick,
    onDeleteHistoryItem,
    onOpenChatInTab,
    currentChatId,
    treeView = false,
    isLoading = false,
    onLoadMore,
    hasMore = false,
    isLoadingMore = false,
    loadMoreError,
    onRetryLoadMore,
    hasConnectionError = false,
  }: ChatHistoryProps) => {
    const sortedHistory = useMemo(() => getSortedHistory(history), [history]);
    const historyTree = useMemo(() => buildHistoryTree(history), [history]);
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

      const observer = new IntersectionObserver(
        (entries) => {
          if (entries[0]?.isIntersecting) {
            onLoadMore();
          }
        },
        { threshold: 0.1 },
      );

      const currentRef = loadMoreRef.current;
      if (currentRef) {
        observer.observe(currentRef);
      }

      return () => {
        if (currentRef) {
          observer.unobserve(currentRef);
        }
      };
    }, [onLoadMore, hasMore, isLoadingMore]);

    const hasChildChats = sortedHistory.some((item) => !!item.parent_id);
    const showTree = treeView || hasChildChats;

    return (
      <Box
        style={{
          overflow: "hidden",
        }}
        pb="2"
        flexGrow="1"
      >
        <ScrollArea scrollbars="vertical">
          <Flex
            justify="center"
            align={sortedHistory.length > 0 ? "center" : "start"}
            pl="2"
            pr="2"
            gap="1"
            direction="column"
          >
            {isLoading ? (
              <Box style={{ width: "100%" }}>
                <ChatLoading />
              </Box>
            ) : sortedHistory.length !== 0 ? (
              <>
                {showTree
                  ? historyTree.map((node) => (
                      <TreeNode
                        key={node.id}
                        node={node}
                        depth={0}
                        onHistoryItemClick={onHistoryItemClick}
                        onDeleteHistoryItem={onDeleteHistoryItem}
                        onOpenChatInTab={onOpenChatInTab}
                        currentChatId={currentChatId}
                        expandedIds={expandedIds}
                        onToggleExpand={handleToggleExpand}
                      />
                    ))
                  : sortedHistory.map((item) => (
                      <HistoryItem
                        onClick={() => onHistoryItemClick(item)}
                        onOpenInTab={onOpenChatInTab}
                        onDelete={onDeleteHistoryItem}
                        key={item.id}
                        historyItem={item}
                        disabled={item.id === currentChatId}
                      />
                    ))}
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
                {hasConnectionError ? "Unable to load chats" : "No chats yet"}
              </Text>
            )}
          </Flex>
        </ScrollArea>
      </Box>
    );
  },
);

ChatHistory.displayName = "ChatHistory";
