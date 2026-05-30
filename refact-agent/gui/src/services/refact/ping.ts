import { PING_URL } from "./consts";
import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";

type PingArgs = {
  port: number;
  lspUrl?: string;
};

function lspBaseUrl(port: number, lspUrl?: string): string {
  const trimmed = lspUrl?.trim();
  return (trimmed ? trimmed : `http://127.0.0.1:${port}`).replace(/\/+$/, "");
}

export const pingApi = createApi({
  reducerPath: "pingApi",
  baseQuery: fetchBaseQuery({ baseUrl: PING_URL }),
  tagTypes: ["PING"],
  endpoints: (builder) => ({
    ping: builder.query<string, PingArgs>({
      providesTags: (_result, _error, args) => [
        { type: "PING", id: `${args.lspUrl ?? ""}:${args.port}` },
      ],
      forceRefetch: ({ currentArg, previousArg }) =>
        currentArg?.port !== previousArg?.port || currentArg?.lspUrl !== previousArg?.lspUrl,
      queryFn: async (args, _api, _extraOptions, baseQuery) => {
        const url = `${lspBaseUrl(args.port, args.lspUrl)}${PING_URL}`;

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
