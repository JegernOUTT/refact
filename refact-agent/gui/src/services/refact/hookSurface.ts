import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import type { RootState } from "../../app/store";
import { buildApiUrlFromState } from "./apiUrl";
import { parseHookSurface, type HookSurface } from "./hookSurfaceTypes";

export type HookSurfaceQuery = { endpoint: string };

export const hookSurfaceApi = createApi({
  reducerPath: "hookSurfaceApi",
  baseQuery: fetchBaseQuery({
    prepareHeaders: (headers, { getState }) => {
      const token = (getState() as RootState).config.apiKey;
      if (token) headers.set("Authorization", `Bearer ${token}`);
      return headers;
    },
  }),
  keepUnusedDataFor: 0,
  endpoints: (builder) => ({
    getHookSurface: builder.query<HookSurface, HookSurfaceQuery>({
      queryFn: async (_query, api, _options, baseQuery) => {
        const result = await baseQuery({
          url: buildApiUrlFromState(api.getState() as RootState, "/v1/hooks"),
        });
        if (result.error) return { error: result.error };
        const data = parseHookSurface(result.data);
        return data
          ? { data }
          : {
              error: {
                status: "CUSTOM_ERROR",
                error: "Invalid hook surface snapshot",
              },
            };
      },
    }),
  }),
});

export const { useGetHookSurfaceQuery } = hookSurfaceApi;
