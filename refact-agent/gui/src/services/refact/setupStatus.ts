import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import { RootState } from "../../app/store";
import { buildApiUrlFromState, hasUsableEngineEndpoint } from "./apiUrl";

export type SetupStatusDetail = {
  project_root?: string | null;
  has_agents_md: boolean;
  has_knowledge: boolean;
  has_trajectories: boolean;
};

export type SetupStatusResponse = {
  configured: boolean;
  reasons: string[];
  detail: SetupStatusDetail;
};

export const setupStatusApi = createApi({
  reducerPath: "setupStatus",
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
    getSetupStatus: builder.query<SetupStatusResponse, undefined>({
      queryFn: async (_args, api, _opts, baseQuery) => {
        const state = api.getState() as RootState;
        if (!hasUsableEngineEndpoint(state.config)) {
          return {
            error: { status: 500, data: "Missing engine endpoint in config" },
          };
        }
        const result = await baseQuery({
          url: buildApiUrlFromState(state, "/v1/setup/status"),
        });
        if (result.error) {
          return { error: result.error };
        }
        return { data: result.data as SetupStatusResponse };
      },
    }),
  }),
});

export const { useGetSetupStatusQuery } = setupStatusApi;
