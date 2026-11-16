import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import type { RootState } from "../../app/store";
import { buildApiUrlFromState } from "./apiUrl";
import { normalizePythonDisplay, type PythonDisplay } from "./types";

export type PythonWorkspaceDisplay = PythonDisplay & {
  run_id: string;
  started_at_ms: number;
};

export type PythonWorkspace = {
  schema: 1;
  chat_id: string;
  exists: boolean;
  workspace_dir: string;
  artifacts_dir: string;
  session: {
    status: "none" | "idle" | "running" | "background";
    live: boolean;
    state_persisted: boolean;
    state_bytes: number;
    saved_at_ms: number | null;
    last_run_id: string | null;
    reset_reason: string | null;
    variables: { name: string; type_name: string; preview: string }[];
    variables_truncated: boolean;
    /** Optional: total variables in the session when the list was capped. */
    variables_total?: number;
    /** Optional: the last run restored this session from disk. */
    last_run_restored?: boolean;
  };
  artifacts: {
    path: string;
    rel_path: string;
    bytes: number;
    modified_at_ms: number;
  }[];
  artifacts_truncated: boolean;
  runs: {
    run_id: string;
    status: string;
    started_at_ms: number;
    finished_at_ms: number | null;
    calls_total: number;
    background: boolean;
    /** Optional one-sentence description of what the run's script does. */
    description?: string;
    /** Optional first non-empty line of the run's code. */
    code_preview?: string;
    /** Optional count of the run's model calls. */
    llm_calls?: number;
    /** Optional count of the displays the run showed. */
    displays_total?: number;
    /** Optional count of the engine changes the run applied. */
    engine_changes?: number;
  }[];
  runs_truncated: boolean;
  /** Optional newest displays of recent runs, newest first (at most 8). */
  displays?: PythonWorkspaceDisplay[];
  displays_truncated?: boolean;
  disk: { workspace_bytes: number; quota_bytes: number };
};
const record = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);
const count = (value: unknown): value is number =>
  typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
const nullableString = (value: unknown) =>
  value === null || typeof value === "string";
/** Copy of a record without the listed optional keys whose value fails its check. */
function keepValid(
  value: Record<string, unknown>,
  checks: Record<string, (item: unknown) => boolean>,
): Record<string, unknown> {
  return Object.fromEntries(
    Object.entries(value).filter(
      ([key, item]) => !(key in checks) || checks[key](item),
    ),
  );
}
const isString = (value: unknown) => typeof value === "string";
const isBoolean = (value: unknown) => typeof value === "boolean";
const WORKSPACE_DISPLAYS_MAX = 8;

function workspaceDisplays(value: unknown): PythonWorkspaceDisplay[] {
  if (!Array.isArray(value)) return [];
  return value.flatMap((item: unknown) => {
    if (
      !record(item) ||
      typeof item.run_id !== "string" ||
      !count(item.started_at_ms)
    )
      return [];
    const display = normalizePythonDisplay(item);
    return display
      ? [{ ...display, run_id: item.run_id, started_at_ms: item.started_at_ms }]
      : [];
  });
}

/**
 * Drops optional fields from a newer or older engine that have an unexpected
 * value, so one bad optional field never rejects the whole workspace; display
 * entries that fail their own check are left out one by one.
 */
export function dropInvalidOptionals(value: unknown): unknown {
  if (!record(value) || !record(value.session)) return value;
  const result: Record<string, unknown> = {
    ...keepValid(value, { displays_truncated: isBoolean }),
    session: keepValid(value.session, {
      variables_total: count,
      last_run_restored: isBoolean,
    }),
    runs: Array.isArray(value.runs)
      ? value.runs.map((run: unknown) =>
          record(run)
            ? keepValid(run, {
                description: isString,
                code_preview: isString,
                llm_calls: count,
                displays_total: count,
                engine_changes: count,
              })
            : run,
        )
      : value.runs,
  };
  if (value.displays === undefined) delete result.displays;
  else result.displays = workspaceDisplays(value.displays);
  return result;
}
const list = (
  value: unknown,
  guard: (item: Record<string, unknown>) => boolean,
) =>
  Array.isArray(value) &&
  value.every((item: unknown) => record(item) && guard(item));
export function isPythonWorkspace(value: unknown): value is PythonWorkspace {
  if (!record(value) || !record(value.session) || !record(value.disk))
    return false;
  const session = value.session;
  return (
    value.schema === 1 &&
    typeof value.chat_id === "string" &&
    typeof value.exists === "boolean" &&
    typeof value.workspace_dir === "string" &&
    typeof value.artifacts_dir === "string" &&
    typeof session.status === "string" &&
    ["none", "idle", "running", "background"].includes(session.status) &&
    typeof session.live === "boolean" &&
    typeof session.state_persisted === "boolean" &&
    count(session.state_bytes) &&
    (session.saved_at_ms === null || count(session.saved_at_ms)) &&
    nullableString(session.last_run_id) &&
    nullableString(session.reset_reason) &&
    typeof session.variables_truncated === "boolean" &&
    (session.variables_total === undefined || count(session.variables_total)) &&
    (session.last_run_restored === undefined ||
      isBoolean(session.last_run_restored)) &&
    list(
      session.variables,
      (v) =>
        typeof v.name === "string" &&
        typeof v.type_name === "string" &&
        typeof v.preview === "string",
    ) &&
    typeof value.artifacts_truncated === "boolean" &&
    list(
      value.artifacts,
      (v) =>
        typeof v.path === "string" &&
        typeof v.rel_path === "string" &&
        count(v.bytes) &&
        count(v.modified_at_ms),
    ) &&
    typeof value.runs_truncated === "boolean" &&
    list(
      value.runs,
      (v) =>
        typeof v.run_id === "string" &&
        typeof v.status === "string" &&
        count(v.started_at_ms) &&
        (v.finished_at_ms === null || count(v.finished_at_ms)) &&
        count(v.calls_total) &&
        typeof v.background === "boolean" &&
        (v.description === undefined || isString(v.description)) &&
        (v.code_preview === undefined || isString(v.code_preview)) &&
        (v.llm_calls === undefined || count(v.llm_calls)) &&
        (v.displays_total === undefined || count(v.displays_total)) &&
        (v.engine_changes === undefined || count(v.engine_changes)),
    ) &&
    (value.displays === undefined ||
      (Array.isArray(value.displays) &&
        workspaceDisplays(value.displays).length === value.displays.length)) &&
    (value.displays_truncated === undefined ||
      isBoolean(value.displays_truncated)) &&
    count(value.disk.workspace_bytes) &&
    count(value.disk.quota_bytes)
  );
}
export const pythonWorkspaceApi = createApi({
  reducerPath: "pythonWorkspaceApi",
  baseQuery: fetchBaseQuery({
    prepareHeaders: (headers, { getState }) => {
      const token = (getState() as RootState).config.apiKey;
      if (token) headers.set("Authorization", `Bearer ${token}`);
      return headers;
    },
  }),
  tagTypes: ["PythonWorkspace"],
  endpoints: (builder) => ({
    getPythonWorkspace: builder.query<PythonWorkspace, { chatId: string }>({
      queryFn: async ({ chatId }, api, _opts, baseQuery) => {
        const result = await baseQuery({
          url: buildApiUrlFromState(
            api.getState() as RootState,
            "/v1/python/workspace",
            { chat_id: chatId },
          ),
        });
        if (result.error) return { error: result.error };
        const candidate = dropInvalidOptionals(result.data);
        if (!isPythonWorkspace(candidate) || candidate.chat_id !== chatId)
          return {
            error: {
              status: "CUSTOM_ERROR",
              error: "Invalid Python workspace response",
            },
          };
        const data = candidate;
        return {
          data: {
            ...data,
            session: {
              ...data.session,
              variables: data.session.variables.slice(0, 500),
              variables_truncated:
                data.session.variables_truncated ||
                data.session.variables.length > 500,
            },
            artifacts: data.artifacts.slice(0, 500),
            artifacts_truncated:
              data.artifacts_truncated || data.artifacts.length > 500,
            runs: data.runs.slice(0, 500),
            runs_truncated: data.runs_truncated || data.runs.length > 500,
            ...(data.displays
              ? {
                  displays: [...data.displays]
                    .sort((a, b) => b.started_at_ms - a.started_at_ms)
                    .slice(0, WORKSPACE_DISPLAYS_MAX),
                  displays_truncated:
                    (data.displays_truncated ?? false) ||
                    data.displays.length > WORKSPACE_DISPLAYS_MAX,
                }
              : {}),
          },
        };
      },
      providesTags: (_result, _error, { chatId }) => [
        { type: "PythonWorkspace", id: chatId },
      ],
    }),
  }),
});
export const { useGetPythonWorkspaceQuery } = pythonWorkspaceApi;
