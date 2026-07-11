import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import { RootState } from "../../app/store";
import { buildApiUrlFromState } from "./apiUrl";

export type MCPInteractionKind = "elicitation" | "sampling_approval";

export type MCPInteraction = {
  id: string;
  config_path: string;
  server_name: string;
  kind: MCPInteractionKind;
  created_at_ms: number;
  timeout_at_ms: number;
  // elicitation
  message?: string;
  requested_schema?: MCPElicitationSchema;
  url?: string;
  elicitation_id?: string;
  // sampling_approval
  message_count?: number;
  max_tokens?: number;
  preview?: string;
};

export type MCPElicitationSchemaProperty = {
  type?: string;
  title?: string;
  description?: string;
  enum?: string[];
  enumNames?: string[];
  default?: string | number | boolean;
  minimum?: number;
  maximum?: number;
  minLength?: number;
  maxLength?: number;
  format?: string;
};

export type MCPElicitationSchema = {
  type?: string;
  properties?: Record<string, MCPElicitationSchemaProperty>;
  required?: string[];
};

export type MCPInteractionAction = "accept" | "decline" | "cancel";

export type MCPInteractionRespondRequest = {
  id: string;
  action: MCPInteractionAction;
  content?: Record<string, unknown>;
};

export const mcpInteractionsApi = createApi({
  reducerPath: "mcpInteractionsApi",
  tagTypes: ["MCP_INTERACTIONS"],
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
    listMcpInteractions: builder.query<
      { interactions: MCPInteraction[] },
      undefined
    >({
      providesTags: ["MCP_INTERACTIONS"],
      async queryFn(_arg, api, _extraOptions, baseQuery) {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(state, "/v1/mcp/interactions");
        const result = await baseQuery(url);
        if (result.error) return { error: result.error };
        return {
          data: result.data as { interactions: MCPInteraction[] },
        };
      },
    }),
    respondMcpInteraction: builder.mutation<
      { success: boolean },
      MCPInteractionRespondRequest
    >({
      invalidatesTags: ["MCP_INTERACTIONS"],
      async queryFn(arg, api, _extraOptions, baseQuery) {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, "/v1/mcp/interactions/respond"),
          method: "POST",
          body: arg,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as { success: boolean } };
      },
    }),
  }),
});

export const { useListMcpInteractionsQuery, useRespondMcpInteractionMutation } =
  mcpInteractionsApi;
