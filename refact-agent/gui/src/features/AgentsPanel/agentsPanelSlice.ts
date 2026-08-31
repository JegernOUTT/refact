import { createSlice, type PayloadAction } from "@reduxjs/toolkit";

export type AgentsPanelTab = "active" | "all";

export type AgentsPanelState = {
  openByChat: Record<string, boolean | undefined>;
  tab: AgentsPanelTab;
  userClosedByChat: Record<string, boolean>;
};

const initialState: AgentsPanelState = {
  openByChat: {},
  tab: "active",
  userClosedByChat: {},
};

export const agentsPanelSlice = createSlice({
  name: "agentsPanel",
  reducerPath: "agentsPanel",
  initialState,
  reducers: {
    panelOpened: (state, action: PayloadAction<string>) => {
      state.openByChat[action.payload] = true;
      state.userClosedByChat[action.payload] = false;
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
      }
    },
  },
});

export const { panelOpened, panelClosed, tabChanged, autoOpenRequested } =
  agentsPanelSlice.actions;

type AgentsPanelRootState = {
  agentsPanel: AgentsPanelState;
};

export const selectAgentsPanelOpen = (
  state: AgentsPanelRootState,
  chatId: string,
) => {
  const openByChat = state.agentsPanel.openByChat;
  return Object.prototype.hasOwnProperty.call(openByChat, chatId)
    ? openByChat[chatId]
    : false;
};

export const selectAgentsPanelTab = (state: AgentsPanelRootState) =>
  state.agentsPanel.tab;

export const selectAgentsPanelUserClosed = (
  state: AgentsPanelRootState,
  chatId: string,
) => {
  const userClosedByChat = state.agentsPanel.userClosedByChat;
  return Object.prototype.hasOwnProperty.call(userClosedByChat, chatId)
    ? userClosedByChat[chatId]
    : false;
};

export default agentsPanelSlice.reducer;
