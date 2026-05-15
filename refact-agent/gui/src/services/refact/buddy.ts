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
} from "../../features/Buddy/types";
import {
  addDraft,
  removeDraft,
  replaceOpportunities,
} from "../../features/Buddy/buddySlice";

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

export interface FrontendErrorReport {
  error: string;
  source_file?: string;
  tool_name?: string;
  chat_id?: string;
}

export type UserActionEntry = { type: string; ts: string };

export type UserActivityResponse = {
  actions: UserActionEntry[];
  time_of_day_pattern: string;
};

export type UserActionPayload =
  | { type: "file_opened"; path: string; ts: string }
  | {
      type: "snippet_selected";
      path: string;
      lines: [number, number];
      ts: string;
    }
  | { type: "tool_approved"; tool_name: string; chat_id: string; ts: string }
  | { type: "tool_rejected"; tool_name: string; chat_id: string; ts: string }
  | {
      type: "command_run";
      command_preview: string;
      chat_id: string;
      ts: string;
    }
  | {
      type: "workspace_changed";
      folders_added: string[];
      folders_removed: string[];
      ts: string;
    }
  | {
      type: "commit_made";
      sha: string;
      message_first_line: string;
      files: number;
      ts: string;
    }
  | { type: "task_failed"; task_id: string; reason_short: string; ts: string }
  | {
      type: "chat_started";
      chat_id: string;
      first_user_text_preview: string;
      ts: string;
    };

export interface BuddyOpportunityDismissResponse {
  snapshot: BuddySnapshot;
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

const FRONTEND_SOURCE_PATTERNS: [RegExp, string][] = [
  [/Bearer\s+[^\s"'`]+/gi, "Bearer [REDACTED]"],
  [/sk-[A-Za-z0-9]{20,}/g, "[REDACTED_SK_TOKEN]"],
  [/\bghp_[A-Za-z0-9]{10,}\b/g, "[REDACTED_GH_TOKEN]"],
  [/\bglpat-[A-Za-z0-9_-]{10,}\b/g, "[REDACTED_GL_TOKEN]"],
  [
    /\b(api[_-]?key|token|secret|password)\s*[:=]\s*[^\s,;]+/gi,
    "$1=[REDACTED]",
  ],
  [/(https?:\/\/[^\s?#]+)\?[^\s)\]]+/gi, "$1?[REDACTED]"],
  [/file:\/\/[^\s)\]]+/gi, "file://[REDACTED_PATH]"],
  [/[A-Za-z]:\\[^\s)\]]+/g, "[REDACTED_PATH]"],
  [/\/(?:Users|home)\/[^\s)]+/g, "[REDACTED_PATH]"],
];

function redactFrontendSource(value: string | undefined): string | undefined {
  if (!value) return undefined;
  const redacted = FRONTEND_SOURCE_PATTERNS.reduce(
    (current, [pattern, replacement]) => current.replace(pattern, replacement),
    value,
  ).trim();
  return redacted || undefined;
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
  tagTypes: [
    "BuddySnapshot",
    "BuddyOpportunities",
    "BuddyPulse",
    "BuddyDrafts",
  ],
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
      providesTags: ["BuddySnapshot"],
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
      providesTags: ["BuddySnapshot"],
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
      invalidatesTags: ["BuddySnapshot"],
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
        const data = result.data as { opportunities: BuddyOpportunity[] };
        return { data: data.opportunities };
      },
      providesTags: ["BuddyOpportunities"],
      async onQueryStarted(_arg, { dispatch, queryFulfilled }) {
        try {
          const { data } = await queryFulfilled;
          dispatch(replaceOpportunities(data));
        } catch {
          return;
        }
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
    dismissOpportunity: builder.mutation<
      BuddyOpportunityDismissResponse,
      string
    >({
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
        return { data: result.data as BuddyOpportunityDismissResponse };
      },
      invalidatesTags: ["BuddyOpportunities", "BuddySnapshot"],
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
      providesTags: ["BuddyPulse"],
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
      invalidatesTags: ["BuddyDrafts", "BuddySnapshot"],
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
      invalidatesTags: ["BuddyDrafts", "BuddySnapshot"],
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
      invalidatesTags: ["BuddyDrafts", "BuddySnapshot"],
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
      invalidatesTags: ["BuddyDrafts", "BuddySnapshot"],
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
      invalidatesTags: ["BuddyDrafts", "BuddySnapshot"],
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
      invalidatesTags: ["BuddyDrafts", "BuddySnapshot"],
    }),
    createHookDraft: builder.mutation<BuddyDraft, CreateDraftRequest>({
      queryFn: async (body, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyApiState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/buddy/drafts/hook`,
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as BuddyDraft };
      },
      invalidatesTags: ["BuddyDrafts", "BuddySnapshot"],
      async onQueryStarted(_arg, { dispatch, queryFulfilled }) {
        try {
          const { data } = await queryFulfilled;
          dispatch(addDraft(data));
        } catch {
          return;
        }
      },
    }),
    createPulseReportDraft: builder.mutation<BuddyDraft, CreateDraftRequest>({
      queryFn: async (body, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyApiState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/buddy/drafts/pulse_report`,
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as BuddyDraft };
      },
      invalidatesTags: ["BuddyDrafts", "BuddySnapshot"],
      async onQueryStarted(_arg, { dispatch, queryFulfilled }) {
        try {
          const { data } = await queryFulfilled;
          dispatch(addDraft(data));
        } catch {
          return;
        }
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
      providesTags: ["BuddyDrafts"],
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
      invalidatesTags: ["BuddyDrafts", "BuddySnapshot"],
      async onQueryStarted(id, { dispatch, queryFulfilled }) {
        try {
          await queryFulfilled;
          dispatch(removeDraft(id));
        } catch {
          return;
        }
      },
    }),
    postUserAction: builder.mutation<undefined, UserActionPayload>({
      queryFn: async (action, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyApiState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/buddy/user_action`,
          method: "POST",
          body: action,
        });
        if (result.error) return { error: result.error };
        return { data: undefined };
      },
    }),
    getUserActivity: builder.query<UserActivityResponse, { hours?: number }>({
      queryFn: async (args, api, _opts, baseQuery) => {
        const state = api.getState() as BuddyApiState;
        const port = state.config.lspPort;
        const result = await baseQuery(
          `http://127.0.0.1:${port}/v1/buddy/user_activity?hours=${
            args.hours ?? 24
          }`,
        );
        if (result.error) return { error: result.error };
        return { data: result.data as UserActivityResponse };
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
          const response = await fetch(
            `http://127.0.0.1:${port}/v1/buddy/frontend-error`,
            {
              method: "POST",
              headers,
              body: JSON.stringify({
                message: body.error,
                stack: "",
                url:
                  redactFrontendSource(body.source_file) ??
                  "frontend/report_frontend_error",
                kind: redactFrontendSource(body.tool_name) ?? "frontend",
                chat_id: body.chat_id,
              }),
            },
          );
          await parseBuddyResponse<unknown>(response);
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
  useCreateHookDraftMutation,
  useCreatePulseReportDraftMutation,
  useGetDraftQuery,
  useDeleteDraftMutation,
  usePostUserActionMutation,
  useGetUserActivityQuery,
  useReportFrontendErrorMutation,
} = buddyApi;
