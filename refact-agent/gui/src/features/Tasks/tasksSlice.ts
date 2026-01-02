import { createSlice, PayloadAction } from "@reduxjs/toolkit";
import { RootState } from "../../app/store";

export interface OpenTask {
  id: string;
  name: string;
  plannerChats: string[];
}

export interface TasksUIState {
  openTasks: OpenTask[];
}

const initialState: TasksUIState = {
  openTasks: [],
};

export const tasksSlice = createSlice({
  name: "tasksUI",
  initialState,
  reducers: {
    openTask: (state, action: PayloadAction<{ id: string; name: string }>) => {
      const rawName = action.payload.name;
      const sanitizedName = (rawName && typeof rawName === "string" ? rawName.trim() : "") || "Task";
      const existing = state.openTasks.find((t) => t.id === action.payload.id);
      if (existing) {
        // Update name if changed and new name is meaningful
        if (sanitizedName !== "Task" && sanitizedName !== existing.name) {
          existing.name = sanitizedName;
        }
      } else {
        state.openTasks.push({ id: action.payload.id, name: sanitizedName, plannerChats: [] });
      }
    },
    closeTask: (state, action: PayloadAction<string>) => {
      state.openTasks = state.openTasks.filter((t) => t.id !== action.payload);
    },
    updateTaskName: (state, action: PayloadAction<{ id: string; name: string }>) => {
      const task = state.openTasks.find((t) => t.id === action.payload.id);
      if (task) {
        task.name = action.payload.name;
      }
    },
    addPlannerChat: (state, action: PayloadAction<{ taskId: string; chatId: string }>) => {
      const task = state.openTasks.find((t) => t.id === action.payload.taskId);
      if (task && !task.plannerChats.includes(action.payload.chatId)) {
        task.plannerChats.push(action.payload.chatId);
      }
    },
    removePlannerChat: (state, action: PayloadAction<{ taskId: string; chatId: string }>) => {
      const task = state.openTasks.find((t) => t.id === action.payload.taskId);
      if (task) {
        task.plannerChats = task.plannerChats.filter((c) => c !== action.payload.chatId);
      }
    },
  },
  selectors: {
    selectOpenTasks: (state) => state.openTasks,
  },
});

export const { openTask, closeTask, updateTaskName, addPlannerChat, removePlannerChat } = tasksSlice.actions;
export const { selectOpenTasks } = tasksSlice.selectors;

// Selector that works with RootState
export const selectOpenTasksFromRoot = (state: RootState) =>
  state.tasksUI.openTasks;
