export type EngineApiConfig = {
  host?: "web" | "ide" | "vscode" | "jetbrains";
  lspPort?: number;
  lspUrl?: string;
  dev?: boolean;
  engineServed?: boolean;
};

export type QueryValue = string | number | boolean | null | undefined;
export type QueryParams = Record<string, QueryValue> | URLSearchParams;

const DEFAULT_LSP_PORT = 8001;
const SAME_ORIGIN_IDENTITY = "same-origin";

function dropV1Path(pathname: string): string {
  const segments = pathname.split("/");
  const v1Index = segments.findIndex((segment) => segment === "v1");
  const kept = v1Index === -1 ? pathname : segments.slice(0, v1Index).join("/");
  return kept.replace(/\/+$/, "") || "/";
}

function appendQuery(url: string, query?: QueryParams): string {
  if (!query) return url;

  const params = new URLSearchParams();
  if (query instanceof URLSearchParams) {
    query.forEach((value, key) => params.append(key, value));
  } else {
    Object.entries(query).forEach(([key, value]) => {
      if (value === null || value === undefined) return;
      params.append(key, String(value));
    });
  }

  const queryString = params.toString();
  return queryString ? `${url}?${queryString}` : url;
}

export function sanitizeEngineBaseUrl(raw: string | undefined): string | null {
  const trimmed = raw?.trim();
  if (!trimmed) return null;

  try {
    const url = new URL(trimmed);
    if (url.protocol !== "http:" && url.protocol !== "https:") return null;

    url.search = "";
    url.hash = "";
    url.pathname = dropV1Path(url.pathname);

    return url.toString().replace(/\/+$/, "");
  } catch {
    return null;
  }
}

export function resolveEngineBaseUrl(config: EngineApiConfig): string {
  const host = config.host ?? "web";

  if (host === "web") {
    if (config.dev || config.engineServed) return "";
    return sanitizeEngineBaseUrl(config.lspUrl) ?? "";
  }

  return (
    sanitizeEngineBaseUrl(config.lspUrl) ??
    `http://127.0.0.1:${config.lspPort ?? DEFAULT_LSP_PORT}`
  );
}

export function normalizeEndpointPath(path: string): string {
  const trimmed = path.trim();
  const withoutLeadingSlash = trimmed.startsWith("/")
    ? trimmed.slice(1)
    : trimmed;

  if (withoutLeadingSlash === "v1" || withoutLeadingSlash.startsWith("v1/")) {
    return `/${withoutLeadingSlash}`;
  }

  throw new Error(`Engine API endpoint must start with /v1/: ${path}`);
}

export function buildApiUrl(
  config: EngineApiConfig,
  path: string,
  query?: QueryParams,
): string {
  const baseUrl = resolveEngineBaseUrl(config);
  const endpointPath = normalizeEndpointPath(path);
  return appendQuery(`${baseUrl}${endpointPath}`, query);
}

export function buildApiUrlFromState(
  state: { config: EngineApiConfig },
  path: string,
  query?: QueryParams,
): string {
  return buildApiUrl(state.config, path, query);
}

/** Legacy local/IDE fallback adapter; it cannot infer dev or engine-served relative mode. */
export function buildApiUrlFromParts(
  port: number,
  lspUrl: string | undefined,
  path: string,
  query?: QueryParams,
): string {
  return buildApiUrl({ host: "ide", lspPort: port, lspUrl }, path, query);
}

export function getEngineEndpointIdentity(config: EngineApiConfig): string {
  return resolveEngineBaseUrl(config) || SAME_ORIGIN_IDENTITY;
}
