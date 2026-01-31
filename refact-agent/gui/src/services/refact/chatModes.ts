import { RootState } from "../../app/store";
import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";

export type ChatModeThreadDefaults = {
  include_project_info: boolean;
  checkpoints_enabled: boolean;
  auto_approve_editing_tools: boolean;
  auto_approve_dangerous_commands: boolean;
};

export type ChatModeUi = {
  order: number;
  tags: string[];
};

export type ChatModeInfo = {
  id: string;
  title: string;
  description: string;
  tools_count: number;
  thread_defaults: ChatModeThreadDefaults;
  ui: ChatModeUi;
};

export type ChatModeError = {
  file_path: string;
  error: string;
};

export type ChatModesResponse = {
  modes: ChatModeInfo[];
  errors: ChatModeError[];
};

export const chatModesApi = createApi({
  reducerPath: "chatModes",
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
    getChatModes: builder.query<ChatModesResponse, undefined>({
      queryFn: async (_args, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        if (!port) {
          return { error: { status: 500, data: "Missing lspPort in config" } };
        }

        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/chat-modes`,
          credentials: "same-origin",
          redirect: "follow",
        });

        if (result.error) {
          return { error: result.error };
        }

        return { data: result.data as ChatModesResponse };
      },
    }),
  }),
  refetchOnMountOrArgChange: true,
});

export const { useGetChatModesQuery } = chatModesApi;
