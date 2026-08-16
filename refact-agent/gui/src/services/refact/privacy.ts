import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import { RootState } from "../../app/store";
import type { ChatMessage } from "./types";
import { buildApiUrlFromState } from "./apiUrl";

export type PrivacyShellBehavior = "withhold" | "ask" | "deny";

export type PrivacyZone = {
  name: string;
  patterns: string[];
  send_to: string[];
  on_shell_read: PrivacyShellBehavior;
};

export type PrivacyPolicy = {
  blocked: string[];
  zones: PrivacyZone[];
  subagents: {
    report_declassifies: boolean;
  };
};

export type PrivacyDestinationKind =
  | "provider"
  | "mcp"
  | "subagent_model"
  | "completion";

export type PrivacyDestination = {
  id: string;
  kind: PrivacyDestinationKind;
  display_name: string;
};

export type PrivacyPolicyResponse = {
  policy: PrivacyPolicy;
  destinations: PrivacyDestination[];
  match_counts: Record<string, number>;
  error: string | null;
  source_paths: string[];
};

export type PrivacyObservationCapability = {
  available: boolean;
  reason: string | null;
};

export type PrivacyStatusResponse = {
  platform: string;
  observation: PrivacyObservationCapability;
  config_error: string | null;
};

export type PrivacyAttribution = "declared" | "observed" | "heuristic";

export type PrivacyFileRecord = {
  path: string;
  zone: string;
  attribution: PrivacyAttribution;
};

export type PrivacyInspectRequest = {
  chat_id: string;
  destination: PrivacyDestination;
};

export type PrivacyBlockedRecord = {
  record_index: number;
  record: PrivacyFileRecord;
};

export type PrivacyInspectResponse = {
  chat_id: string;
  destination: PrivacyDestination;
  sendable: boolean;
  would_send: ChatMessage[];
  records: PrivacyFileRecord[];
  blocked: PrivacyBlockedRecord[];
  refusal: string | null;
};

export const privacyApi = createApi({
  reducerPath: "privacyApi",
  tagTypes: ["PRIVACY_POLICY", "PRIVACY_STATUS"],
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
    getPrivacyPolicy: builder.query<PrivacyPolicyResponse, undefined>({
      providesTags: ["PRIVACY_POLICY"],
      async queryFn(_arg, api, extraOptions, baseQuery) {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(state, "/v1/privacy/policy");
        const response = await baseQuery({ url, ...extraOptions });
        if (response.error) return { error: response.error };
        return { data: response.data as PrivacyPolicyResponse };
      },
    }),
    updatePrivacyPolicy: builder.mutation<PrivacyPolicyResponse, PrivacyPolicy>(
      {
        invalidatesTags: ["PRIVACY_POLICY", "PRIVACY_STATUS"],
        async queryFn(policy, api, extraOptions, baseQuery) {
          const state = api.getState() as RootState;
          const url = buildApiUrlFromState(state, "/v1/privacy/policy");
          const response = await baseQuery({
            url,
            method: "POST",
            body: policy,
            ...extraOptions,
          });
          if (response.error) return { error: response.error };
          return { data: response.data as PrivacyPolicyResponse };
        },
      },
    ),
    getPrivacyStatus: builder.query<PrivacyStatusResponse, undefined>({
      providesTags: ["PRIVACY_STATUS"],
      async queryFn(_arg, api, extraOptions, baseQuery) {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(state, "/v1/privacy/status");
        const response = await baseQuery({ url, ...extraOptions });
        if (response.error) return { error: response.error };
        return { data: response.data as PrivacyStatusResponse };
      },
    }),
    inspectPrivacy: builder.mutation<
      PrivacyInspectResponse,
      PrivacyInspectRequest
    >({
      async queryFn(request, api, extraOptions, baseQuery) {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(state, "/v1/privacy/inspect");
        const response = await baseQuery({
          url,
          method: "POST",
          body: request,
          ...extraOptions,
        });
        if (response.error) return { error: response.error };
        return { data: response.data as PrivacyInspectResponse };
      },
    }),
  }),
});

export const {
  useGetPrivacyPolicyQuery,
  useUpdatePrivacyPolicyMutation,
  useGetPrivacyStatusQuery,
  useInspectPrivacyMutation,
} = privacyApi;
