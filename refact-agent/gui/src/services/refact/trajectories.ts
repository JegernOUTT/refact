import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import { ChatThread } from "../../features/Chat/Thread/types";
import { ChatMessages } from "./types";
import { RootState } from "../../app/store";

export type TrajectoryMeta = {
  id: string;
  title: string;
  created_at: string;
  updated_at: string;
  model: string;
  mode: string;
  message_count: number;
  parent_id?: string;
  link_type?: string;
  task_id?: string;
  task_role?: string;
  agent_id?: string;
  card_id?: string;
  session_state?:
    | "idle"
    | "generating"
    | "executing_tools"
    | "paused"
    | "waiting_ide"
    | "waiting_user_input"
    | "completed"
    | "error";
  root_chat_id?: string;
  total_coins?: number;
  total_lines_added: number;
  total_lines_removed: number;
  tasks_total: number;
  tasks_done: number;
  tasks_failed: number;
};

export type TrajectoryData = {
  id: string;
  title: string;
  created_at: string;
  updated_at: string;
  model: string;
  mode: string;
  tool_use: string;
  messages: ChatMessages;
  boost_reasoning?: boolean;
  context_tokens_cap?: number;
  include_project_info?: boolean;
  increase_max_tokens?: boolean;
  project_name?: string;
  isTitleGenerated?: boolean;
};

export type TrajectoryEvent = {
  type: "created" | "updated" | "deleted";
  id: string;
  updated_at?: string;
  title?: string;
  is_title_generated?: boolean;
  session_state?:
    | "idle"
    | "generating"
    | "executing_tools"
    | "paused"
    | "waiting_ide"
    | "waiting_user_input"
    | "completed"
    | "error";
  error?: string;
  message_count?: number;
  parent_id?: string;
  link_type?: string;
  root_chat_id?: string;
  model?: string;
  mode?: string;
  total_coins?: number;
  total_lines_added?: number;
  total_lines_removed?: number;
  tasks_total?: number;
  tasks_done?: number;
  tasks_failed?: number;
};

export type PaginatedTrajectories = {
  items: TrajectoryMeta[];
  next_cursor: string | null;
  has_more: boolean;
  total_count: number;
};

export type TrajectoriesListParams = {
  limit?: number;
  cursor?: string;
};

export function chatThreadToTrajectoryData(
  thread: ChatThread,
  createdAt?: string,
): TrajectoryData {
  const now = new Date().toISOString();
  return {
    id: thread.id,
    title: thread.title ?? "New Chat",
    created_at: createdAt ?? now,
    updated_at: now,
    model: thread.model,
    mode: thread.mode ?? "AGENT",
    tool_use: thread.tool_use ?? "agent",
    messages: thread.messages,
    boost_reasoning: thread.boost_reasoning,
    context_tokens_cap: thread.context_tokens_cap,
    include_project_info: thread.include_project_info,
    increase_max_tokens: thread.increase_max_tokens,
    project_name: thread.project_name,
    isTitleGenerated: thread.isTitleGenerated,
  };
}

export function trajectoryDataToChatThread(data: TrajectoryData): ChatThread {
  return {
    id: data.id,
    title: data.title,
    model: data.model,
    mode: data.mode as ChatThread["mode"],
    tool_use: data.tool_use as ChatThread["tool_use"],
    messages: data.messages,
    boost_reasoning: data.boost_reasoning ?? false,
    context_tokens_cap: data.context_tokens_cap,
    include_project_info: data.include_project_info ?? true,
    increase_max_tokens: data.increase_max_tokens ?? false,
    project_name: data.project_name,
    isTitleGenerated: data.isTitleGenerated,
    createdAt: data.created_at,
    last_user_message_id: "",
    new_chat_suggested: { wasSuggested: false },
  };
}

export const trajectoriesApi = createApi({
  reducerPath: "trajectoriesApi",
  baseQuery: fetchBaseQuery({
    prepareHeaders: (headers, { getState }) => {
      const token = (getState() as RootState).config.apiKey;
      if (token) {
        headers.set("Authorization", `Bearer ${token}`);
      }
      return headers;
    },
  }),
  tagTypes: ["Trajectory"],
  endpoints: (builder) => ({
    listTrajectoriesFirstPage: builder.query<TrajectoryMeta[], undefined>({
      queryFn: async (_args, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        const url = `http://127.0.0.1:${port}/v1/trajectories`;
        const result = await baseQuery({ url });
        if (result.error) return { error: result.error };
        const response = result.data as PaginatedTrajectories;
        return { data: response.items };
      },
      providesTags: ["Trajectory"],
    }),
    listTrajectoriesPaginated: builder.query<
      PaginatedTrajectories,
      TrajectoriesListParams | undefined
    >({
      queryFn: async (args, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        const params = new URLSearchParams();
        if (args?.limit) params.set("limit", String(args.limit));
        if (args?.cursor) params.set("cursor", args.cursor);
        const queryString = params.toString();
        const url = `http://127.0.0.1:${port}/v1/trajectories${
          queryString ? `?${queryString}` : ""
        }`;
        const result = await baseQuery({ url });
        if (result.error) return { error: result.error };
        return { data: result.data as PaginatedTrajectories };
      },
      providesTags: ["Trajectory"],
    }),
    listAllTrajectories: builder.query<TrajectoryMeta[], undefined>({
      queryFn: async (_args, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        const url = `http://127.0.0.1:${port}/v1/trajectories/all`;
        const result = await baseQuery({ url });
        if (result.error) return { error: result.error };
        return { data: result.data as TrajectoryMeta[] };
      },
      providesTags: ["Trajectory"],
    }),
    getTrajectory: builder.query<TrajectoryData, string>({
      queryFn: async (id, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        const url = `http://127.0.0.1:${port}/v1/trajectories/${id}`;
        const result = await baseQuery({ url });
        if (result.error) return { error: result.error };
        return { data: result.data as TrajectoryData };
      },
      providesTags: (_result, _error, id) => [{ type: "Trajectory", id }],
    }),
    saveTrajectory: builder.mutation<undefined, TrajectoryData>({
      queryFn: async (data, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        const url = `http://127.0.0.1:${port}/v1/trajectories/${data.id}`;
        const result = await baseQuery({
          url,
          method: "PUT",
          body: data,
        });
        if (result.error) return { error: result.error };
        return { data: undefined };
      },
      invalidatesTags: (_result, _error, data) => [
        { type: "Trajectory", id: data.id },
        "Trajectory",
      ],
    }),
    deleteTrajectory: builder.mutation<undefined, string>({
      queryFn: async (id, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        const url = `http://127.0.0.1:${port}/v1/trajectories/${id}`;
        const result = await baseQuery({
          url,
          method: "DELETE",
        });
        if (result.error) return { error: result.error };
        return { data: undefined };
      },
      invalidatesTags: ["Trajectory"],
    }),
  }),
});

export const {
  useListTrajectoriesFirstPageQuery,
  useListTrajectoriesPaginatedQuery,
  useListAllTrajectoriesQuery,
  useGetTrajectoryQuery,
  useSaveTrajectoryMutation,
  useDeleteTrajectoryMutation,
} = trajectoriesApi;
