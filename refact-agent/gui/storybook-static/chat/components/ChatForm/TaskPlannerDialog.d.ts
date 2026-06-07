import React from "react";
type TaskPlannerDialogProps = {
    open: boolean;
    onOpenChange: (open: boolean) => void;
    /** Present when opened from inside a task workspace; otherwise a new task is created */
    taskId?: string;
    /** Description of the task_planner mode, used for context-transfer analysis */
    targetModeDescription?: string;
};
export declare const TaskPlannerDialog: React.FC<TaskPlannerDialogProps>;
export {};
