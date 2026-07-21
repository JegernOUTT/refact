import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import { RootState } from "../../app/store";
import { buildApiUrlFromState } from "./apiUrl";

export type MCPToolAnnotations = {
  readOnlyHint?: boolean;
  destructiveHint?: boolean;
  idempotentHint?: boolean;
  openWorldHint?: boolean;
  title?: string;
};

export type MCPToolInfo = {
  name: string;
  description: string;
  input_schema: Record<string, unknown>;
  annotations?: MCPToolAnnotations;
  internal_name: string;
};

export type MCPResourceInfo = {
  uri: string;
  name: string;
  description?: string;
  mime_type?: string;
};

export type MCPPromptInfo = {
  name: string;
  description?: string;
};

export type MCPServerCapabilities = {
  tools: boolean;
  resources: boolean;
  prompts: boolean;
  sampling: boolean;
};

export type MCPProgressInfo = {
  token: string;
  progress: number;
  total?: number;
  message?: string;
  updated_at_ms: number;
};

export type MCPAuthStatus =
  | "not_applicable"
  | "authenticated"
  | "needs_login"
  | "needs_reauth"
  | "refreshing"
  | { error: string };

export type MCPServerInfo = {
  config_path: string;
  status: Record<string, unknown>;
  auth_status: MCPAuthStatus;
  server_name?: string;
  server_version?: string;
  protocol_version?: string;
  tools: MCPToolInfo[];
  resources: MCPResourceInfo[];
  prompts: MCPPromptInfo[];
  capabilities: MCPServerCapabilities;
  logs_tail: string[];
  active_progress?: MCPProgressInfo[];
};

export const mcpServerInfoApi = createApi({
  reducerPath: "mcpServerInfoApi",
  tagTypes: ["MCPServerInfo"],
  baseQuery: fetchBaseQuery({
    prepareHeaders: (headers, api) => {
      const getState = api.getState as () => RootState;
      const state = getState();
      const token = state.config.apiKey;
      if (token) {
        headers.set("Authorization", `Bearer ${token}`);
      }
      return headers;
    },
  }),
  endpoints: (builder) => ({
    getMCPServerInfo: builder.query<MCPServerInfo, { configPath: string }>({
      providesTags: (_result, _error, arg) => [
        { type: "MCPServerInfo", id: arg.configPath },
      ],
      async queryFn({ configPath }, api, _extraOptions, baseQuery) {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(state, "/v1/mcp-server-info", {
          config_path: configPath,
        });
        const result = await baseQuery(url);
        if (result.error) return { error: result.error };
        return { data: result.data as MCPServerInfo };
      },
    }),
    reconnectMCPServer: builder.mutation<
      { reconnect_triggered: boolean },
      { configPath: string }
    >({
      invalidatesTags: (_result, _error, arg) => [
        { type: "MCPServerInfo", id: arg.configPath },
      ],
      async queryFn({ configPath }, api, _extraOptions, baseQuery) {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, "/v1/mcp-server-reconnect"),
          method: "POST",
          body: { config_path: configPath },
        });
        if (result.error) return { error: result.error };
        return { data: result.data as { reconnect_triggered: boolean } };
      },
    }),
  }),
});

export const { useGetMCPServerInfoQuery, useReconnectMCPServerMutation } =
  mcpServerInfoApi;
