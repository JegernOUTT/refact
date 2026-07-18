import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";

import {
  buildApiUrlFromState,
  hasUsableEngineEndpoint,
  type EngineApiConfig,
} from "./apiUrl";

type BugReportApiState = {
  config: EngineApiConfig & { apiKey?: string | null };
};

export const BUG_REPORT_CONTEXT_URL = "/v1/bug-report/context";
export const BUG_REPORT_LOGS_URL = "/v1/bug-report/logs";
export const BUG_REPORT_ERRORS_URL = "/v1/bug-report/errors";
export const BUG_REPORT_BUNDLE_URL = "/v1/bug-report/bundle";

export type BugReportLogSource = "engine" | "daemon";

export type BugReportLogPaths = {
  engine_log_target: string;
  engine_log_exists: boolean;
  daemon_log_file: string;
  daemon_log_exists: boolean;
  daemon_logs_dir: string;
};

export type BugReportContext = {
  engine_version: string;
  os: string;
  http_port: number;
  cache_dir: string;
  config_dir: string;
  workspace_roots: string[];
  log_paths: BugReportLogPaths;
  bundle_default_dir: string;
};

export type BugReportLogsResponse = {
  source: string;
  path: string;
  exists: boolean;
  lines: string[];
  read_error?: string;
};

export type BugReportErrorLevel = "error" | "warn";

export type BugReportErrorEntry = {
  source: string;
  level: string;
  message: string;
  count?: number;
  location?: string;
};

export type BugReportErrorsResponse = {
  errors: BugReportErrorEntry[];
};

export type BugReportBundleRequest = {
  dest_dir?: string;
  redact?: boolean;
  webui_lines?: string[];
  ide_lines?: string[];
};

export type BugReportBundleFile = {
  name: string;
  size_bytes: number;
};

export type BugReportBundleResponse = {
  path: string;
  size_bytes: number;
  files: BugReportBundleFile[];
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

export function isBugReportContext(value: unknown): value is BugReportContext {
  if (!isRecord(value)) return false;
  return (
    typeof value.engine_version === "string" &&
    typeof value.os === "string" &&
    typeof value.bundle_default_dir === "string" &&
    Array.isArray(value.workspace_roots) &&
    isRecord(value.log_paths)
  );
}

export function isBugReportLogsResponse(
  value: unknown,
): value is BugReportLogsResponse {
  if (!isRecord(value)) return false;
  return (
    typeof value.source === "string" &&
    typeof value.path === "string" &&
    typeof value.exists === "boolean" &&
    Array.isArray(value.lines) &&
    value.lines.every((line) => typeof line === "string") &&
    (value.read_error === undefined || typeof value.read_error === "string")
  );
}

export function isBugReportErrorsResponse(
  value: unknown,
): value is BugReportErrorsResponse {
  if (!isRecord(value)) return false;
  return (
    Array.isArray(value.errors) &&
    value.errors.every(
      (entry) =>
        isRecord(entry) &&
        typeof entry.source === "string" &&
        typeof entry.level === "string" &&
        typeof entry.message === "string" &&
        (entry.count === undefined || typeof entry.count === "number") &&
        (entry.location === undefined || typeof entry.location === "string"),
    )
  );
}

export function isBugReportBundleResponse(
  value: unknown,
): value is BugReportBundleResponse {
  if (!isRecord(value)) return false;
  return (
    typeof value.path === "string" &&
    typeof value.size_bytes === "number" &&
    Array.isArray(value.files) &&
    value.files.every(
      (file) =>
        isRecord(file) &&
        typeof file.name === "string" &&
        typeof file.size_bytes === "number",
    )
  );
}

export const bugReportApi = createApi({
  reducerPath: "bugReportApi",
  baseQuery: fetchBaseQuery({
    prepareHeaders: (headers, { getState }) => {
      const token = (getState() as BugReportApiState).config.apiKey;
      if (token) {
        headers.set("Authorization", `Bearer ${token}`);
      }
      return headers;
    },
  }),
  endpoints: (builder) => ({
    getBugReportContext: builder.query<BugReportContext, undefined>({
      queryFn: async (_arg, api, _extraOptions, baseQuery) => {
        const state = api.getState() as BugReportApiState;
        if (!hasUsableEngineEndpoint(state.config)) {
          return {
            error: { status: 500, data: "Missing engine endpoint in config" },
          };
        }
        const url = buildApiUrlFromState(state, BUG_REPORT_CONTEXT_URL);
        const response = await baseQuery({ url });
        if (response.error) {
          return { error: response.error };
        }
        if (!isBugReportContext(response.data)) {
          return {
            error: {
              status: "CUSTOM_ERROR",
              error: `Invalid response from ${url}`,
              data: response.data,
            },
          };
        }
        return { data: response.data };
      },
    }),
    getBugReportLogs: builder.query<
      BugReportLogsResponse,
      { source: BugReportLogSource; tail?: number }
    >({
      queryFn: async (arg, api, _extraOptions, baseQuery) => {
        const state = api.getState() as BugReportApiState;
        if (!hasUsableEngineEndpoint(state.config)) {
          return {
            error: { status: 500, data: "Missing engine endpoint in config" },
          };
        }
        const params = new URLSearchParams({ source: arg.source });
        if (arg.tail !== undefined) {
          params.set("tail", String(arg.tail));
        }
        const url = `${buildApiUrlFromState(
          state,
          BUG_REPORT_LOGS_URL,
        )}?${params.toString()}`;
        const response = await baseQuery({ url });
        if (response.error) {
          return { error: response.error };
        }
        if (!isBugReportLogsResponse(response.data)) {
          return {
            error: {
              status: "CUSTOM_ERROR",
              error: `Invalid response from ${url}`,
              data: response.data,
            },
          };
        }
        return { data: response.data };
      },
    }),
    getBugReportErrors: builder.query<BugReportErrorsResponse, undefined>({
      queryFn: async (_arg, api, _extraOptions, baseQuery) => {
        const state = api.getState() as BugReportApiState;
        if (!hasUsableEngineEndpoint(state.config)) {
          return {
            error: { status: 500, data: "Missing engine endpoint in config" },
          };
        }
        const url = buildApiUrlFromState(state, BUG_REPORT_ERRORS_URL);
        const response = await baseQuery({ url });
        if (response.error) {
          return { error: response.error };
        }
        if (!isBugReportErrorsResponse(response.data)) {
          return {
            error: {
              status: "CUSTOM_ERROR",
              error: `Invalid response from ${url}`,
              data: response.data,
            },
          };
        }
        return { data: response.data };
      },
    }),
    createBugReportBundle: builder.mutation<
      BugReportBundleResponse,
      BugReportBundleRequest
    >({
      queryFn: async (arg, api, _extraOptions, baseQuery) => {
        const state = api.getState() as BugReportApiState;
        if (!hasUsableEngineEndpoint(state.config)) {
          return {
            error: { status: 500, data: "Missing engine endpoint in config" },
          };
        }
        const url = buildApiUrlFromState(state, BUG_REPORT_BUNDLE_URL);
        const response = await baseQuery({
          url,
          method: "POST",
          body: arg,
        });
        if (response.error) {
          return { error: response.error };
        }
        if (!isBugReportBundleResponse(response.data)) {
          return {
            error: {
              status: "CUSTOM_ERROR",
              error: `Invalid response from ${url}`,
              data: response.data,
            },
          };
        }
        return { data: response.data };
      },
    }),
  }),
});

export const {
  useGetBugReportContextQuery,
  useGetBugReportLogsQuery,
  useGetBugReportErrorsQuery,
  useCreateBugReportBundleMutation,
} = bugReportApi;
