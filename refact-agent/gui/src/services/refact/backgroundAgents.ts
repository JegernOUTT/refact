import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import type { BaseQueryApi, FetchBaseQueryError } from "@reduxjs/toolkit/query";

import type { RootState } from "../../app/store";
import {
  agentDisclosureDenied,
  agentRosterAuthorized,
  agentRosterReceived,
  agentRosterTransportFailed,
  isCurrentAgentRosterSource,
  selectAgentRosterSource,
  type AgentRosterSource,
} from "../../features/AgentsPanel/agentsPanelSlice";
import { buildApiUrlFromState } from "./apiUrl";
import {
  isValidBackgroundAgent,
  normalizeBackgroundAgentSummary,
  type ChatEventEnvelope,
} from "./chatSubscription";
import type { BackgroundAgentSummary } from "./types";
import { setError } from "../../features/Errors/errorsSlice";
import {
  backgroundAgentErrorText,
  sanitizeBackgroundAgentError,
} from "./backgroundAgentErrors";

export type BackgroundAgentsQuery = {
  chatId: string;
  source: AgentRosterSource;
};

export type CancelBackgroundAgentRequest = BackgroundAgentsQuery & {
  agentId: string;
  subtree?: boolean;
};

export type MessageBackgroundAgentRequest = BackgroundAgentsQuery & {
  agentId: string;
  text: string;
};

const sourceChangedError: FetchBaseQueryError = {
  status: "CUSTOM_ERROR",
  error: "Agent source changed or access is unavailable. Refresh the roster.",
};

export function isAgentDisclosureDenial(error: FetchBaseQueryError): boolean {
  const data: unknown = "data" in error ? error.data : undefined;
  const code =
    typeof data === "object" && data !== null && "code" in data
      ? data.code
      : undefined;
  if (
    typeof code === "string" &&
    /unauthorized|denied|privacy|scope|policy|not_visible/u.test(code)
  )
    return true;
  if (error.status === 409) return false;
  return !(
    error.status === "FETCH_ERROR" ||
    error.status === "TIMEOUT_ERROR" ||
    (typeof error.status === "number" && error.status >= 500)
  );
}

export function isChatAuthorizationDenial(error: Error): boolean {
  return /^SSE connection failed: (401|403)$/u.test(error.message);
}

export const captureAgentRosterConnection =
  () => (_dispatch: BaseQueryApi["dispatch"], getState: () => RootState) => {
    const state = getState();
    return {
      source: selectAgentRosterSource(state),
      config: state.config,
      rosterAllowed:
        !state.agentsPanel.denied &&
        state.agentsPanel.acceptedDisclosure === state.agentsPanel.disclosure,
    };
  };

export const receiveAgentRosterEvent =
  (source: AgentRosterSource, event: ChatEventEnvelope) =>
  (dispatch: BaseQueryApi["dispatch"], getState: () => RootState) => {
    const state = getState();
    if (
      !isCurrentAgentRosterSource(state, source) ||
      state.agentsPanel.denied ||
      state.agentsPanel.acceptedDisclosure !== source.disclosure
    )
      return false;
    if (event.type !== "snapshot" && event.type !== "background_agent_updated")
      return true;
    dispatch(
      agentRosterReceived({
        source,
        agents:
          event.type === "snapshot" ? event.background_agents : [event.agent],
      }),
    );
    return true;
  };

export const rejectAgentRosterStream =
  (source: AgentRosterSource, error: Error) =>
  (dispatch: BaseQueryApi["dispatch"], getState: () => RootState) => {
    const status = /^SSE connection failed: (\d+)$/u.exec(error.message)?.[1];
    const denial =
      status !== undefined
        ? isAgentDisclosureDenial({ status: Number(status), data: null })
        : /invalid|malformed|schema|privacy|policy|scope.denied/iu.test(
            error.message,
          );
    if (denial) {
      denyAgentDisclosure({ dispatch, getState }, source, {
        status: "CUSTOM_ERROR",
        error: "Agent stream access could not be validated.",
      });
    } else if (isCurrentAgentRosterSource(getState(), source)) {
      dispatch(agentRosterTransportFailed(source));
    }
    return denial;
  };

function mutationDenial(
  api: AgentQueryApi,
  source: AgentRosterSource,
  error: FetchBaseQueryError,
) {
  if (!isCurrentAgentRosterSource(api.getState() as RootState, source)) return;
  api.dispatch(setError(backgroundAgentErrorText(error)));
  denyAgentDisclosure(api, source, error);
}

type AgentQueryApi = {
  dispatch: BaseQueryApi["dispatch"];
  getState: () => unknown;
};

function canRequest(api: AgentQueryApi, source: AgentRosterSource): boolean {
  const state = api.getState() as RootState;
  return isCurrentAgentRosterSource(state, source) && !state.agentsPanel.denied;
}

export function denyAgentDisclosure(
  api: AgentQueryApi,
  source: AgentRosterSource,
  _error: FetchBaseQueryError,
) {
  if (!isCurrentAgentRosterSource(api.getState() as RootState, source)) return;
  api.dispatch(
    agentDisclosureDenied({
      source,
      error: {
        status: "CUSTOM_ERROR",
        error: "Agent access must be revalidated.",
      },
    }),
  );
  const state = api.getState() as RootState;
  for (const arg of backgroundAgentsApi.util.selectCachedArgsForQuery(
    state,
    "getBackgroundAgents",
  )) {
    api.dispatch(
      backgroundAgentsApi.util.updateQueryData(
        "getBackgroundAgents",
        arg,
        () => [],
      ),
    );
  }
}

export const backgroundAgentsApi = createApi({
  reducerPath: "backgroundAgentsApi",
  baseQuery: fetchBaseQuery({
    prepareHeaders: (headers, { getState }) => {
      const token = (getState() as RootState).config.apiKey;
      if (token) headers.set("Authorization", `Bearer ${token}`);
      return headers;
    },
  }),
  keepUnusedDataFor: 0,
  tagTypes: ["BackgroundAgents"],
  endpoints: (builder) => ({
    getBackgroundAgents: builder.query<
      BackgroundAgentSummary[],
      BackgroundAgentsQuery
    >({
      queryFn: async ({ chatId, source }, api, _opts, baseQuery) => {
        if (!canRequest(api, source)) return { error: sourceChangedError };
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, "/v1/background-agents", {
            chat_id: chatId,
          }),
        });
        if (!canRequest(api, source)) return { error: sourceChangedError };
        if (result.error) {
          const error = sanitizeBackgroundAgentError(
            result.error,
            (api.getState() as RootState).config.apiKey,
          );
          if (isAgentDisclosureDenial(result.error))
            denyAgentDisclosure(api, source, error);
          return { error };
        }
        if (
          !Array.isArray(result.data) ||
          !result.data.every(isValidBackgroundAgent)
        ) {
          const error: FetchBaseQueryError = {
            status: "CUSTOM_ERROR",
            error: "Invalid response from background agents",
          };
          denyAgentDisclosure(api, source, error);
          return { error };
        }
        api.dispatch(agentRosterAuthorized(source));
        return {
          data: result.data.map(normalizeBackgroundAgentSummary),
        };
      },
      providesTags: (_result, _error, { chatId }) => [
        { type: "BackgroundAgents", id: chatId },
      ],
    }),
    cancelBackgroundAgent: builder.mutation<
      unknown,
      CancelBackgroundAgentRequest
    >({
      queryFn: async (
        { agentId, chatId, source, subtree = true },
        api,
        _opts,
        baseQuery,
      ) => {
        if (
          !canRequest(api, source) ||
          !(api.getState() as RootState).agentsPanel.authorized
        )
          return { error: sourceChangedError };
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(
            state,
            `/v1/background-agents/${encodeURIComponent(agentId)}/cancel`,
          ),
          method: "POST",
          body: { chat_id: chatId, subtree },
        });
        if (!canRequest(api, source)) return { error: sourceChangedError };
        if (result.error) {
          const error = sanitizeBackgroundAgentError(
            result.error,
            (api.getState() as RootState).config.apiKey,
          );
          if (isAgentDisclosureDenial(result.error))
            mutationDenial(api, source, error);
          return { error };
        }
        return { data: result.data };
      },
      invalidatesTags: (_result, error) => (error ? [] : ["BackgroundAgents"]),
    }),
    messageBackgroundAgent: builder.mutation<
      unknown,
      MessageBackgroundAgentRequest
    >({
      queryFn: async (
        { agentId, chatId, source, text },
        api,
        _opts,
        baseQuery,
      ) => {
        if (
          !canRequest(api, source) ||
          !(api.getState() as RootState).agentsPanel.authorized
        )
          return { error: sourceChangedError };
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(
            state,
            `/v1/background-agents/${encodeURIComponent(agentId)}/message`,
          ),
          method: "POST",
          body: { chat_id: chatId, text },
        });
        if (!canRequest(api, source)) return { error: sourceChangedError };
        if (result.error) {
          const error = sanitizeBackgroundAgentError(
            result.error,
            (api.getState() as RootState).config.apiKey,
          );
          if (isAgentDisclosureDenial(result.error))
            mutationDenial(api, source, error);
          return { error };
        }
        return { data: result.data };
      },
      invalidatesTags: (_result, error) => (error ? [] : ["BackgroundAgents"]),
    }),
  }),
});

export const {
  useGetBackgroundAgentsQuery,
  useCancelBackgroundAgentMutation,
  useMessageBackgroundAgentMutation,
} = backgroundAgentsApi;
