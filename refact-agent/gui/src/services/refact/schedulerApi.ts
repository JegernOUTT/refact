import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import type { FetchBaseQueryError } from "@reduxjs/toolkit/query";
import type { RootState } from "../../app/store";
import { isDetailMessage } from "./commands";

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
};

export type CreateCronRequest = {
  cron: string;
  prompt: string;
  recurring: boolean;
  durable: boolean;
  description: string;
};

export type CreateCronResponse = {
  id: string;
  human_schedule: string;
  recurring: boolean;
  durable: boolean;
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
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/scheduler/cron`,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as CronTask[] };
      },
      providesTags: ["CronTasks"],
    }),
    createCron: builder.mutation<CreateCronResponse, CreateCronRequest>({
      queryFn: async (body, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/scheduler/cron`,
          method: "POST",
          body,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as CreateCronResponse };
      },
      invalidatesTags: ["CronTasks"],
    }),
    deleteCron: builder.mutation<DeleteCronResponse, DeleteCronRequest>({
      queryFn: async ({ id }, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        const port = state.config.lspPort;
        const result = await baseQuery({
          url: `http://127.0.0.1:${port}/v1/scheduler/cron/${encodeURIComponent(
            id,
          )}`,
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
  useDeleteCronMutation,
} = schedulerApi;
