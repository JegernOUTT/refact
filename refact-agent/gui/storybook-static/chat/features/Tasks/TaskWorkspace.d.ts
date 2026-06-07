import React from "react";
import { PlannerInfo } from "./tasksSlice";
interface PlannerItemProps {
    planner: PlannerInfo;
    isSelected: boolean;
    onSelect: () => void;
    onRemove: () => void;
}
export declare const PlannerItem: React.FC<PlannerItemProps>;
interface TaskWorkspaceProps {
    taskId: string;
}
export declare const TaskWorkspace: React.FC<TaskWorkspaceProps>;
export {};
