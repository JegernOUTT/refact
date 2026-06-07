import { Selector } from 'reselect';
import { WritableDraft } from 'immer';
import { Slice, ActionCreatorWithPayload, PayloadAction } from '@reduxjs/toolkit';
import { RootState } from "../../app/store";
import type { TaskEvent } from "../../services/refact/sidebarSubscription";
type ActiveChat = {
    type: "planner";
    chatId: string;
} | {
    type: "agent";
    cardId: string;
    chatId: string;
} | null;
export interface PlannerInfo {
    id: string;
    title: string;
    createdAt: string;
    updatedAt: string;
    sessionState?: string;
    waitingForCardIds?: string[];
}
export interface OpenTask {
    id: string;
    name: string;
    plannerChats: PlannerInfo[];
    activeChat: ActiveChat;
}
export interface TasksUIState {
    openTasks: OpenTask[];
}
export declare const tasksSlice: Slice<TasksUIState, {
    openTask: (state: WritableDraft<TasksUIState>, action: PayloadAction<{
        id: string;
        name: string;
    }>) => void;
    closeTask: (state: WritableDraft<TasksUIState>, action: PayloadAction<string>) => void;
    updateTaskName: (state: WritableDraft<TasksUIState>, action: PayloadAction<{
        id: string;
        name: string;
    }>) => void;
    addPlannerChat: (state: WritableDraft<TasksUIState>, action: PayloadAction<{
        taskId: string;
        planner: PlannerInfo;
    }>) => void;
    updatePlannerChat: (state: WritableDraft<TasksUIState>, action: PayloadAction<{
        taskId: string;
        planner: Partial<PlannerInfo> & {
            id: string;
        };
    }>) => void;
    removePlannerChat: (state: WritableDraft<TasksUIState>, action: PayloadAction<{
        taskId: string;
        chatId: string;
    }>) => void;
    restorePlannerChat: (state: WritableDraft<TasksUIState>, action: PayloadAction<{
        taskId: string;
        planner: PlannerInfo;
    }>) => void;
    setTaskActiveChat: (state: WritableDraft<TasksUIState>, action: PayloadAction<{
        taskId: string;
        activeChat: ActiveChat;
    }>) => void;
}, "tasksUI", "tasksUI", {
    selectOpenTasks: (state: TasksUIState) => OpenTask[];
}>;
export declare const taskSseEventReceived: ActionCreatorWithPayload<TaskEvent, string>;
export declare const openTask: ActionCreatorWithPayload<{
    id: string;
    name: string;
}, "tasksUI/openTask">, closeTask: ActionCreatorWithPayload<string, "tasksUI/closeTask">, updateTaskName: ActionCreatorWithPayload<{
    id: string;
    name: string;
}, "tasksUI/updateTaskName">, addPlannerChat: ActionCreatorWithPayload<{
    taskId: string;
    planner: PlannerInfo;
}, "tasksUI/addPlannerChat">, updatePlannerChat: ActionCreatorWithPayload<{
    taskId: string;
    planner: Partial<PlannerInfo> & {
        id: string;
    };
}, "tasksUI/updatePlannerChat">, removePlannerChat: ActionCreatorWithPayload<{
    taskId: string;
    chatId: string;
}, "tasksUI/removePlannerChat">, restorePlannerChat: ActionCreatorWithPayload<{
    taskId: string;
    planner: PlannerInfo;
}, "tasksUI/restorePlannerChat">, setTaskActiveChat: ActionCreatorWithPayload<{
    taskId: string;
    activeChat: ActiveChat;
}, "tasksUI/setTaskActiveChat">;
export declare const selectOpenTasks: Selector<{
    tasksUI: TasksUIState;
}, OpenTask[], []> & {
    unwrapped: (state: TasksUIState) => OpenTask[];
};
export declare const selectOpenTasksFromRoot: (state: RootState) => OpenTask[];
export declare const selectTaskActiveChat: (state: RootState, taskId: string) => ActiveChat;
export {};
