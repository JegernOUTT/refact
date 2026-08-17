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

export type ResolvedPrivacyZone = Omit<PrivacyZone, "patterns">;

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
  platform_supported: boolean;
  runtime_available: boolean;
  last_error: string | null;
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
  records: PrivacyFileRecord[];
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

export type PrivacyRecordMetadata = {
  files: PrivacyFileRecord[];
};

export type PrivacyShellMetadata = {
  withheld: true;
  local_only_output: string;
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isPrivacyFileRecord(value: unknown): value is PrivacyFileRecord {
  return (
    isRecord(value) &&
    typeof value.path === "string" &&
    typeof value.zone === "string" &&
    (value.attribution === "declared" ||
      value.attribution === "observed" ||
      value.attribution === "heuristic")
  );
}

export function extractPrivacyFiles(
  extra: Record<string, unknown> | undefined,
): PrivacyFileRecord[] {
  const privacy = extra?.privacy;
  if (!isRecord(privacy) || !Array.isArray(privacy.files)) return [];
  return privacy.files.filter(isPrivacyFileRecord);
}

export function extractPrivacyShellMetadata(
  extra: Record<string, unknown> | undefined,
): PrivacyShellMetadata | null {
  const shell = extra?.privacy_shell;
  if (
    !isRecord(shell) ||
    shell.withheld !== true ||
    typeof shell.local_only_output !== "string"
  ) {
    return null;
  }
  return {
    withheld: true,
    local_only_output: shell.local_only_output,
  };
}

export function privacyDestinationForModel(model: string): PrivacyDestination {
  const separator = model.indexOf("/");
  return {
    id: separator === -1 ? model : model.slice(0, separator),
    kind: "provider",
    display_name: model,
  };
}

export function blockedPrivacyFilesFromInspection(
  files: PrivacyFileRecord[],
  inspection: PrivacyInspectResponse | undefined,
): PrivacyFileRecord[] {
  if (!inspection) return files;
  const blocked = new Set(
    inspection.blocked.map(({ record }) =>
      [record.path, record.zone, record.attribution].join("\u0000"),
    ),
  );
  return files.filter((file) =>
    blocked.has([file.path, file.zone, file.attribution].join("\u0000")),
  );
}

export function isPrivacyRefusalContent(content: unknown): boolean {
  return (
    typeof content === "string" &&
    content.startsWith("Output withheld by user privacy policy")
  );
}

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
    inspectPrivacy: builder.query<
      PrivacyInspectResponse,
      PrivacyInspectRequest
    >({
      providesTags: ["PRIVACY_POLICY"],
      async queryFn(request, api, extraOptions, baseQuery) {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(state, "/v1/privacy/inspect");
        const response = await baseQuery({
          url,
          method: "POST",
          body: {
            chat_id: request.chat_id,
            destination: request.destination,
          },
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
  useInspectPrivacyQuery,
} = privacyApi;
