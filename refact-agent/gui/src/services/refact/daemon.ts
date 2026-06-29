import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import type { FetchBaseQueryError } from "@reduxjs/toolkit/query";

import type { RootState } from "../../app/store";
import type { EngineApiConfig } from "./apiUrl";

const DEFAULT_DAEMON_PORT = 8488;

export type DaemonStatus = {
  pid: number;
  version: string;
  port: number;
  started_at_ms: number;
  uptime_secs: number;
  workers: number;
  cron_pending: number;
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

export type DaemonInfo = {
  status: DaemonStatus;
  workers: DaemonWorker[];
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

function isWorkersOptionalError(error: FetchBaseQueryError): boolean {
  if (error.status === 401) return true;
  if (error.status !== "PARSING_ERROR") return false;
  return error.originalStatus === 200 || error.originalStatus === 204;
}

export const daemonApi = createApi({
  reducerPath: "daemon",
  tagTypes: ["Daemon"],
  baseQuery: fetchBaseQuery({
    prepareHeaders: (headers, { getState }) => {
      const state = getState() as RootState;
      const apiKey = state.config.apiKey;
      if (apiKey) {
        headers.set("Authorization", `Bearer ${apiKey}`);
      }
      return headers;
    },
  }),
  endpoints: (builder) => ({
    getDaemonInfo: builder.query<DaemonInfo, undefined>({
      keepUnusedDataFor: 10,
      providesTags: ["Daemon"],
      queryFn: async (_args, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
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

        if (workersResult.error) {
          if (isWorkersOptionalError(workersResult.error)) {
            return {
              data: {
                status: statusResult.data as DaemonStatus,
                workers: [],
              },
            };
          }

          return { error: workersResult.error };
        }

        return {
          data: {
            status: statusResult.data as DaemonStatus,
            workers: Array.isArray(workersResult.data)
              ? (workersResult.data as DaemonWorker[])
              : [],
          },
        };
      },
    }),
  }),
});

export const { useGetDaemonInfoQuery } = daemonApi;
