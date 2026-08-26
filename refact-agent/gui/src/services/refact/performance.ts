import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";

import type { RootState } from "../../app/store";
import { buildApiUrlFromState } from "./apiUrl";

export type PerformanceAggregate = {
  component?: string | null;
  sample_count?: number;
  success_count?: number;
  failure_count?: number;
  skipped_count?: number;
  min_us?: number | null;
  max_us?: number | null;
  p50_us?: number | null;
  p95_us?: number | null;
  p99_us?: number | null;
  last_sample_at_ms?: number | null;
  size_bytes_sum?: number;
  item_count_sum?: number;
  batch_size_sum?: number;
};

export type PerformanceIndexWatcherVecdbRollup = {
  trajectory_index_operations_per_commit?: number;
  watcher_rebuilds_per_commit?: number;
  vecdb_searches_per_enrichment_attempt?: number;
  aggregate?: PerformanceAggregate;
};

export type PerformanceTelemetryResponse = {
  schema_version?: number;
  enabled?: boolean;
  collection_started_at_ms?: number;
  uptime_ms?: number;
  components?: PerformanceAggregate[];
  rollups?: {
    advancement?: PerformanceAggregate;
    tool_stages?: PerformanceAggregate;
    index_watcher_vecdb_amplification?: PerformanceIndexWatcherVecdbRollup;
    enrichment_stages?: PerformanceAggregate;
  };
  rollout_switches?: Record<string, boolean>;
};

export type PerformanceTelemetrySettingsResponse = {
  schema_version?: number;
  enabled?: boolean;
};

export type PerformanceTelemetryResetResponse = {
  schema_version?: number;
  reset?: boolean;
  enabled?: boolean;
};

export type TrajectorySettingValue = boolean | number | string | null;

export type TrajectorySettingsConfig = Record<string, TrajectorySettingValue>;

export type TrajectorySettingField = {
  name: string;
  value_type: string;
  minimum?: number | null;
  maximum?: number | null;
  apply_mode: string;
};

export type TrajectorySettingsResponse = {
  path?: string;
  config: TrajectorySettingsConfig;
  current: TrajectorySettingsConfig;
  defaults: TrajectorySettingsConfig;
  fields: TrajectorySettingField[];
  environment_precedence?: string;
};

export const performanceApi = createApi({
  reducerPath: "performanceApi",
  baseQuery: fetchBaseQuery({
    prepareHeaders: (headers, { getState }) => {
      const token = (getState() as RootState).config.apiKey;
      if (token) headers.set("Authorization", `Bearer ${token}`);
      return headers;
    },
  }),
  endpoints: (builder) => ({
    getPerformanceTelemetry: builder.query<
      PerformanceTelemetryResponse,
      undefined
    >({
      queryFn: async (_arg, api, _extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, "/v1/performance/telemetry"),
        });
        if (result.error) return { error: result.error };
        return { data: result.data as PerformanceTelemetryResponse };
      },
    }),
    setPerformanceTelemetryEnabled: builder.mutation<
      PerformanceTelemetrySettingsResponse,
      boolean
    >({
      queryFn: async (enabled, api, _extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, "/v1/performance/telemetry"),
          method: "POST",
          body: { enabled },
        });
        if (result.error) return { error: result.error };
        return { data: result.data as PerformanceTelemetrySettingsResponse };
      },
    }),
    resetPerformanceTelemetry: builder.mutation<
      PerformanceTelemetryResetResponse,
      undefined
    >({
      queryFn: async (_arg, api, _extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, "/v1/performance/telemetry/reset"),
          method: "POST",
        });
        if (result.error) return { error: result.error };
        return { data: result.data as PerformanceTelemetryResetResponse };
      },
    }),
    getTrajectorySettings: builder.query<TrajectorySettingsResponse, undefined>(
      {
        queryFn: async (_arg, api, _extraOptions, baseQuery) => {
          const state = api.getState() as RootState;
          const result = await baseQuery({
            url: buildApiUrlFromState(state, "/v1/trajectory-settings"),
          });
          if (result.error) return { error: result.error };
          return { data: result.data as TrajectorySettingsResponse };
        },
      },
    ),
    saveTrajectorySettings: builder.mutation<
      TrajectorySettingsResponse,
      TrajectorySettingsConfig
    >({
      queryFn: async (config, api, _extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, "/v1/trajectory-settings"),
          method: "POST",
          body: config,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as TrajectorySettingsResponse };
      },
    }),
  }),
});

export const {
  useGetTrajectorySettingsQuery,
  useGetPerformanceTelemetryQuery,
  useResetPerformanceTelemetryMutation,
  useSaveTrajectorySettingsMutation,
  useSetPerformanceTelemetryEnabledMutation,
} = performanceApi;
