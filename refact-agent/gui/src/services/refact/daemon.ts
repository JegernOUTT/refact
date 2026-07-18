import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import type { FetchBaseQueryError } from "@reduxjs/toolkit/query";

import type { EngineApiConfig } from "./apiUrl";

const DEFAULT_DAEMON_PORT = 8488;

export type DaemonStatus = {
  pid: number;
  version: string;
  executable_sha256?: string;
  port: number;
  started_at_ms: number;
  uptime_secs: number;
  workers: number;
  cron_pending: Record<string, number>;
};

export type DaemonWorker = {
  project_id: string;
  slug: string;
  root: string;
  pinned: boolean;
  last_active_ms: number | null;
  state: string;
  pid: number | null;
  rss_bytes?: number | null;
  cpu_percent?: number | null;
  uptime_secs?: number | null;
  http_port: number | null;
  lsp_port: number | null;
  lsp_clients: number;
  busy_chats: number;
  exec_running: number;
  live_proxy_streams: number;
  cron_next_fire_ms: number | null;
  idle_deadline_ms: number | null;
  last_status_report_ms: number | null;
  last_error: string | null;
};

export type DaemonWorkersAccess = "visible" | "auth_hidden";

export type DaemonSettingsAccess = "visible" | "auth_hidden";

export type DaemonSettings = {
  bind: string;
  lan_enabled: boolean;
  mdns_enabled: boolean;
  auth_enabled: boolean;
  username: string | null;
  has_password: boolean;
  hostname_local: string;
  urls: DaemonUrls;
};

export type DaemonUrls = {
  loopback: string;
  mdns: string;
};

export type DaemonSettingsResponse =
  | { settings: DaemonSettings; access: "visible" }
  | { settings: null; access: "auth_hidden" };

export type DaemonSettingsUpdate = {
  lan_enabled: boolean;
  mdns_enabled: boolean;
  auth_enabled: boolean;
  username?: string;
  password?: string;
};

export type DaemonRelease = {
  version: string;
  published_at: string | null;
  prerelease: boolean;
  url: string | null;
};

export type DaemonUpdateCheck = {
  current_version: string;
  latest_version: string | null;
  update_available: boolean;
  releases: DaemonRelease[];
  checked_at_ms: number;
};

export type DaemonUpdatePhase =
  | "idle"
  | "checking"
  | "downloading"
  | "restarting"
  | "failed";

export type DaemonUpdateStatus = {
  phase: DaemonUpdatePhase;
  detail: string | null;
  target_version: string | null;
  started_at_ms: number | null;
  finished_at_ms: number | null;
};

export type DaemonInfo = {
  status: DaemonStatus;
  workers: DaemonWorker[];
  workersAccess: DaemonWorkersAccess;
};

export type DaemonEvent = {
  seq: number;
  ts_ms: number;
  kind: string;
  project_id: string | null;
  payload: unknown;
};

export type DaemonProjectOpenRequest = {
  root: string;
  client_kind?: string;
};

export type DaemonProjectWorker = {
  project_id: string;
  pid: number | null;
  http_port: number;
  lsp_port: number;
  state: string | { failed: { reason: string } };
  last_error?: string;
};

export type DaemonProjectOpenResponse = {
  project_id: string;
  slug: string;
  root: string;
  pinned: boolean;
  worker: DaemonProjectWorker | null;
  cron_pending: number | null;
};

export type DaemonCronStatus = {
  enabled: boolean;
  jobs: number;
  next_wake_ms: number | null;
};

export type DaemonFolderEntry = {
  name: string;
  has_git: boolean;
};

export type DaemonFolderBrowseResponse = {
  path: string;
  parent: string | null;
  dirs: DaemonFolderEntry[];
  can_open: boolean;
  truncated: boolean;
};

function validPort(port: number | undefined): port is number {
  return port !== undefined && Number.isFinite(port) && port > 0;
}

export function resolveDaemonBaseUrl(config: EngineApiConfig): string {
  if (
    config.host === "web" &&
    config.engineServed === true &&
    typeof window !== "undefined"
  ) {
    return window.location.origin;
  }

  const rawLspUrl = config.lspUrl?.trim();
  if (rawLspUrl) {
    try {
      const base =
        typeof window !== "undefined" ? window.location.origin : undefined;
      const url = base ? new URL(rawLspUrl, base) : new URL(rawLspUrl);
      if (url.protocol === "http:" || url.protocol === "https:") {
        return url.origin;
      }
    } catch {
      return `http://127.0.0.1:${
        validPort(config.lspPort) ? config.lspPort : DEFAULT_DAEMON_PORT
      }`;
    }
  }

  const port = validPort(config.lspPort) ? config.lspPort : DEFAULT_DAEMON_PORT;
  return `http://127.0.0.1:${port}`;
}

export function resolveDaemonLogsUrl(
  config: EngineApiConfig,
  projectId: string | null,
  stream: boolean,
  tail: number,
): string {
  const params = new URLSearchParams({ tail: String(tail) });
  if (projectId) params.set("project_id", projectId);
  const path = stream ? "/daemon/v1/logs/stream" : "/daemon/v1/logs";
  return `${resolveDaemonBaseUrl(config)}${path}?${params.toString()}`;
}

export function projectApiUrl(
  daemonBase: string,
  projectId: string,
  path: string,
): string {
  const base = daemonBase.replace(/\/+$/, "");
  const suffix = path.startsWith("/") ? path : `/${path}`;
  return `${base}/p/${encodeURIComponent(projectId)}/v1${suffix}`;
}

function isWorkersEmptyBody(error: FetchBaseQueryError): boolean {
  if (error.status !== "PARSING_ERROR") return false;
  return error.originalStatus === 200 || error.originalStatus === 204;
}

function parseDaemonEvents(body: string): DaemonEvent[] {
  const events: DaemonEvent[] = [];
  for (const block of body.split(/\r?\n\r?\n/)) {
    const data = block
      .split(/\r?\n/)
      .filter((line) => line.startsWith("data:"))
      .map((line) => line.slice(5).trimStart())
      .join("\n");
    if (!data) continue;
    try {
      const event = JSON.parse(data) as DaemonEvent;
      if (
        typeof event.seq === "number" &&
        typeof event.ts_ms === "number" &&
        typeof event.kind === "string"
      ) {
        events.push(event);
      }
    } catch {
      continue;
    }
  }
  return events;
}

export const daemonApi = createApi({
  reducerPath: "daemon",
  tagTypes: ["Daemon"],
  baseQuery: fetchBaseQuery(),
  endpoints: (builder) => ({
    getDaemonInfo: builder.query<DaemonInfo, undefined>({
      keepUnusedDataFor: 10,
      providesTags: ["Daemon"],
      queryFn: async (_args, api, _opts, baseQuery) => {
        const state = api.getState() as { config: EngineApiConfig };
        const root = resolveDaemonBaseUrl(state.config);
        const statusResult = await baseQuery({
          url: `${root}/daemon/v1/status`,
        });

        if (statusResult.error) {
          return { error: statusResult.error };
        }

        const workersResult = await baseQuery({
          url: `${root}/daemon/v1/workers`,
        });

        const status = statusResult.data as DaemonStatus;

        if (workersResult.error) {
          if (workersResult.error.status === 401) {
            return {
              data: {
                status,
                workers: [],
                workersAccess: "auth_hidden",
              },
            };
          }

          if (isWorkersEmptyBody(workersResult.error)) {
            return {
              data: {
                status,
                workers: [],
                workersAccess: "visible",
              },
            };
          }

          return { error: workersResult.error };
        }

        return {
          data: {
            status,
            workers: Array.isArray(workersResult.data)
              ? (workersResult.data as DaemonWorker[])
              : [],
            workersAccess: "visible",
          },
        };
      },
    }),
    listProjects: builder.query<DaemonWorker[], undefined>({
      providesTags: ["Daemon"],
      queryFn: async (_args, api, _opts, baseQuery) => {
        const state = api.getState() as { config: EngineApiConfig };
        const result = await baseQuery({
          url: `${resolveDaemonBaseUrl(state.config)}/daemon/v1/workers`,
        });
        if (result.error) return { error: result.error };
        return {
          data: Array.isArray(result.data)
            ? (result.data as DaemonWorker[])
            : [],
        };
      },
    }),
    openProject: builder.mutation<
      DaemonProjectOpenResponse,
      DaemonProjectOpenRequest
    >({
      invalidatesTags: ["Daemon"],
      queryFn: async (body, api, _opts, baseQuery) => {
        const state = api.getState() as { config: EngineApiConfig };
        const result = await baseQuery({
          url: `${resolveDaemonBaseUrl(state.config)}/daemon/v1/projects/open`,
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as DaemonProjectOpenResponse };
      },
    }),
    forgetProject: builder.mutation<unknown, string>({
      invalidatesTags: ["Daemon"],
      queryFn: async (projectId, api, _opts, baseQuery) => {
        const state = api.getState() as { config: EngineApiConfig };
        const result = await baseQuery({
          url: `${resolveDaemonBaseUrl(
            state.config,
          )}/daemon/v1/projects/${encodeURIComponent(projectId)}`,
          method: "DELETE",
        });
        if (result.error) return { error: result.error };
        return { data: result.data };
      },
    }),
    pinProject: builder.mutation<
      unknown,
      { projectId: string; pinned: boolean }
    >({
      invalidatesTags: ["Daemon"],
      queryFn: async ({ projectId, pinned }, api, _opts, baseQuery) => {
        const state = api.getState() as { config: EngineApiConfig };
        const result = await baseQuery({
          url: `${resolveDaemonBaseUrl(
            state.config,
          )}/daemon/v1/projects/${encodeURIComponent(projectId)}/pin`,
          method: "POST",
          body: { pinned },
        });
        if (result.error) return { error: result.error };
        return { data: result.data };
      },
    }),
    restartProject: builder.mutation<DaemonProjectWorker, string>({
      invalidatesTags: ["Daemon"],
      queryFn: async (projectId, api, _opts, baseQuery) => {
        const state = api.getState() as { config: EngineApiConfig };
        const result = await baseQuery({
          url: `${resolveDaemonBaseUrl(
            state.config,
          )}/daemon/v1/projects/${encodeURIComponent(projectId)}/restart`,
          method: "POST",
        });
        if (result.error) return { error: result.error };
        return { data: result.data as DaemonProjectWorker };
      },
    }),
    stopProject: builder.mutation<unknown, string>({
      invalidatesTags: ["Daemon"],
      queryFn: async (projectId, api, _opts, baseQuery) => {
        const state = api.getState() as { config: EngineApiConfig };
        const result = await baseQuery({
          url: `${resolveDaemonBaseUrl(
            state.config,
          )}/daemon/v1/projects/${encodeURIComponent(projectId)}/stop`,
          method: "POST",
        });
        if (result.error) return { error: result.error };
        return { data: result.data };
      },
    }),
    getDaemonEvents: builder.query<DaemonEvent[], number | undefined>({
      queryFn: async (afterSequence, api, _opts, baseQuery) => {
        const state = api.getState() as { config: EngineApiConfig };
        const params = new URLSearchParams({
          after_seq: String(afterSequence ?? 0),
          follow: "false",
        });
        const result = await baseQuery({
          url: `${resolveDaemonBaseUrl(
            state.config,
          )}/daemon/v1/events?${params.toString()}`,
          responseHandler: "text",
        });
        if (result.error) return { error: result.error };
        return { data: parseDaemonEvents(String(result.data ?? "")) };
      },
    }),
    getCronStatus: builder.query<DaemonCronStatus, undefined>({
      queryFn: async (_args, api, _opts, baseQuery) => {
        const state = api.getState() as { config: EngineApiConfig };
        const result = await baseQuery({
          url: `${resolveDaemonBaseUrl(state.config)}/cron/status`,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as DaemonCronStatus };
      },
    }),
    browseFolders: builder.mutation<
      DaemonFolderBrowseResponse,
      { path?: string }
    >({
      queryFn: async (body, api, _opts, baseQuery) => {
        const state = api.getState() as { config: EngineApiConfig };
        const result = await baseQuery({
          url: `${resolveDaemonBaseUrl(state.config)}/daemon/v1/fs/browse`,
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as DaemonFolderBrowseResponse };
      },
    }),
    getDaemonSettings: builder.query<DaemonSettingsResponse, undefined>({
      providesTags: ["Daemon"],
      queryFn: async (_args, api, _opts, baseQuery) => {
        const state = api.getState() as { config: EngineApiConfig };
        const root = resolveDaemonBaseUrl(state.config);
        const result = await baseQuery({
          url: `${root}/daemon/v1/settings`,
        });

        if (result.error) {
          if (result.error.status === 401) {
            return { data: { settings: null, access: "auth_hidden" } };
          }

          return { error: result.error };
        }

        return {
          data: {
            settings: result.data as DaemonSettings,
            access: "visible",
          },
        };
      },
    }),
    updateDaemonSettings: builder.mutation<
      { success: boolean; restarting: boolean },
      DaemonSettingsUpdate
    >({
      invalidatesTags: ["Daemon"],
      queryFn: async (body, api, _opts, baseQuery) => {
        const state = api.getState() as { config: EngineApiConfig };
        const root = resolveDaemonBaseUrl(state.config);
        const result = await baseQuery({
          url: `${root}/daemon/v1/settings`,
          method: "POST",
          body,
        });

        if (result.error) return { error: result.error };

        return {
          data: result.data as { success: boolean; restarting: boolean },
        };
      },
    }),
    restartDaemon: builder.mutation<
      { success: boolean; restarting: boolean },
      undefined
    >({
      invalidatesTags: ["Daemon"],
      queryFn: async (_args, api, _opts, baseQuery) => {
        const state = api.getState() as { config: EngineApiConfig };
        const root = resolveDaemonBaseUrl(state.config);
        const result = await baseQuery({
          url: `${root}/daemon/v1/restart`,
          method: "POST",
        });

        if (result.error) return { error: result.error };

        return {
          data: result.data as { success: boolean; restarting: boolean },
        };
      },
    }),
    shutdownDaemon: builder.mutation<unknown, { reason: string }>({
      invalidatesTags: ["Daemon"],
      queryFn: async (body, api, _opts, baseQuery) => {
        const state = api.getState() as { config: EngineApiConfig };
        const root = resolveDaemonBaseUrl(state.config);
        const result = await baseQuery({
          url: `${root}/daemon/v1/shutdown`,
          method: "POST",
          body,
        });

        if (result.error) return { error: result.error };

        return { data: result.data };
      },
    }),
    checkDaemonUpdate: builder.query<
      DaemonUpdateCheck,
      { refresh?: boolean } | undefined
    >({
      queryFn: async (args, api, _opts, baseQuery) => {
        const state = api.getState() as { config: EngineApiConfig };
        const root = resolveDaemonBaseUrl(state.config);
        const refresh = args?.refresh === true ? "true" : "false";
        const result = await baseQuery({
          url: `${root}/daemon/v1/update/check?refresh=${refresh}`,
        });

        if (result.error) return { error: result.error };

        return { data: result.data as DaemonUpdateCheck };
      },
    }),
    installDaemonUpdate: builder.mutation<
      { started: boolean; target_version: string | null },
      { version?: string }
    >({
      queryFn: async (body, api, _opts, baseQuery) => {
        const state = api.getState() as { config: EngineApiConfig };
        const root = resolveDaemonBaseUrl(state.config);
        const result = await baseQuery({
          url: `${root}/daemon/v1/update/install`,
          method: "POST",
          body,
        });

        if (result.error) return { error: result.error };

        return {
          data: result.data as {
            started: boolean;
            target_version: string | null;
          },
        };
      },
    }),
    getDaemonUpdateStatus: builder.query<DaemonUpdateStatus, undefined>({
      queryFn: async (_args, api, _opts, baseQuery) => {
        const state = api.getState() as { config: EngineApiConfig };
        const root = resolveDaemonBaseUrl(state.config);
        const result = await baseQuery({
          url: `${root}/daemon/v1/update/status`,
        });

        if (result.error) return { error: result.error };

        return { data: result.data as DaemonUpdateStatus };
      },
    }),
  }),
});

export const {
  useBrowseFoldersMutation,
  useCheckDaemonUpdateQuery,
  useForgetProjectMutation,
  useGetCronStatusQuery,
  useGetDaemonEventsQuery,
  useGetDaemonInfoQuery,
  useGetDaemonSettingsQuery,
  useGetDaemonUpdateStatusQuery,
  useInstallDaemonUpdateMutation,
  useLazyCheckDaemonUpdateQuery,
  useLazyGetDaemonEventsQuery,
  useListProjectsQuery,
  useOpenProjectMutation,
  usePinProjectMutation,
  useRestartDaemonMutation,
  useRestartProjectMutation,
  useShutdownDaemonMutation,
  useStopProjectMutation,
  useUpdateDaemonSettingsMutation,
} = daemonApi;
