import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";

import type { RootState } from "../../app/store";
import { buildApiUrlFromState } from "./apiUrl";

export type ShellApprovalMode = "strict" | "balanced" | "permissive" | "yolo";
export type ShellLlmAuthority = "ask_only" | "ask_and_allow";
export type ShellLlmOnFailure = "pass" | "ask";
export type ShellRiskLevel = "low" | "medium" | "high" | "critical";
export type ShellGateDecision = "pass" | "confirmation" | "deny";

export type ShellCommandTestResult = {
  decision: ShellGateDecision;
  rule: string;
  reason: string;
  risk_level: ShellRiskLevel | null;
  segments: string[];
};

export type ShellAuditEntry = {
  ts_ms: number;
  chat_id: string;
  command: string;
  decision: ShellGateDecision;
  layer: string;
  rule: string;
  risk_level: ShellRiskLevel | null;
};

export type ShellLlmValidation = {
  enabled: boolean;
  model: string;
  authority: ShellLlmAuthority;
  timeout_secs: number;
  on_failure: ShellLlmOnFailure;
  cache_per_chat: boolean;
};

export type ShellExecutionDefaults = {
  foreground_timeout_secs: number;
  output_limit_lines: number;
};

export type ShellRiskEntryView = {
  id: string;
  exec: string;
  level: ShellRiskLevel;
  reason: string;
  enabled: boolean;
};

export type ShellPolicy = {
  mode: ShellApprovalMode;
  deny: string[];
  ask: string[];
  allow: string[];
  trust_caller_confirmation: boolean;
  llm_validation: ShellLlmValidation;
  execution: ShellExecutionDefaults;
  catalogue: ShellRiskEntryView[];
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isStringArray(value: unknown): value is string[] {
  return (
    Array.isArray(value) && value.every((item) => typeof item === "string")
  );
}

function isRiskLevel(value: unknown): value is ShellRiskLevel {
  return (
    value === "low" ||
    value === "medium" ||
    value === "high" ||
    value === "critical"
  );
}

function isGateDecision(value: unknown): value is ShellGateDecision {
  return value === "pass" || value === "confirmation" || value === "deny";
}

function isRiskEntry(value: unknown): value is ShellRiskEntryView {
  return (
    isRecord(value) &&
    typeof value.id === "string" &&
    typeof value.exec === "string" &&
    isRiskLevel(value.level) &&
    typeof value.reason === "string" &&
    typeof value.enabled === "boolean"
  );
}

function isShellCommandTestResult(
  value: unknown,
): value is ShellCommandTestResult {
  return (
    isRecord(value) &&
    isGateDecision(value.decision) &&
    typeof value.rule === "string" &&
    typeof value.reason === "string" &&
    (value.risk_level === null || isRiskLevel(value.risk_level)) &&
    isStringArray(value.segments)
  );
}

function isShellAuditEntry(value: unknown): value is ShellAuditEntry {
  return (
    isRecord(value) &&
    typeof value.ts_ms === "number" &&
    typeof value.chat_id === "string" &&
    typeof value.command === "string" &&
    isGateDecision(value.decision) &&
    typeof value.layer === "string" &&
    typeof value.rule === "string" &&
    (value.risk_level === null || isRiskLevel(value.risk_level))
  );
}

function isShellAuditResponse(
  value: unknown,
): value is { entries: ShellAuditEntry[] } {
  return (
    isRecord(value) &&
    Array.isArray(value.entries) &&
    value.entries.every(isShellAuditEntry)
  );
}

export function isShellPolicy(value: unknown): value is ShellPolicy {
  if (!isRecord(value)) return false;
  const llm = value.llm_validation;
  const execution = value.execution;

  return (
    (value.mode === "strict" ||
      value.mode === "balanced" ||
      value.mode === "permissive" ||
      value.mode === "yolo") &&
    isStringArray(value.deny) &&
    isStringArray(value.ask) &&
    isStringArray(value.allow) &&
    typeof value.trust_caller_confirmation === "boolean" &&
    isRecord(llm) &&
    typeof llm.enabled === "boolean" &&
    typeof llm.model === "string" &&
    (llm.authority === "ask_only" || llm.authority === "ask_and_allow") &&
    typeof llm.timeout_secs === "number" &&
    (llm.on_failure === "pass" || llm.on_failure === "ask") &&
    typeof llm.cache_per_chat === "boolean" &&
    isRecord(execution) &&
    typeof execution.foreground_timeout_secs === "number" &&
    typeof execution.output_limit_lines === "number" &&
    Array.isArray(value.catalogue) &&
    value.catalogue.every(isRiskEntry)
  );
}

export const shellPolicyApi = createApi({
  reducerPath: "shellPolicyApi",
  tagTypes: ["SHELL_POLICY", "SHELL_AUDIT"],
  baseQuery: fetchBaseQuery({
    prepareHeaders: (headers, api) => {
      const getState = api.getState as () => RootState;
      const token = getState().config.apiKey;
      if (token) {
        headers.set("Authorization", `Bearer ${token}`);
      }
      return headers;
    },
  }),
  endpoints: (builder) => ({
    getShellPolicy: builder.query<ShellPolicy, undefined>({
      providesTags: ["SHELL_POLICY"],
      async queryFn(_arg, api, extraOptions, baseQuery) {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(state, "/v1/shell-policy");
        const response = await baseQuery({ url, ...extraOptions });
        if (response.error) return { error: response.error };
        if (!isShellPolicy(response.data)) {
          return {
            error: {
              status: "CUSTOM_ERROR",
              data: response.data,
              error: "Invalid shell policy response",
            },
          };
        }
        return { data: response.data };
      },
    }),
    updateShellPolicy: builder.mutation<ShellPolicy, ShellPolicy>({
      invalidatesTags: ["SHELL_POLICY"],
      async queryFn(policy, api, extraOptions, baseQuery) {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(state, "/v1/shell-policy");
        const response = await baseQuery({
          url,
          method: "POST",
          body: policy,
          ...extraOptions,
        });
        if (response.error) return { error: response.error };
        if (!isShellPolicy(response.data)) {
          return {
            error: {
              status: "CUSTOM_ERROR",
              data: response.data,
              error: "Invalid shell policy response",
            },
          };
        }
        return { data: response.data };
      },
    }),
    testShellCommand: builder.mutation<
      ShellCommandTestResult,
      { command: string }
    >({
      async queryFn(body, api, extraOptions, baseQuery) {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(state, "/v1/shell-policy/test");
        const response = await baseQuery({
          url,
          method: "POST",
          body,
          ...extraOptions,
        });
        if (response.error) return { error: response.error };
        if (!isShellCommandTestResult(response.data)) {
          return {
            error: {
              status: "CUSTOM_ERROR",
              data: response.data,
              error: "Invalid shell command test response",
            },
          };
        }
        return { data: response.data };
      },
    }),
    getShellAudit: builder.query<
      { entries: ShellAuditEntry[] },
      { limit: number }
    >({
      providesTags: ["SHELL_AUDIT"],
      async queryFn({ limit }, api, extraOptions, baseQuery) {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(
          state,
          `/v1/shell-policy/audit?limit=${String(limit)}`,
        );
        const response = await baseQuery({ url, ...extraOptions });
        if (response.error) return { error: response.error };
        if (!isShellAuditResponse(response.data)) {
          return {
            error: {
              status: "CUSTOM_ERROR",
              data: response.data,
              error: "Invalid shell audit response",
            },
          };
        }
        return { data: response.data };
      },
    }),
  }),
});

export const {
  useGetShellAuditQuery,
  useGetShellPolicyQuery,
  useTestShellCommandMutation,
  useUpdateShellPolicyMutation,
} = shellPolicyApi;
