import { createAction, createSlice, PayloadAction } from "@reduxjs/toolkit";
import { RootState } from "../../app/store";
import type { TaskEvent } from "../../services/refact/sidebarSubscription";
import {
  loadPersistedTasksUIState,
  savePersistedTasksUIState,
} from "../../utils/chatUiPersistence";
import { hydratePersistedChatTabs } from "../Chat/Thread/actions";

type ActiveChat =
  | { type: "planner"; chatId: string }
  | { type: "agent"; cardId: string; chatId: string }
  | null;

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

function persistTasksUIState(state: TasksUIState): void {
  savePersistedTasksUIState({
    openTasks: state.openTasks.map((task) => ({
      id: task.id,
      name: task.name,
      plannerChats: task.plannerChats.map((planner) => ({ ...planner })),
      activeChat: task.activeChat ? { ...task.activeChat } : null,
    })),
  });
}

const initialState: TasksUIState = loadPersistedTasksUIState();

export const tasksSlice = createSlice({
  name: "tasksUI",
  initialState,
  reducers: {
    openTask: (state, action: PayloadAction<{ id: string; name: string }>) => {
      const rawName = action.payload.name;
      const sanitizedName =
        (rawName && typeof rawName === "string" ? rawName.trim() : "") ||
        "Task";
      const existing = state.openTasks.find((t) => t.id === action.payload.id);
      if (existing) {
        // Update name if changed and new name is meaningful
        if (sanitizedName !== "Task" && sanitizedName !== existing.name) {
          existing.name = sanitizedName;
        }
      } else {
        state.openTasks.push({
          id: action.payload.id,
          name: sanitizedName,
          plannerChats: [],
          activeChat: null,
        });
      }
      persistTasksUIState(state);
    },
    closeTask: (state, action: PayloadAction<string>) => {
      state.openTasks = state.openTasks.filter((t) => t.id !== action.payload);
      persistTasksUIState(state);
    },
    reorderOpenTasks: (
      state,
      action: PayloadAction<{ sourceId: string; targetId: string }>,
    ) => {
      const { sourceId, targetId } = action.payload;
      if (sourceId === targetId) return;
      const sourceIndex = state.openTasks.findIndex((task) => task.id === sourceId);
      const targetIndex = state.openTasks.findIndex((task) => task.id === targetId);
      if (sourceIndex === -1 || targetIndex === -1) return;
      const [source] = state.openTasks.splice(sourceIndex, 1);
      state.openTasks.splice(targetIndex, 0, source);
      persistTasksUIState(state);
    },
    updateTaskName: (
      state,
      action: PayloadAction<{ id: string; name: string }>,
    ) => {
      const task = state.openTasks.find((t) => t.id === action.payload.id);
      if (task) {
        task.name = action.payload.name;
        persistTasksUIState(state);
      }
    },
    addPlannerChat: (
      state,
      action: PayloadAction<{ taskId: string; planner: PlannerInfo }>,
    ) => {
      const task = state.openTasks.find((t) => t.id === action.payload.taskId);
      if (!task) return;

      const existing = task.plannerChats.find(
        (p) => p.id === action.payload.planner.id,
      );
      if (existing) {
        Object.assign(existing, action.payload.planner);
      } else {
        task.plannerChats.push(action.payload.planner);
      }
      if (!task.activeChat) {
        task.activeChat = {
          type: "planner",
          chatId: action.payload.planner.id,
        };
      }
      persistTasksUIState(state);
    },
    updatePlannerChat: (
      state,
      action: PayloadAction<{
        taskId: string;
        planner: Partial<PlannerInfo> & { id: string };
      }>,
    ) => {
      const task = state.openTasks.find((t) => t.id === action.payload.taskId);
      if (task) {
        const idx = task.plannerChats.findIndex(
          (p) => p.id === action.payload.planner.id,
        );
        if (idx !== -1) {
          task.plannerChats[idx] = {
            ...task.plannerChats[idx],
            ...action.payload.planner,
          };
          persistTasksUIState(state);
        }
      }
    },
    removePlannerChat: (
      state,
      action: PayloadAction<{ taskId: string; chatId: string }>,
    ) => {
      const task = state.openTasks.find((t) => t.id === action.payload.taskId);
      if (task) {
        task.plannerChats = task.plannerChats.filter(
          (p) => p.id !== action.payload.chatId,
        );
        persistTasksUIState(state);
      }
    },
    restorePlannerChat: (
      state,
      action: PayloadAction<{ taskId: string; planner: PlannerInfo }>,
    ) => {
      const task = state.openTasks.find((t) => t.id === action.payload.taskId);
      if (!task) return;
      const existing = task.plannerChats.find(
        (p) => p.id === action.payload.planner.id,
      );
      if (!existing) {
        task.plannerChats.push(action.payload.planner);
        persistTasksUIState(state);
      }
    },
    setTaskActiveChat: (
      state,
      action: PayloadAction<{ taskId: string; activeChat: ActiveChat }>,
    ) => {
      const task = state.openTasks.find((t) => t.id === action.payload.taskId);
      if (task) {
        task.activeChat = action.payload.activeChat;
        persistTasksUIState(state);
      }
    },
  },
  extraReducers: (builder) => {
    builder.addCase(hydratePersistedChatTabs, (state) => {
      state.openTasks = loadPersistedTasksUIState().openTasks;
    });
  },
  selectors: {
    selectOpenTasks: (state) => state.openTasks,
  },
});

export const taskSseEventReceived = createAction<TaskEvent>(
  "tasks/sseEventReceived",
);

export const {
  openTask,
  closeTask,
  reorderOpenTasks,
  updateTaskName,
  addPlannerChat,
  updatePlannerChat,
  removePlannerChat,
  restorePlannerChat,
  setTaskActiveChat,
} = tasksSlice.actions;
export const { selectOpenTasks } = tasksSlice.selectors;

// Selector that works with RootState
export const selectOpenTasksFromRoot = (state: RootState) =>
  state.tasksUI.openTasks;

export const selectTaskActiveChat = (
  state: RootState,
  taskId: string,
): ActiveChat => {
  const task = state.tasksUI.openTasks.find((t) => t.id === taskId);
  return task?.activeChat ?? null;
};
