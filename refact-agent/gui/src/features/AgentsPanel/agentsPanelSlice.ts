import { createSlice, type PayloadAction } from "@reduxjs/toolkit";

export type AgentsPanelTab = "active" | "all";

export type AgentsPanelState = {
  openByChat: Record<string, boolean>;
  tab: AgentsPanelTab;
  userClosedByChat: Record<string, boolean>;
  userOpenedByChat: Record<string, boolean>;
};

const initialState: AgentsPanelState = {
  openByChat: {},
  tab: "active",
  userClosedByChat: {},
  userOpenedByChat: {},
};

export const agentsPanelSlice = createSlice({
  name: "agentsPanel",
  reducerPath: "agentsPanel",
  initialState,
  reducers: {
    panelOpened: (state, action: PayloadAction<string>) => {
      state.openByChat[action.payload] = true;
      state.userClosedByChat[action.payload] = false;
      state.userOpenedByChat[action.payload] = true;
    },
    panelClosed: (state, action: PayloadAction<string>) => {
      state.openByChat[action.payload] = false;
      state.userClosedByChat[action.payload] = true;
    },
    tabChanged: (state, action: PayloadAction<AgentsPanelTab>) => {
      state.tab = action.payload;
    },
    autoOpenRequested: (state, action: PayloadAction<string>) => {
      if (!state.userClosedByChat[action.payload]) {
        state.openByChat[action.payload] = true;
        state.userOpenedByChat[action.payload] = false;
      }
    },
    panelAutoClosed: (state, action: PayloadAction<string>) => {
      state.openByChat[action.payload] = false;
    },
  },
});

export const {
  panelOpened,
  panelClosed,
  tabChanged,
  autoOpenRequested,
  panelAutoClosed,
} = agentsPanelSlice.actions;

type AgentsPanelRootState = {
  agentsPanel: AgentsPanelState;
};

export const selectAgentsPanelOpen = (
  state: AgentsPanelRootState,
  chatId: string,
) => {
  const openByChat: Partial<Record<string, boolean>> =
    state.agentsPanel.openByChat;
  return openByChat[chatId] ?? false;
};

export const selectAgentsPanelTab = (state: AgentsPanelRootState) =>
  state.agentsPanel.tab;

export const selectAgentsPanelUserClosed = (
  state: AgentsPanelRootState,
  chatId: string,
) => {
  const userClosedByChat: Partial<Record<string, boolean>> =
    state.agentsPanel.userClosedByChat;
  return userClosedByChat[chatId] ?? false;
};

export const selectAgentsPanelUserOpened = (
  state: AgentsPanelRootState,
  chatId: string,
) => {
  const userOpenedByChat: Partial<Record<string, boolean>> =
    state.agentsPanel.userOpenedByChat;
  return userOpenedByChat[chatId] ?? false;
};

export default agentsPanelSlice.reducer;
