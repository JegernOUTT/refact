import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";

import type { RootState } from "../../app/store";
import { buildApiUrlFromState } from "./apiUrl";
import type {
  BlastReport,
  CodeIntelCommunity,
  CodeIntelDeadSymbol,
  CodeIntelDuplication,
  CodeIntelGitRisk,
  CodeIntelGraph,
  CodeIntelHealth,
  CodeIntelOverview,
  CodeIntelResponse,
  SecurityFinding,
} from "./types";

export type CodeIntelGraphQuery = { limit?: number } | undefined;
export type CodeIntelListQuery =
  | {
      path?: string;
      limit?: number;
    }
  | undefined;

export type PrBlastRequest = {
  changed_files: string[];
  max_depth?: number;
};

export type SecurityScanPathRequest = {
  path: string;
  lang?: string;
};

export type SecurityScanFilePathRequest = {
  file_path: string;
  lang?: string;
};

export type SecurityScanTextRequest = {
  lang: string;
  text: string;
  path?: string;
  file_path?: string;
};

export type SecurityScanRequest =
  | SecurityScanPathRequest
  | SecurityScanFilePathRequest
  | SecurityScanTextRequest;

export const codeIntelApi = createApi({
  reducerPath: "codeIntelApi",
  baseQuery: fetchBaseQuery({
    prepareHeaders: (headers, { getState }) => {
      const token = (getState() as RootState).config.apiKey;
      if (token) {
        headers.set("Authorization", `Bearer ${token}`);
      }
      return headers;
    },
  }),
  endpoints: (builder) => ({
    getCodeIntelOverview: builder.query<
      CodeIntelResponse<CodeIntelOverview>,
      undefined
    >({
      queryFn: async (_args, api, _extraOptions, baseQuery) => {
        const state = (api.getState as () => RootState)();
        const url = buildApiUrlFromState(state, "/v1/code-intel/overview");
        const result = await baseQuery(url);
        if (result.error) return { error: result.error };
        return {
          data: result.data as CodeIntelResponse<CodeIntelOverview>,
        };
      },
    }),
    getCodeIntelGraph: builder.query<
      CodeIntelResponse<CodeIntelGraph>,
      CodeIntelGraphQuery
    >({
      queryFn: async (args, api, _extraOptions, baseQuery) => {
        const state = (api.getState as () => RootState)();
        const url = buildApiUrlFromState(state, "/v1/code-intel/graph", {
          limit: args?.limit,
        });
        const result = await baseQuery(url);
        if (result.error) return { error: result.error };
        return { data: result.data as CodeIntelResponse<CodeIntelGraph> };
      },
    }),
    getCodeIntelCommunities: builder.query<
      CodeIntelResponse<CodeIntelCommunity[]>,
      undefined
    >({
      queryFn: async (_args, api, _extraOptions, baseQuery) => {
        const state = (api.getState as () => RootState)();
        const url = buildApiUrlFromState(state, "/v1/code-intel/communities");
        const result = await baseQuery(url);
        if (result.error) return { error: result.error };
        return {
          data: result.data as CodeIntelResponse<CodeIntelCommunity[]>,
        };
      },
    }),
    getCodeIntelDeadCode: builder.query<
      CodeIntelResponse<CodeIntelDeadSymbol[]>,
      undefined
    >({
      queryFn: async (_args, api, _extraOptions, baseQuery) => {
        const state = (api.getState as () => RootState)();
        const url = buildApiUrlFromState(state, "/v1/code-intel/dead-code");
        const result = await baseQuery(url);
        if (result.error) return { error: result.error };
        return {
          data: result.data as CodeIntelResponse<CodeIntelDeadSymbol[]>,
        };
      },
    }),
    getCodeIntelHealth: builder.query<
      CodeIntelResponse<CodeIntelHealth>,
      CodeIntelListQuery
    >({
      queryFn: async (args, api, _extraOptions, baseQuery) => {
        const state = (api.getState as () => RootState)();
        const url = buildApiUrlFromState(state, "/v1/code-intel/health", {
          path: args?.path,
          limit: args?.limit,
        });
        const result = await baseQuery(url);
        if (result.error) return { error: result.error };
        return { data: result.data as CodeIntelResponse<CodeIntelHealth> };
      },
    }),
    getCodeIntelGitRisk: builder.query<
      CodeIntelResponse<CodeIntelGitRisk>,
      CodeIntelListQuery
    >({
      queryFn: async (args, api, _extraOptions, baseQuery) => {
        const state = (api.getState as () => RootState)();
        const url = buildApiUrlFromState(state, "/v1/code-intel/git-risk", {
          path: args?.path,
          limit: args?.limit,
        });
        const result = await baseQuery(url);
        if (result.error) return { error: result.error };
        return { data: result.data as CodeIntelResponse<CodeIntelGitRisk> };
      },
    }),
    getCodeIntelDuplication: builder.query<
      CodeIntelResponse<CodeIntelDuplication>,
      CodeIntelListQuery
    >({
      queryFn: async (args, api, _extraOptions, baseQuery) => {
        const state = (api.getState as () => RootState)();
        const url = buildApiUrlFromState(state, "/v1/code-intel/duplication", {
          path: args?.path,
          limit: args?.limit,
        });
        const result = await baseQuery(url);
        if (result.error) return { error: result.error };
        return { data: result.data as CodeIntelResponse<CodeIntelDuplication> };
      },
    }),
    prBlast: builder.mutation<CodeIntelResponse<BlastReport>, PrBlastRequest>({
      queryFn: async (args, api, _extraOptions, baseQuery) => {
        const state = (api.getState as () => RootState)();
        const url = buildApiUrlFromState(state, "/v1/code-intel/pr-blast");
        const result = await baseQuery({
          url,
          method: "POST",
          body: args,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as CodeIntelResponse<BlastReport> };
      },
    }),
    securityScan: builder.mutation<
      CodeIntelResponse<SecurityFinding[]>,
      SecurityScanRequest
    >({
      queryFn: async (args, api, _extraOptions, baseQuery) => {
        const state = (api.getState as () => RootState)();
        const url = buildApiUrlFromState(state, "/v1/code-intel/security-scan");
        const result = await baseQuery({
          url,
          method: "POST",
          body: args,
        });
        if (result.error) return { error: result.error };
        return { data: result.data as CodeIntelResponse<SecurityFinding[]> };
      },
    }),
  }),
  refetchOnMountOrArgChange: true,
});

export const {
  useGetCodeIntelOverviewQuery,
  useGetCodeIntelGraphQuery,
  useGetCodeIntelCommunitiesQuery,
  useGetCodeIntelDeadCodeQuery,
  useGetCodeIntelHealthQuery,
  useGetCodeIntelGitRiskQuery,
  useGetCodeIntelDuplicationQuery,
  usePrBlastMutation,
  useSecurityScanMutation,
} = codeIntelApi;
