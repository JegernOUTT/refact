import { createSlice, PayloadAction } from "@reduxjs/toolkit";
import { RootState } from "../../app/store";

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

const initialState: TasksUIState = {
  openTasks: [],
};

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
    },
    closeTask: (state, action: PayloadAction<string>) => {
      state.openTasks = state.openTasks.filter((t) => t.id !== action.payload);
    },
    updateTaskName: (
      state,
      action: PayloadAction<{ id: string; name: string }>,
    ) => {
      const task = state.openTasks.find((t) => t.id === action.payload.id);
      if (task) {
        task.name = action.payload.name;
      }
    },
    addPlannerChat: (
      state,
      action: PayloadAction<{ taskId: string; planner: PlannerInfo }>,
    ) => {
      const task = state.openTasks.find((t) => t.id === action.payload.taskId);
      if (
        task &&
        !task.plannerChats.some((p) => p.id === action.payload.planner.id)
      ) {
        task.plannerChats.push(action.payload.planner);
      }
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
      }
    },
    setTaskActiveChat: (
      state,
      action: PayloadAction<{ taskId: string; activeChat: ActiveChat }>,
    ) => {
      const task = state.openTasks.find((t) => t.id === action.payload.taskId);
      if (task) {
        task.activeChat = action.payload.activeChat;
      }
    },
  },
  selectors: {
    selectOpenTasks: (state) => state.openTasks,
  },
});

export const {
  openTask,
  closeTask,
  updateTaskName,
  addPlannerChat,
  updatePlannerChat,
  removePlannerChat,
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
