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

function validPort(port: number | undefined): port is number {
  return port !== undefined && Number.isFinite(port) && port > 0;
}

export function resolveDaemonBaseUrl(config: EngineApiConfig): string {
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

function isWorkersEmptyBody(error: FetchBaseQueryError): boolean {
  if (error.status !== "PARSING_ERROR") return false;
  return error.originalStatus === 200 || error.originalStatus === 204;
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
  useCheckDaemonUpdateQuery,
  useGetDaemonInfoQuery,
  useGetDaemonSettingsQuery,
  useGetDaemonUpdateStatusQuery,
  useInstallDaemonUpdateMutation,
  useLazyCheckDaemonUpdateQuery,
  useRestartDaemonMutation,
  useShutdownDaemonMutation,
  useUpdateDaemonSettingsMutation,
} = daemonApi;
