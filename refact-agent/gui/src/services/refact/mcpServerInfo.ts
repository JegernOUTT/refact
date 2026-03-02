import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import { RootState } from "../../app/store";

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

export type MCPServerInfo = {
  config_path: string;
  status: Record<string, unknown>;
  server_name?: string;
  server_version?: string;
  protocol_version?: string;
  tools: MCPToolInfo[];
  resources: MCPResourceInfo[];
  prompts: MCPPromptInfo[];
  capabilities: MCPServerCapabilities;
  logs_tail: string[];
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
    getMCPServerInfo: builder.query<MCPServerInfo, { configPath: string; port: number }>({
      providesTags: (_result, _error, arg) => [
        { type: "MCPServerInfo", id: arg.configPath },
      ],
      async queryFn({ configPath, port }, _api, _extraOptions, baseQuery) {
        const url = `http://127.0.0.1:${port}/v1/mcp-server-info?config_path=${encodeURIComponent(configPath)}`;
        const result = await baseQuery(url);
        if (result.error) return { error: result.error };
        return { data: result.data as MCPServerInfo };
      },
    }),
    reconnectMCPServer: builder.mutation<{ reconnect_triggered: boolean }, { configPath: string; port: number }>({
      invalidatesTags: (_result, _error, arg) => [
        { type: "MCPServerInfo", id: arg.configPath },
      ],
      async queryFn({ configPath, port }, _api, _extraOptions, baseQuery) {
        const url = `http://127.0.0.1:${port}/v1/mcp-server-reconnect`;
        const result = await baseQuery({
          url,
          method: "POST",
          body: { config_path: configPath },
        });
        if (result.error) return { error: result.error };
        return { data: result.data as { reconnect_triggered: boolean } };
      },
    }),
  }),
});

export const {
  useGetMCPServerInfoQuery,
  useReconnectMCPServerMutation,
} = mcpServerInfoApi;
