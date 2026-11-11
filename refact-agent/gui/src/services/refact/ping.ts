import { PING_URL } from "./consts";
import { createApi, fetchBaseQuery } from "@reduxjs/toolkit/query/react";
import {
  buildApiUrl,
  getEngineEndpointIdentity,
  type EngineApiConfig,
} from "./apiUrl";

import { setBackendStatus } from "../../features/Connection";
import type { RootState } from "../../app/store";
import { capsApi } from "./caps";

type PingArgs = EngineApiConfig;
type FailureRecord = { failures: number; settled: number };

const records = new WeakMap<object, Map<string, FailureRecord>>();

function recordFor(store: object, identity: string): FailureRecord {
  let byIdentity = records.get(store);
  if (!byIdentity) {
    byIdentity = new Map();
    records.set(store, byIdentity);
  }
  let record = byIdentity.get(identity);
  if (!record) {
    record = { failures: 0, settled: 0 };
    byIdentity.set(identity, record);
  }
  return record;
}

function describeFailure(reason: unknown): string {
  const error =
    reason && typeof reason === "object" && "error" in reason
      ? reason.error
      : reason;
  if (error && typeof error === "object") {
    if ("message" in error && typeof error.message === "string")
      return error.message;
    if ("error" in error && typeof error.error === "string") return error.error;
    if ("status" in error && typeof error.status === "number") {
      const detail =
        "data" in error && typeof error.data === "string" && error.data
          ? `: ${error.data}`
          : "";
      return `Ping failed with HTTP ${error.status}${detail}`;
    }
  }
  return "Connection failed";
}

export const pingApi = createApi({
  reducerPath: "pingApi",
  baseQuery: fetchBaseQuery(),
  tagTypes: ["PING"],
  endpoints: (builder) => ({
    ping: builder.query<string, PingArgs>({
      serializeQueryArgs: ({ queryArgs }) =>
        getEngineEndpointIdentity(queryArgs),
      async onCacheEntryAdded(args, { cacheEntryRemoved, dispatch }) {
        await cacheEntryRemoved;
        records.get(dispatch)?.delete(getEngineEndpointIdentity(args));
      },
      async onQueryStarted(args, { dispatch, getState, queryFulfilled }) {
        const identity = getEngineEndpointIdentity(args);
        const record = recordFor(dispatch, identity);
        const startedAfter = record.settled;
        const isCurrent = () =>
          getEngineEndpointIdentity((getState() as RootState).config) ===
          identity;
        try {
          await queryFulfilled;
          if (!isCurrent()) return;
          const recovered = record.failures > 0;
          record.failures = 0;
          record.settled++;
          dispatch(setBackendStatus({ status: "online" }));
          if (recovered) dispatch(capsApi.util.invalidateTags(["CAPS"]));
        } catch (reason) {
          // A failure that overlapped an already settled request belongs to the same poll.
          if (!isCurrent() || record.settled !== startedAfter) return;
          record.failures++;
          record.settled++;
          if (record.failures >= 2)
            dispatch(
              setBackendStatus({
                status: "offline",
                error: describeFailure(reason),
              }),
            );
        }
      },
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
