import type { TabDisplayData } from "../Chat/Thread/selectors";
import type { TodoItem } from "../Chat/Thread/types";
import type { HistoryTreeNode } from "../History/historySlice";
export type DashboardBreakpoint = "narrow" | "medium" | "wide";
export type OpenTabData = TabDisplayData & {
    todos: TodoItem[];
    treeNode?: HistoryTreeNode;
};
