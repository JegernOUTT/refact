import {
  createSelector,
  createSlice,
  type PayloadAction,
} from "@reduxjs/toolkit";
import type { RootState } from "../../app/store";
import {
  getEngineEndpointIdentity,
  hasUsableEngineEndpoint,
} from "../../services/refact/apiUrl";
import type { BackgroundAgentSummary } from "../../services/refact/types";
import type { FetchBaseQueryError } from "@reduxjs/toolkit/query";

export type AgentsPanelTab = "active" | "all";

export type AgentRosterSource = {
  identity: string;
  generation: number;
  disclosure: number;
};

type AgentRosterState = Pick<
  RootState,
  "config" | "current_project" | "agentsPanel"
>;

export type AgentsPanelState = {
  tab: AgentsPanelTab;
  generation: number;
  disclosure: number;
  authorized: boolean;
  acceptedDisclosure: number;
  denied: boolean;
  failure: FetchBaseQueryError | null;
  liveAgents: Record<string, BackgroundAgentSummary>;
  transportStale: boolean;
};

const initialState: AgentsPanelState = {
  tab: "active",
  generation: 0,
  disclosure: 0,
  authorized: false,
  acceptedDisclosure: 0,
  denied: false,
  failure: null,
  liveAgents: {},
  transportStale: false,
};

function matchesGeneration(state: AgentsPanelState, source: AgentRosterSource) {
  return (
    state.generation === source.generation &&
    state.disclosure === source.disclosure
  );
}

function mergeAgent(
  agents: Partial<Record<string, BackgroundAgentSummary>>,
  incoming: BackgroundAgentSummary,
) {
  const existing = agents[incoming.agent_id];
  const terminal = (agent: BackgroundAgentSummary) =>
    ["completed", "failed", "cancelled", "interrupted"].includes(agent.status);
  if (
    !existing ||
    incoming.change_seq > existing.change_seq ||
    (incoming.change_seq === existing.change_seq &&
      terminal(incoming) &&
      !terminal(existing))
  ) {
    agents[incoming.agent_id] = incoming;
  }
}

export const agentsPanelSlice = createSlice({
  name: "agentsPanel",
  reducerPath: "agentsPanel",
  initialState,
  reducers: {
    agentSourceChanged: (state) => {
      state.generation += 1;
      state.disclosure = 0;
      state.acceptedDisclosure = 0;
      state.authorized = false;
      state.denied = false;
      state.failure = null;
      state.liveAgents = {};
      state.transportStale = false;
    },
    agentDisclosureDenied: (
      state,
      action: PayloadAction<{
        source: AgentRosterSource;
        error: FetchBaseQueryError;
      }>,
    ) => {
      if (!matchesGeneration(state, action.payload.source)) return;
      state.disclosure += 1;
      state.authorized = false;
      state.denied = true;
      state.failure = action.payload.error;
      state.liveAgents = {};
    },
    agentRecoveryRequested: (state) => {
      state.denied = false;
    },
    agentRosterAuthorized: (
      state,
      action: PayloadAction<AgentRosterSource>,
    ) => {
      if (!matchesGeneration(state, action.payload) || state.denied) return;
      state.authorized = true;
      state.acceptedDisclosure = state.disclosure;
      state.failure = null;
      state.transportStale = false;
    },
    agentRosterReceived: (
      state,
      action: PayloadAction<{
        source: AgentRosterSource;
        agents: BackgroundAgentSummary[];
      }>,
    ) => {
      if (!matchesGeneration(state, action.payload.source) || state.denied)
        return;
      for (const agent of action.payload.agents)
        mergeAgent(state.liveAgents, agent);
      state.transportStale = false;
    },
    agentRosterTransportFailed: (
      state,
      action: PayloadAction<AgentRosterSource>,
    ) => {
      if (!matchesGeneration(state, action.payload) || state.denied) return;
      state.transportStale = true;
    },
    tabChanged: (state, action: PayloadAction<AgentsPanelTab>) => {
      state.tab = action.payload;
    },
  },
});

export const {
  agentSourceChanged,
  agentDisclosureDenied,
  agentRecoveryRequested,
  agentRosterAuthorized,
  agentRosterReceived,
  agentRosterTransportFailed,
  tabChanged,
} = agentsPanelSlice.actions;

type AgentsPanelRootState = {
  agentsPanel: AgentsPanelState;
};

export const selectAgentsPanelTab = (state: AgentsPanelRootState) =>
  state.agentsPanel.tab;

export const agentRosterIdentity = (state: AgentRosterState): string =>
  JSON.stringify([
    getEngineEndpointIdentity(state.config),
    state.current_project.projectRoot,
    state.current_project.workspaceRoots,
    state.current_project.name,
    state.config.currentWorkspaceName,
    hasUsableEngineEndpoint(state.config),
    state.config.backendReady,
    state.config.connectionStatus,
  ]);

export const selectAgentRosterSource = createSelector(
  [
    agentRosterIdentity,
    (state: AgentRosterState) => state.agentsPanel.generation,
    (state: AgentRosterState) => state.agentsPanel.disclosure,
  ],
  (identity, generation, disclosure): AgentRosterSource => ({
    identity,
    generation,
    disclosure,
  }),
);

export function isCurrentAgentRosterSource(
  state: RootState,
  source: AgentRosterSource,
): boolean {
  return (
    agentRosterIdentity(state) === source.identity &&
    matchesGeneration(state.agentsPanel, source) &&
    hasUsableEngineEndpoint(state.config)
  );
}

export const selectDisclosedAgentPool = (state: RootState) =>
  state.agentsPanel.liveAgents;

export default agentsPanelSlice.reducer;
