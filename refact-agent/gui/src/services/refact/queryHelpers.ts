import type { FetchArgs, FetchBaseQueryError } from "@reduxjs/toolkit/query";
import type { EngineApiConfig } from "./apiUrl";
import { buildApiUrlFromState } from "./apiUrl";

type QueryState = { config: EngineApiConfig };

type InnerBaseQuery = (
  arg: string | FetchArgs,
) => Promise<
  | { data: unknown; error?: undefined }
  | { error: FetchBaseQueryError; data?: undefined }
>;

function isEngineApiPath(url: string): boolean {
  const trimmed = url.trim();
  return (
    trimmed === "v1" ||
    trimmed.startsWith("v1/") ||
    trimmed === "/v1" ||
    trimmed.startsWith("/v1/")
  );
}

function isLoopbackHost(hostname: string): boolean {
  return (
    hostname === "127.0.0.1" || hostname === "localhost" || hostname === "[::1]"
  );
}

function normalizeRequestUrl(state: QueryState, url: string): string {
  const trimmed = url.trim();
  if (isEngineApiPath(trimmed)) {
    return buildApiUrlFromState(state, trimmed);
  }

  try {
    const parsed = new URL(trimmed);
    const path = `${parsed.pathname}${parsed.search}`;
    if (isLoopbackHost(parsed.hostname) && isEngineApiPath(path)) {
      return buildApiUrlFromState(state, path);
    }
  } catch {
    return url;
  }

  return url;
}

function normalizeRequest(
  state: QueryState,
  request: string | FetchArgs,
): FetchArgs {
  if (typeof request === "string") {
    return { url: normalizeRequestUrl(state, request) };
  }

  return { ...request, url: normalizeRequestUrl(state, request.url) };
}

export function lspQueryFn<TArg, TResult>(
  buildRequest: (arg: TArg, state: QueryState) => string | FetchArgs,
) {
  return async (
    arg: TArg,
    api: { getState: () => unknown },
    _opts: object,
    baseQuery: InnerBaseQuery,
  ) => {
    const state = api.getState() as QueryState;
    const request = normalizeRequest(state, buildRequest(arg, state));
    const result = await baseQuery(request);
    if (result.error) {
      return {
        error: {
          status: result.error.status as number,
          data: result.error.data ? String(result.error.data) : "Unknown error",
        } as FetchBaseQueryError,
      };
    }
    return { data: result.data as TResult };
  };
}
