import React from "react";
import type { TodoItem } from "../../../Chat/Thread/types";
import type { DashboardBreakpoint } from "../../types";
type TodoProgressProps = {
    todos: TodoItem[];
    breakpoint: DashboardBreakpoint;
};
export declare const TodoProgress: React.FC<TodoProgressProps>;
export {};
