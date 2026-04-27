import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import { RootState } from "../../app/store";
import type {
  BuddySnapshot,
  BuddySettings,
  BuddyActivityEntry,
  BuddyConversationMeta,
  BuddyConversationEntry,
} from "../../features/Buddy/types";

export const buddyApi = createApi({
  reducerPath: "buddyApi",
  baseQuery: fetchBaseQuery({
    prepareHeaders: (headers, { getState }) => {
      const token = (getState() as RootState).config.apiKey;
      if (token) {
        headers.set("Authorization", `Bearer ${token}`);
      }
      return headers;
    },
  }),
  endpoints: (builder) => ({
    getBuddySnapshot: builder.query<BuddySnapshot, undefined>({
      queryFn: async (_args, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        const result = await baseQuery(`http://127.0.0.1:${port}/v1/buddy`);
        if (result.error) return { error: result.error };
        return { data: result.data as BuddySnapshot };
      },
    }),
    getBuddySettings: builder.query<BuddySettings, undefined>({
      queryFn: async (_args, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
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
      Partial<BuddySettings>
    >({
      queryFn: async (settings, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
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
    getBuddyActivities: builder.query<BuddyActivityEntry[], undefined>({
      queryFn: async (_args, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
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
        const state = api.getState() as RootState;
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
    createBuddyConversation: builder.mutation<BuddyConversationMeta, undefined>(
      {
        queryFn: async (_args, api, _opts, baseQuery) => {
          const state = api.getState() as RootState;
          const port = state.config.lspPort;
          const result = await baseQuery({
            url: `http://127.0.0.1:${port}/v1/buddy/conversations`,
            method: "POST",
          });
          if (result.error) return { error: result.error };
          return { data: result.data as BuddyConversationMeta };
        },
      },
    ),
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
        const state = api.getState() as RootState;
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
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/buddy/suggestions/${id}/dismiss`,
          method: "POST",
        });
        if (result.error) return { error: result.error };
        return { data: { dismissed: true } };
      },
    }),
    reportError: builder.mutation<
      void,
      {
        error: string;
        source_file?: string;
        tool_name?: string;
        chat_id?: string;
      }
    >({
      queryFn: async (body, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/buddy/diagnostics/collect`,
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: undefined };
      },
    }),
  }),
});

export const {
  useGetBuddySnapshotQuery,
  useGetBuddySettingsQuery,
  useUpdateBuddySettingsMutation,
  useGetBuddyActivitiesQuery,
  useGetBuddyConversationsQuery,
  useCreateBuddyConversationMutation,
  useCreateSetupConversationMutation,
  useDismissBuddySuggestionMutation,
  useReportErrorMutation,
} = buddyApi;
