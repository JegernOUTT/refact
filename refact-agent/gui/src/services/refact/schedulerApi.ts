import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import type { FetchBaseQueryError } from "@reduxjs/toolkit/query";
import type { RootState } from "../../app/store";
import { isDetailMessage } from "./commands";
import { buildApiUrlFromState } from "./apiUrl";

export type CronTriggerKind =
  | "cron"
  | "interval"
  | "once"
  | "manual"
  | "webhook";

export type CronRunRecord = {
  at_ms: number;
  status: string;
  error: string | null;
};

export type CronTask = {
  id: string;
  cron: string;
  human_schedule: string;
  description: string;
  prompt: string;
  recurring: boolean;
  durable: boolean;
  next_fire_at_ms: number;
  fire_count: number;
  created_at_ms: number;
  enabled: boolean;
  paused: boolean;
  trigger_kind: CronTriggerKind;
  tz: string | null;
  every_ms: number | null;
  at_ms: number | null;
  last_status: string | null;
  last_error: string | null;
  recent_runs: CronRunRecord[];
};

export type CreateCronRequest = {
  cron?: string;
  every?: string;
  at?: string;
  tz?: string;
  prompt: string;
  recurring?: boolean;
  durable: boolean;
  description: string;
  chat_id: string;
  mode?: string;
};

export type CreateCronResponse = {
  id: string;
  human_schedule: string;
  recurring: boolean;
  durable: boolean;
};

export type UpdateCronRequest = {
  id: string;
  cron?: string;
  every?: string;
  at?: string;
  tz?: string;
  prompt?: string;
  description?: string;
  enabled?: boolean;
  run_now?: boolean;
};

export type UpdateCronResponse = {
  id: string;
  updated: boolean;
  human_schedule: string;
};

export type RunCronRequest = {
  id: string;
};

export type RunCronResponse = {
  id: string;
  triggered: boolean;
};

export type DeleteCronRequest = {
  id: string;
};

export type DeleteCronResponse = {
  removed: boolean;
};

export function schedulerErrorMessage(error: unknown): string {
  if (!error || typeof error !== "object") return "Scheduler request failed";
  const queryError = error as Partial<FetchBaseQueryError>;
  if (isDetailMessage(queryError.data)) return queryError.data.detail;
  if ("error" in queryError && typeof queryError.error === "string") {
    return queryError.error;
  }
  return "Scheduler request failed";
}

export const schedulerApi = createApi({
  reducerPath: "schedulerApi",
  baseQuery: fetchBaseQuery({
    prepareHeaders: (headers, { getState }) => {
      const token = (getState() as RootState).config.apiKey;
      if (token) {
        headers.set("Authorization", `Bearer ${token}`);
      }
      return headers;
    },
  }),
  tagTypes: ["CronTasks"],
  endpoints: (builder) => ({
    getCronTasks: builder.query<CronTask[], undefined>({
      queryFn: async (_args, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, "/v1/scheduler/cron"),
        });
        if (result.error) return { error: result.error };
        return { data: result.data as CronTask[] };
      },
      providesTags: ["CronTasks"],
    }),
    createCron: builder.mutation<CreateCronResponse, CreateCronRequest>({
      queryFn: async (body, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(state, "/v1/scheduler/cron"),
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as CreateCronResponse };
      },
      invalidatesTags: ["CronTasks"],
    }),
    updateCron: builder.mutation<UpdateCronResponse, UpdateCronRequest>({
      queryFn: async ({ id, ...body }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(
            state,
            `/v1/scheduler/cron/${encodeURIComponent(id)}`,
          ),
          method: "PATCH",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as UpdateCronResponse };
      },
      invalidatesTags: ["CronTasks"],
    }),
    runCron: builder.mutation<RunCronResponse, RunCronRequest>({
      queryFn: async ({ id }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(
            state,
            `/v1/scheduler/cron/${encodeURIComponent(id)}/run`,
          ),
          method: "POST",
        });
        if (result.error) return { error: result.error };
        return { data: result.data as RunCronResponse };
      },
      invalidatesTags: ["CronTasks"],
    }),
    deleteCron: builder.mutation<DeleteCronResponse, DeleteCronRequest>({
      queryFn: async ({ id }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const result = await baseQuery({
          url: buildApiUrlFromState(
            state,
            `/v1/scheduler/cron/${encodeURIComponent(id)}`,
          ),
          method: "DELETE",
        });
        if (result.error) return { error: result.error };
        return { data: result.data as DeleteCronResponse };
      },
      invalidatesTags: ["CronTasks"],
    }),
  }),
});

export const {
  useGetCronTasksQuery,
  useCreateCronMutation,
  useUpdateCronMutation,
  useRunCronMutation,
  useDeleteCronMutation,
} = schedulerApi;
