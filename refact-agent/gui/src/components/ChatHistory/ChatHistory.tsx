import { memo, useState, useCallback } from "react";
import { Flex, Box, Text } from "@radix-ui/themes";
import { ChatLoading } from "../ChatContent/ChatLoading";
import { ScrollArea } from "../ScrollArea";
import { HistoryItem } from "./HistoryItem";
import {
  ChatHistoryItem,
  HistoryTreeNode,
} from "../../features/History/historySlice";

export type ChatHistoryProps = {
  history: Record<string, ChatHistoryItem>;
  isLoading?: boolean;
  onHistoryItemClick: (id: ChatHistoryItem) => void;
  onDeleteHistoryItem: (id: string) => void;
  onOpenChatInTab?: (id: string) => void;
  currentChatId?: string;
  treeView?: boolean;
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

function getSortedHistory(history: Record<string, ChatHistoryItem>): ChatHistoryItem[] {
  return Object.values(history)
    .filter((item) => !item.task_id)
    .sort((a, b) => b.updatedAt.localeCompare(a.updatedAt));
}

function buildHistoryTree(history: Record<string, ChatHistoryItem>): HistoryTreeNode[] {
  const items = Object.values(history).filter((item) => !item.task_id);
  const itemMap = new Map<string, HistoryTreeNode>();
  const roots: HistoryTreeNode[] = [];

  for (const item of items) {
    itemMap.set(item.id, { ...item, children: [] });
  }

  const assignedAsChild = new Set<string>();
  const handoffParentIds = new Set<string>();

  for (const item of items) {
    if (item.link_type === "handoff" && item.parent_id && itemMap.has(item.parent_id)) {
      handoffParentIds.add(item.parent_id);
    }
  }

  for (const item of items) {
    const node = itemMap.get(item.id);
    if (!node) continue;

    if (handoffParentIds.has(item.id)) continue;

    if (item.parent_id && itemMap.has(item.parent_id)) {
      if (assignedAsChild.has(item.id)) {
        roots.push(node);
        continue;
      }
      const parent = itemMap.get(item.parent_id);
      if (!parent || parent.parent_id === item.id) {
        roots.push(node);
        continue;
      }

      if (item.link_type === "handoff") {
        const parentNode = itemMap.get(item.parent_id);
        if (parentNode) {
          node.children.push(parentNode);
          assignedAsChild.add(item.parent_id);
          roots.push(node);
        }
      } else {
        const parentNode = itemMap.get(item.parent_id);
        if (parentNode) {
          parentNode.children.push(node);
          assignedAsChild.add(item.id);
        }
      }
    } else {
      roots.push(node);
    }
  }

  const sortByUpdated = (a: HistoryTreeNode, b: HistoryTreeNode) =>
    b.updatedAt.localeCompare(a.updatedAt);

  const sortTree = (nodes: HistoryTreeNode[]) => {
    nodes.sort(sortByUpdated);
    for (const node of nodes) {
      if (node.children.length > 0) sortTree(node.children);
    }
  };

  sortTree(roots);
  return roots;
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
  }: ChatHistoryProps) => {
    const sortedHistory = getSortedHistory(history);
    const historyTree = buildHistoryTree(history);
    const [expandedIds, setExpandedIds] = useState<Set<string>>(new Set());

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
              showTree ? (
                historyTree.map((node) => (
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
              ) : (
                sortedHistory.map((item) => (
                  <HistoryItem
                    onClick={() => onHistoryItemClick(item)}
                    onOpenInTab={onOpenChatInTab}
                    onDelete={onDeleteHistoryItem}
                    key={item.id}
                    historyItem={item}
                    disabled={item.id === currentChatId}
                  />
                ))
              )
            ) : (
              <Text as="p" size="2" mt="2">
                Your chat history is currently empty. Click &quot;New Chat&quot;
                to start a conversation.
              </Text>
            )}
          </Flex>
        </ScrollArea>
      </Box>
    );
  },
);

ChatHistory.displayName = "ChatHistory";
