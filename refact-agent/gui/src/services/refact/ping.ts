import { PING_URL } from "./consts";
import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import {
  buildApiUrl,
  getEngineEndpointIdentity,
  type EngineApiConfig,
} from "./apiUrl";

type PingArgs = EngineApiConfig;

export const pingApi = createApi({
  reducerPath: "pingApi",
  baseQuery: fetchBaseQuery(),
  tagTypes: ["PING"],
  endpoints: (builder) => ({
    ping: builder.query<string, PingArgs>({
      providesTags: (_result, _error, args) => [
        { type: "PING", id: getEngineEndpointIdentity(args) },
      ],
      forceRefetch: ({ currentArg, previousArg }) =>
        getEngineEndpointIdentity(currentArg ?? {}) !==
        getEngineEndpointIdentity(previousArg ?? {}),
      queryFn: async (args, _api, _extraOptions, baseQuery) => {
        const url = buildApiUrl(args, PING_URL);

        const response = await baseQuery({
          method: "GET",
          url,
          redirect: "follow",
          cache: "no-cache",
          responseHandler: "text",
        });

        if (response.error) {
          return {
            error: response.error,
          };
        }

        if (response.data && typeof response.data === "string") {
          return { data: response.data };
        } else {
          return {
            error: {
              status: "FETCH_ERROR",
              error: "No data received in response",
            },
          };
        }
      },
    }),
    reset: builder.mutation<null, undefined>({
      queryFn: () => ({ data: null }),
      invalidatesTags: ["PING"],
    }),
  }),
});
