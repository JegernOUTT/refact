import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import type {
  BuddySnapshot,
  BuddySettings,
  BuddyActivityEntry,
  BuddyConversationMeta,
  BuddyConversationEntry,
  BuddyCareRequest,
  BuddyCareResponse,
  BuddyQuestAcceptResponse,
  BuddyPersonalityRerollResponse,
  BuddyOpportunity,
  OpportunityStatus,
  BuddyPulse,
  BuddyDraft,
  BuddyOpportunityAcceptResponse,
  InvestigationContext,
} from "../../features/Buddy/types";

type BuddyApiState = {
  config: {
    apiKey: string | null;
    lspPort: number;
  };
};

type BuddySettingsUpdateRequest = Partial<BuddySettings> & {
  clear_personality_prompt?: boolean;
};

export type BuddyConversationCreateRequest = {
  title?: string;
};

export type BuddyErrorReport = {
  error: string;
  source_file?: string;
  tool_name?: string;
  chat_id?: string;
  diagnostic_id?: string;
  collected_at?: string;
};

export type BuddyInvestigationContextRequest = BuddyErrorReport;

export interface CreateDraftRequest {
  title: string;
  yaml_or_json: string;
  explanation: string;
}

export interface LaunchInvestigationResponse {
  chat_id: string;
}

export interface FrontendErrorReport {
  error: string;
  source_file?: string;
  tool_name?: string;
  chat_id?: string;
}

export interface BuddyInvestigationContextResponse {
  logs: string;
  internal_context: string;
  repo_owner: string;
  repo_name: string;
}

type BuddySnapshotResponse = Partial<BuddySnapshot> & { enabled?: boolean };

function makeHeaders(apiKey: string | undefined, includeJson = true): Headers {
  const headers = new Headers();
  if (includeJson) {
    headers.set("Content-Type", "application/json");
  }
  if (apiKey) {
    headers.set("Authorization", `Bearer ${apiKey}`);
  }
  return headers;
}

async function parseBuddyResponse<T>(response: Response): Promise<T> {
  if (!response.ok) {
    const text = await response.text();
    throw new Error(`${response.status} ${response.statusText}: ${text}`);
  }
  return (await response.json()) as T;
}

export async function createBuddyConversationRequest(
  port: number,
  apiKey: string | undefined,
  body?: BuddyConversationCreateRequest,
): Promise<BuddyConversationMeta> {
  const response = await fetch(
    `http://127.0.0.1:${port}/v1/buddy/conversations`,
    {
      method: "POST",
      headers: makeHeaders(apiKey, !!body),
      body: body ? JSON.stringify(body) : undefined,
    },
  );
  return parseBuddyResponse<BuddyConversationMeta>(response);
}

export async function postBuddyErrorRequest(
  port: number,
  apiKey: string | undefined,
  body: BuddyErrorReport,
): Promise<void> {
  const response = await fetch(
    `http://127.0.0.1:${port}/v1/buddy/diagnostics/collect`,
    {
      method: "POST",
      headers: makeHeaders(apiKey),
      body: JSON.stringify(body),
    },
  );
  await parseBuddyResponse(response);
}

export async function fetchBuddyInvestigationContextRequest(
  port: number,
  apiKey: string | undefined,
  body: BuddyInvestigationContextRequest,
): Promise<BuddyInvestigationContextResponse> {
  const response = await fetch(
    `http://127.0.0.1:${port}/v1/buddy/investigation-context`,
    {
      method: "POST",
      headers: makeHeaders(apiKey),
      body: JSON.stringify(body),
    },
  );
  return parseBuddyResponse<BuddyInvestigationContextResponse>(response);
}

export const buddyApi = createApi({
  reducerPath: "buddyApi",
  tagTypes: ["BuddyOpportunities", "BuddySnapshot"],
  baseQuery: fetchBaseQuery({
    prepareHeaders: (headers, { getState }) => {
      const token = (getState() as BuddyApiState).config.apiKey;
      if (token) {
        headers.set("Authorization", `Bearer ${token}`);
      }
      return headers;
    },
  }),
  endpoints: (builder) => ({
    getBuddySnapshot: builder.query<BuddySnapshot, undefined>({
      queryFn: async (_args, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyApiState;
        const port = state.config.lspPort;
        const result = await baseQuery(`http://127.0.0.1:${port}/v1/buddy`);
        if (result.error) return { error: result.error };
        return { data: result.data as BuddySnapshotResponse as BuddySnapshot };
      },
    }),
    getBuddySettings: builder.query<BuddySettings, undefined>({
      queryFn: async (_args, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyApiState;
        const port = state.config.lspPort;
        const result = await baseQuery(
          `http://127.0.0.1:${port}/v1/buddy/settings`,
        );
        if (result.error) return { error: result.error };
        return { data: result.data as BuddySettings };
      },
    }),
    updateBuddySettings: builder.mutation<
      BuddySettings,
      BuddySettingsUpdateRequest
    >({
      queryFn: async (settings, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyApiState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/buddy/settings`,
          method: "POST",
          body: settings,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as BuddySettings };
      },
    }),
    careBuddy: builder.mutation<BuddyCareResponse, BuddyCareRequest>({
      queryFn: async (body, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyApiState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/buddy/care`,
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as BuddyCareResponse };
      },
    }),
    acceptBuddyQuest: builder.mutation<BuddyQuestAcceptResponse, string>({
      queryFn: async (suggestionId, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyApiState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/buddy/quest/accept`,
          method: "POST",
          body: { suggestion_id: suggestionId },
        });
        if (result.error) return { error: result.error };
        return { data: result.data as BuddyQuestAcceptResponse };
      },
    }),
    rerollBuddyPersonality: builder.mutation<
      BuddyPersonalityRerollResponse,
      undefined
    >({
      queryFn: async (_body, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyApiState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/buddy/personality/reroll`,
          method: "POST",
        });
        if (result.error) return { error: result.error };
        return { data: result.data as BuddyPersonalityRerollResponse };
      },
    }),
    getBuddyActivities: builder.query<BuddyActivityEntry[], undefined>({
      queryFn: async (_args, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyApiState;
        const port = state.config.lspPort;
        const result = await baseQuery(
          `http://127.0.0.1:${port}/v1/buddy/activities`,
        );
        if (result.error) return { error: result.error };
        return { data: result.data as BuddyActivityEntry[] };
      },
    }),
    getBuddyConversations: builder.query<
      BuddyConversationEntry[],
      { kind?: string } | undefined
    >({
      queryFn: async (args, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyApiState;
        const port = state.config.lspPort;
        const kind = args?.kind;
        const url = kind
          ? `http://127.0.0.1:${port}/v1/buddy/conversations?kind=${encodeURIComponent(
              kind,
            )}`
          : `http://127.0.0.1:${port}/v1/buddy/conversations`;
        const result = await baseQuery(url);
        if (result.error) return { error: result.error };
        return { data: result.data as BuddyConversationEntry[] };
      },
    }),
    createBuddyConversation: builder.mutation<
      BuddyConversationMeta,
      BuddyConversationCreateRequest | undefined
    >({
      queryFn: async (args, api) => {
        const state = api.getState() as BuddyApiState;
        const port: number = state.config.lspPort;
        const apiKey: string | undefined = state.config.apiKey ?? undefined;
        try {
          return {
            data: await createBuddyConversationRequest(port, apiKey, args),
          };
        } catch (error) {
          return {
            error: {
              status: "FETCH_ERROR",
              error: error instanceof Error ? error.message : String(error),
            },
          };
        }
      },
    }),
    createSetupConversation: builder.mutation<
      {
        chat_id: string;
        title: string;
        kind: string;
        flow: string;
        badge: string;
        created_at: string;
      },
      { flow: string; title?: string }
    >({
      queryFn: async (body, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyApiState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/buddy/conversations/setup`,
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return {
          data: result.data as {
            chat_id: string;
            title: string;
            kind: string;
            flow: string;
            badge: string;
            created_at: string;
          },
        };
      },
    }),
    dismissBuddySuggestion: builder.mutation<{ dismissed: boolean }, string>({
      queryFn: async (id, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyApiState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/buddy/suggestions/${id}/dismiss`,
          method: "POST",
        });
        if (result.error) return { error: result.error };
        return { data: { dismissed: true } };
      },
    }),
    dismissBuddyRuntimeEvent: builder.mutation<{ dismissed: boolean }, string>({
      queryFn: async (id, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyApiState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/buddy/runtime/${encodeURIComponent(
            id,
          )}/dismiss`,
          method: "POST",
        });
        if (result.error) return { error: result.error };
        const data = result.data as { dismissed?: boolean } | undefined;
        return { data: { dismissed: data?.dismissed ?? true } };
      },
    }),
    reportError: builder.mutation<undefined, BuddyErrorReport>({
      queryFn: async (body, api) => {
        const state = api.getState() as BuddyApiState;
        const port: number = state.config.lspPort;
        const apiKey: string | undefined = state.config.apiKey ?? undefined;
        try {
          await postBuddyErrorRequest(port, apiKey, body);
          return { data: undefined };
        } catch (error) {
          return {
            error: {
              status: "FETCH_ERROR",
              error: error instanceof Error ? error.message : String(error),
            },
          };
        }
      },
    }),
    getOpportunities: builder.query<
      BuddyOpportunity[],
      { status?: OpportunityStatus } | undefined
    >({
      queryFn: async (args, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyApiState;
        const port = state.config.lspPort;
        const url = args?.status
          ? `http://127.0.0.1:${port}/v1/buddy/opportunities?status=${encodeURIComponent(
              args.status,
            )}`
          : `http://127.0.0.1:${port}/v1/buddy/opportunities`;
        const result = await baseQuery(url);
        if (result.error) return { error: result.error };
        return { data: result.data as BuddyOpportunity[] };
      },
    }),
    acceptOpportunity: builder.mutation<
      BuddyOpportunityAcceptResponse,
      { id: string; action_index: number }
    >({
      queryFn: async ({ id, action_index }, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyApiState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/buddy/opportunities/${encodeURIComponent(
            id,
          )}/accept`,
          method: "POST",
          body: { action_index },
        });
        if (result.error) return { error: result.error };
        return { data: result.data as BuddyOpportunityAcceptResponse };
      },
      invalidatesTags: ["BuddyOpportunities", "BuddySnapshot"],
    }),
    dismissOpportunity: builder.mutation<{ dismissed: boolean }, string>({
      queryFn: async (id, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyApiState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/buddy/opportunities/${encodeURIComponent(
            id,
          )}/dismiss`,
          method: "POST",
        });
        if (result.error) return { error: result.error };
        return { data: { dismissed: true } };
      },
    }),
    getPulse: builder.query<BuddyPulse, undefined>({
      queryFn: async (_args, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyApiState;
        const port = state.config.lspPort;
        const result = await baseQuery(
          `http://127.0.0.1:${port}/v1/buddy/pulse`,
        );
        if (result.error) return { error: result.error };
        return { data: result.data as BuddyPulse };
      },
    }),
    createSkillDraft: builder.mutation<BuddyDraft, CreateDraftRequest>({
      queryFn: async (body, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyApiState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/buddy/drafts/skill`,
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as BuddyDraft };
      },
    }),
    createCommandDraft: builder.mutation<BuddyDraft, CreateDraftRequest>({
      queryFn: async (body, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyApiState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/buddy/drafts/command`,
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as BuddyDraft };
      },
    }),
    createSubagentDraft: builder.mutation<BuddyDraft, CreateDraftRequest>({
      queryFn: async (body, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyApiState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/buddy/drafts/subagent`,
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as BuddyDraft };
      },
    }),
    createModeDraft: builder.mutation<BuddyDraft, CreateDraftRequest>({
      queryFn: async (body, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyApiState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/buddy/drafts/mode`,
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as BuddyDraft };
      },
    }),
    createAgentsMdDraft: builder.mutation<BuddyDraft, CreateDraftRequest>({
      queryFn: async (body, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyApiState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/buddy/drafts/agents_md`,
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as BuddyDraft };
      },
    }),
    createDefaultsDraft: builder.mutation<BuddyDraft, CreateDraftRequest>({
      queryFn: async (body, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyApiState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/buddy/drafts/defaults`,
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as BuddyDraft };
      },
    }),
    getDraft: builder.query<BuddyDraft, string>({
      queryFn: async (id, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyApiState;
        const port = state.config.lspPort;
        const result = await baseQuery(
          `http://127.0.0.1:${port}/v1/buddy/drafts/${encodeURIComponent(id)}`,
        );
        if (result.error) return { error: result.error };
        return { data: result.data as BuddyDraft };
      },
    }),
    deleteDraft: builder.mutation<undefined, string>({
      queryFn: async (id, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyApiState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/buddy/drafts/${encodeURIComponent(
            id,
          )}`,
          method: "DELETE",
        });
        if (result.error) return { error: result.error };
        return { data: undefined };
      },
    }),
    launchInvestigation: builder.mutation<
      LaunchInvestigationResponse,
      InvestigationContext
    >({
      queryFn: async (body, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyApiState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/buddy/investigations`,
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as LaunchInvestigationResponse };
      },
    }),
    reportFrontendError: builder.mutation<undefined, FrontendErrorReport>({
      queryFn: async (body, api) => {
        const state = api.getState() as BuddyApiState;
        const port: number = state.config.lspPort;
        const apiKey: string | undefined = state.config.apiKey ?? undefined;
        try {
          const headers = new Headers({ "Content-Type": "application/json" });
          if (apiKey) headers.set("Authorization", `Bearer ${apiKey}`);
          await fetch(`http://127.0.0.1:${port}/v1/buddy/frontend-error`, {
            method: "POST",
            headers,
            body: JSON.stringify(body),
          });
          return { data: undefined };
        } catch (error) {
          return {
            error: {
              status: "FETCH_ERROR",
              error: error instanceof Error ? error.message : String(error),
            },
          };
        }
      },
    }),
  }),
});

export const {
  useGetBuddySnapshotQuery,
  useGetBuddySettingsQuery,
  useUpdateBuddySettingsMutation,
  useCareBuddyMutation,
  useAcceptBuddyQuestMutation,
  useRerollBuddyPersonalityMutation,
  useGetBuddyActivitiesQuery,
  useGetBuddyConversationsQuery,
  useCreateBuddyConversationMutation,
  useCreateSetupConversationMutation,
  useDismissBuddySuggestionMutation,
  useDismissBuddyRuntimeEventMutation,
  useReportErrorMutation,
  useGetOpportunitiesQuery,
  useAcceptOpportunityMutation,
  useDismissOpportunityMutation,
  useGetPulseQuery,
  useCreateSkillDraftMutation,
  useCreateCommandDraftMutation,
  useCreateSubagentDraftMutation,
  useCreateModeDraftMutation,
  useCreateAgentsMdDraftMutation,
  useCreateDefaultsDraftMutation,
  useGetDraftQuery,
  useDeleteDraftMutation,
  useLaunchInvestigationMutation,
  useReportFrontendErrorMutation,
} = buddyApi;
