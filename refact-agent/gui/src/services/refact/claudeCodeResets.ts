import { RootState } from "../../app/store";
import { hasProperty } from "../../utils";
import { buildApiUrlFromState } from "./apiUrl";
import { PROVIDERS_URL } from "./consts";
import {
  providersApi,
  type ClaudeCodeResetClaimResponse,
  type ClaudeCodeResetProgram,
  type ClaudeCodeResetStatusResponse,
  type ProviderScopedQueryRequiredArg,
} from "./providers";

function isUsageEnvelope(
  data: unknown,
): data is { data?: unknown; error?: unknown } {
  if (typeof data !== "object" || data === null) return false;
  return hasProperty(data, "data") || hasProperty(data, "error");
}

const resetStatusTagId = (providerName: string) => `resets:${providerName}`;

export const claudeCodeResetsApi = providersApi.injectEndpoints({
  endpoints: (builder) => ({
    getClaudeCodeResetStatus: builder.query<
      ClaudeCodeResetStatusResponse,
      ProviderScopedQueryRequiredArg
    >({
      providesTags: (_result, _error, { providerName }) => [
        { type: "QUOTA", id: resetStatusTagId(providerName) },
      ],
      queryFn: async (args, api, extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(
          state,
          `${PROVIDERS_URL}/${encodeURIComponent(
            args.providerName,
          )}/usage/resets`,
        );

        const controller = new AbortController();
        const timeoutId = setTimeout(() => controller.abort(), 15_000);
        let result: Awaited<ReturnType<typeof baseQuery>>;
        try {
          result = await baseQuery({
            ...extraOptions,
            method: "GET",
            url,
            credentials: "same-origin",
            redirect: "follow",
            signal: controller.signal,
          });
        } finally {
          clearTimeout(timeoutId);
        }

        if (result.error) {
          return { error: result.error };
        }

        if (!isUsageEnvelope(result.data)) {
          return {
            meta: result.meta,
            error: {
              error: `Invalid response from /v1/providers/${args.providerName}/usage/resets`,
              data: result.data,
              status: "CUSTOM_ERROR",
            },
          };
        }

        return { data: result.data as ClaudeCodeResetStatusResponse };
      },
    }),

    claimClaudeCodeReset: builder.mutation<
      ClaudeCodeResetClaimResponse,
      ProviderScopedQueryRequiredArg & {
        program: ClaudeCodeResetProgram;
        grantId?: string;
        requestId?: string;
      }
    >({
      invalidatesTags: (_result, _error, { providerName }) => [
        { type: "QUOTA", id: resetStatusTagId(providerName) },
        { type: "QUOTA", id: providerName },
        { type: "QUOTA", id: "LIST" },
      ],
      queryFn: async (args, api, extraOptions, baseQuery) => {
        const state = api.getState() as RootState;
        const url = buildApiUrlFromState(
          state,
          `${PROVIDERS_URL}/${encodeURIComponent(
            args.providerName,
          )}/usage/redeem`,
        );

        const controller = new AbortController();
        const timeoutId = setTimeout(() => controller.abort(), 40_000);
        let result: Awaited<ReturnType<typeof baseQuery>>;
        try {
          result = await baseQuery({
            ...extraOptions,
            method: "POST",
            url,
            body: {
              program: args.program,
              grant_id: args.grantId,
              request_id: args.requestId,
            },
            credentials: "same-origin",
            redirect: "follow",
            signal: controller.signal,
          });
        } finally {
          clearTimeout(timeoutId);
        }

        if (result.error) {
          return { error: result.error };
        }

        if (!isUsageEnvelope(result.data)) {
          return {
            meta: result.meta,
            error: {
              error: `Invalid response from /v1/providers/${args.providerName}/usage/redeem`,
              data: result.data,
              status: "CUSTOM_ERROR",
            },
          };
        }

        return { data: result.data as ClaudeCodeResetClaimResponse };
      },
    }),
  }),
});

export const {
  useGetClaudeCodeResetStatusQuery,
  useClaimClaudeCodeResetMutation,
} = claudeCodeResetsApi;
