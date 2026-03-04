import { useMemo } from "react";
import { useAppSelector } from "../../../hooks";
import {
  selectTabsDisplayData,
  selectAllThreads,
  deriveTasksFromMessages,
} from "../../Chat/Thread/selectors";
import { buildHistoryTree } from "../../History/historySlice";
import { isToolMessage } from "../../../services/refact/types";
import type { HistoryTreeNode } from "../../History/historySlice";
import type { OpenTabData } from "../types";

function findNodeInList(
  nodes: HistoryTreeNode[],
  id: string,
): HistoryTreeNode | undefined {
  for (const n of nodes) {
    if (n.id === id) return n;
    const found = findNodeInList(n.children, id);
    if (found) return found;
  }
  return undefined;
}

export function useOpenTabsData(): OpenTabData[] {
  const tabs = useAppSelector(selectTabsDisplayData);
  const threads = useAppSelector(selectAllThreads);
  const historyChats = useAppSelector((state) => state.history.chats, {
    devModeChecks: { stabilityCheck: "never" },
  });

  const tree = useMemo(() => buildHistoryTree(historyChats), [historyChats]);

  return useMemo(() => {
    return tabs.map((tab) => {
      const runtime = threads[tab.id];
      let todos: OpenTabData["todos"] = [];
      if (runtime?.thread.messages && runtime.thread.messages.length > 0) {
        const messages = runtime.thread.messages;
        const toolMessages = messages.filter(isToolMessage);
        todos = deriveTasksFromMessages(messages, toolMessages);
      }
      const treeNode = findNodeInList(tree, tab.id);
      return { ...tab, todos, treeNode };
    });
  }, [tabs, threads, tree]);
}
