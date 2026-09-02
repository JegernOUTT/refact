import { createSlice, type PayloadAction } from "@reduxjs/toolkit";

import {
  setDockOpen,
  setDockSection,
  toggleDock,
} from "../Workspace/workspaceSlice";

export type AgentsPanelTab = "active" | "all";

export type AgentsPanelState = {
  autoOpenedFor: string | null;
  tab: AgentsPanelTab;
  userClosedByChat: Record<string, boolean>;
};

const initialState: AgentsPanelState = {
  autoOpenedFor: null,
  tab: "active",
  userClosedByChat: {},
};

export const agentsPanelSlice = createSlice({
  name: "agentsPanel",
  reducerPath: "agentsPanel",
  initialState,
  reducers: {
    tabChanged: (state, action: PayloadAction<AgentsPanelTab>) => {
      state.tab = action.payload;
    },
    autoOpenRequested: (state, action: PayloadAction<string>) => {
      if (state.userClosedByChat[action.payload]) return;
      state.autoOpenedFor = action.payload;
    },
    autoOpenCleared: (state) => {
      state.autoOpenedFor = null;
    },
    agentsSectionUserClosed: (state, action: PayloadAction<string>) => {
      state.userClosedByChat[action.payload] = true;
      state.autoOpenedFor = null;
    },
    agentsSectionUserOpened: (state, action: PayloadAction<string>) => {
      state.userClosedByChat[action.payload] = false;
      state.autoOpenedFor = null;
    },
  },
  extraReducers: (builder) => {
    builder
      .addCase(setDockSection, (state) => {
        state.autoOpenedFor = null;
      })
      .addCase(setDockOpen, (state) => {
        state.autoOpenedFor = null;
      })
      .addCase(toggleDock, (state) => {
        state.autoOpenedFor = null;
      });
  },
});

export const {
  tabChanged,
  autoOpenRequested,
  autoOpenCleared,
  agentsSectionUserClosed,
  agentsSectionUserOpened,
} = agentsPanelSlice.actions;

type AgentsPanelRootState = {
  agentsPanel: AgentsPanelState;
};

export const selectAgentsPanelTab = (state: AgentsPanelRootState) =>
  state.agentsPanel.tab;

export const selectAgentsPanelAutoOpenedFor = (state: AgentsPanelRootState) =>
  state.agentsPanel.autoOpenedFor;

export const selectAgentsPanelUserClosed = (
  state: AgentsPanelRootState,
  chatId: string,
) => {
  const userClosedByChat: Partial<Record<string, boolean>> =
    state.agentsPanel.userClosedByChat;
  return userClosedByChat[chatId] ?? false;
};

export default agentsPanelSlice.reducer;
