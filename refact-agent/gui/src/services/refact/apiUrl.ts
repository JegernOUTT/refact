export type RefactBackendConnectionStatus =
  | "connecting"
  | "starting"
  | "installing"
  | "ready"
  | "failed";

export type EngineApiConfig = {
  host?: "web" | "ide" | "vscode" | "jetbrains";
  lspPort?: number;
  lspUrl?: string;
  browserUrl?: string;
  dev?: boolean;
  engineServed?: boolean;
  backendReady?: boolean;
  connectionStatus?: RefactBackendConnectionStatus;
};

export type QueryValue = string | number | boolean | null | undefined;
export type QueryParams = Record<string, QueryValue> | URLSearchParams;

const DEFAULT_LSP_PORT = 8001;
const SAME_ORIGIN_IDENTITY = "same-origin";
const ENGINE_V1_ENDPOINT_SEGMENTS = new Set([
  "ast-file-symbols",
  "ast-status",
  "at-command-completion",
  "at-command-preview",
  "caps",
  "chat",
  "chats",
  "checkpoints-preview",
  "checkpoints-restore",
  "codegraph-status",
  "code-intel/overview",
  "code-intel/graph",
  "code-intel/communities",
  "code-intel/dead-code",
  "code-intel/health",
  "code-intel/git-risk",
  "code-intel/duplication",
  "code-intel/pr-blast",
  "code-intel/security-scan",
  "code-completion",
  "code-lens",
  "customization",
  "docker-container-action",
  "docker-container-list",
  "files",
  "git",
  "git-commit",
  "graceful-shutdown",
  "integration-get",
  "integration-save",
  "integrations",
  "knowledge",
  "knowledge-graph",
  "links",
  "ping",
  "bug-report",
  "rag-status",
  "task",
  "tasks",
  "tools",
  "trajectories",
  "vecdb-search",
  "voice",
  "worktrees",
]);

function isEngineV1Suffix(segments: string[], index: number): boolean {
  if (index >= segments.length - 1) return true;
  const nextSegment = segments[index + 1];
  if (nextSegment === "") return true;

  const suffix = segments
    .slice(index + 1)
    .filter((segment) => segment.length > 0)
    .join("/");
  return (
    ENGINE_V1_ENDPOINT_SEGMENTS.has(nextSegment) ||
    ENGINE_V1_ENDPOINT_SEGMENTS.has(suffix)
  );
}

function isValidLspPort(port: number | undefined): boolean {
  return Number.isFinite(port) && port !== undefined && port > 0;
}

function isIdeHost(host: EngineApiConfig["host"]): boolean {
  return host === "ide" || host === "vscode" || host === "jetbrains";
}

export function hasReadyPluginBackend(config: EngineApiConfig): boolean {
  const host = config.host ?? "web";
  if (!isIdeHost(host)) return true;
  if (config.backendReady === false) return false;
  if (
    config.connectionStatus !== undefined &&
    config.connectionStatus !== "ready"
  ) {
    return false;
  }
  return true;
}

function dropV1Path(pathname: string): string {
  const segments = pathname.split("/");

  for (let index = segments.length - 1; index >= 0; index -= 1) {
    if (segments[index] === "v1" && isEngineV1Suffix(segments, index)) {
      const kept = segments.slice(0, index).join("/");
      return kept.replace(/\/+$/, "") || "/";
    }
  }

  return pathname.replace(/\/+$/, "") || "/";
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
    if ((config.dev ?? false) || (config.engineServed ?? false)) return "";
    return sanitizeEngineBaseUrl(config.lspUrl) ?? "";
  }

  return (
    sanitizeEngineBaseUrl(config.lspUrl) ??
    `http://127.0.0.1:${config.lspPort ?? DEFAULT_LSP_PORT}`
  );
}

export function hasUsableEngineEndpoint(config: EngineApiConfig): boolean {
  const host = config.host ?? "web";
  if (!hasReadyPluginBackend(config)) return false;

  const baseUrl = resolveEngineBaseUrl(config);

  if (baseUrl === "") {
    return (
      host === "web" &&
      ((config.dev ?? false) || (config.engineServed ?? false))
    );
  }
  if (sanitizeEngineBaseUrl(config.lspUrl) !== null) return true;
  return host !== "web" && isValidLspPort(config.lspPort);
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
