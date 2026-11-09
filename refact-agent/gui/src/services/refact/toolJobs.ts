import type { RootState } from "../../app/store";
import { buddyApi } from "./buddy";
import {
  buildApiUrlFromState,
  getEngineEndpointIdentity,
  hasUsableEngineEndpoint,
} from "./apiUrl";

export type ToolJobStatus =
  | "accepted"
  | "running"
  | "cancel_requested"
  | "succeeded"
  | "failed"
  | "cancelled"
  | "unknown";
export type ToolJobReceipt = {
  job_id: string;
  chat_id: string;
  tool_call_id: string;
  tool_name: string;
  model: string;
  turn_id: string;
  status: ToolJobStatus;
  result_url: string;
  cancel_url: string;
  revision?: number;
  result_available?: boolean;
  terminal?: boolean;
};
export type ToolJobSnapshot = {
  receipt: ToolJobReceipt;
  status: ToolJobStatus;
  revision: number;
};
export type ToolJobQuery = { chatId: string; jobId: string; endpoint: string };

const statuses: readonly string[] = [
  "accepted",
  "running",
  "cancel_requested",
  "succeeded",
  "failed",
  "cancelled",
  "unknown",
];
const record = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);
export function parseToolJobReceipt(value: unknown): ToolJobReceipt | null {
  if (!record(value)) return null;
  for (const key of [
    "job_id",
    "chat_id",
    "tool_name",
    "tool_call_id",
    "turn_id",
    "result_url",
    "cancel_url",
  ] as const) {
    if (typeof value[key] !== "string" || !value[key]) return null;
  }
  if (
    typeof value.model !== "string" ||
    (!value.model && value.tool_name !== "review_readiness")
  )
    return null;
  if (typeof value.status !== "string" || !statuses.includes(value.status))
    return null;
  if (
    (value.revision !== undefined &&
      (!Number.isSafeInteger(value.revision) ||
        (value.revision as number) < 0)) ||
    (value.result_available !== undefined &&
      typeof value.result_available !== "boolean") ||
    (value.terminal !== undefined && typeof value.terminal !== "boolean")
  )
    return null;
  return {
    ...(value.revision !== undefined
      ? { revision: value.revision as number }
      : {}),
    ...(value.result_available !== undefined
      ? { result_available: value.result_available as boolean }
      : {}),
    ...(value.terminal !== undefined
      ? { terminal: value.terminal as boolean }
      : {}),
    job_id: value.job_id as string,
    chat_id: value.chat_id as string,
    tool_call_id: value.tool_call_id as string,
    tool_name: value.tool_name as string,
    model: value.model,
    turn_id: value.turn_id as string,
    status: value.status as ToolJobStatus,
    result_url: value.result_url as string,
    cancel_url: value.cancel_url as string,
  };
}
export function parseToolJobSnapshot(
  value: unknown,
  args: ToolJobQuery,
): ToolJobSnapshot | null {
  if (!record(value)) return null;
  const receipt = parseToolJobReceipt(value.receipt);
  if (
    !receipt ||
    receipt.chat_id !== args.chatId ||
    receipt.job_id !== args.jobId ||
    typeof value.status !== "string" ||
    !statuses.includes(value.status) ||
    !Number.isSafeInteger(value.revision) ||
    (value.revision as number) < 0
  )
    return null;
  // Deliberately discard result data: delivery and privacy filtering belong to the backend.
  return {
    receipt,
    status: value.status as ToolJobStatus,
    revision: value.revision as number,
  };
}
export function isTerminalToolJob(status: ToolJobStatus): boolean {
  return (
    status === "succeeded" ||
    status === "failed" ||
    status === "cancelled" ||
    status === "unknown"
  );
}
const error = (message: string) => ({
  error: { status: "CUSTOM_ERROR" as const, error: message },
});
const current = (state: RootState, endpoint: string) =>
  hasUsableEngineEndpoint(state.config) &&
  getEngineEndpointIdentity(state.config) === endpoint;
const path = ({ chatId, jobId }: ToolJobQuery) =>
  `/v1/chats/${encodeURIComponent(chatId)}/tool-jobs/${encodeURIComponent(
    jobId,
  )}`;

export const toolJobsApi = buddyApi.injectEndpoints({
  endpoints: (builder) => ({
    getToolJob: builder.query<ToolJobSnapshot, ToolJobQuery>({
      async queryFn(args, api, _options, baseQuery) {
        const state = api.getState() as RootState;
        if (!current(state, args.endpoint)) return error("Connection changed");
        const key = state.config.apiKey;
        const response = await baseQuery({
          url: buildApiUrlFromState(state, path(args)),
        });
        const after = api.getState() as RootState;
        if (!current(after, args.endpoint) || after.config.apiKey !== key)
          return error("Connection changed");
        if (response.error) return { error: response.error };
        const data = parseToolJobSnapshot(response.data, args);
        return data ? { data } : error("Invalid job status");
      },
      merge(currentCache, incoming) {
        if (incoming.revision > currentCache.revision) return incoming;
      },
    }),
    cancelToolJob: builder.mutation<
      { job_id: string; status: ToolJobStatus },
      ToolJobQuery
    >({
      async queryFn(args, api, _options, baseQuery) {
        const state = api.getState() as RootState;
        if (!current(state, args.endpoint)) return error("Connection changed");
        const key = state.config.apiKey;
        const response = await baseQuery({
          url: buildApiUrlFromState(state, `${path(args)}/cancel`),
          method: "POST",
        });
        const after = api.getState() as RootState;
        if (!current(after, args.endpoint) || after.config.apiKey !== key)
          return error("Connection changed");
        if (response.error) return { error: response.error };
        const data: unknown = response.data;
        if (
          !record(data) ||
          data.job_id !== args.jobId ||
          typeof data.status !== "string" ||
          !statuses.includes(data.status)
        )
          return error("Invalid cancellation response");
        return {
          data: { job_id: args.jobId, status: data.status as ToolJobStatus },
        };
      },
    }),
  }),
});
export const { useGetToolJobQuery, useCancelToolJobMutation } = toolJobsApi;
