import {
  resolveEngineBaseUrl,
  type EngineApiConfig,
} from "../../services/refact/apiUrl";

function isUsableHttpUrl(value: string | undefined): value is string {
  if (!value) return false;
  try {
    const url = new URL(value);
    return url.protocol === "http:" || url.protocol === "https:";
  } catch {
    return false;
  }
}

function normalizeDisplayUrl(value: string): string {
  return value.replace(/\/+$/, "");
}

function isLocalhostUrl(value: string): boolean {
  try {
    const { hostname } = new URL(value);
    return (
      hostname === "localhost" || hostname === "127.0.0.1" || hostname === "::1"
    );
  } catch {
    return false;
  }
}

function resolveCommonBrowserUrl(config: EngineApiConfig): string | null {
  if (isUsableHttpUrl(config.browserUrl)) {
    return normalizeDisplayUrl(config.browserUrl);
  }

  const candidates = window.__REFACT_ENGINE_ORIGIN_CANDIDATES__ ?? [];
  const mdnsCandidate = candidates.find((candidate) => {
    if (!isUsableHttpUrl(candidate)) return false;
    return new URL(candidate).hostname.endsWith(".local");
  });
  if (mdnsCandidate) return normalizeDisplayUrl(mdnsCandidate);

  const lanCandidate = candidates.find((candidate) => {
    if (!isUsableHttpUrl(candidate)) return false;
    return !isLocalhostUrl(candidate);
  });
  if (lanCandidate) return normalizeDisplayUrl(lanCandidate);

  return null;
}

export function resolveBrowserEngineUrl(config: EngineApiConfig): string {
  const commonUrl = resolveCommonBrowserUrl(config);
  if (commonUrl) return commonUrl;

  const baseUrl = resolveEngineBaseUrl(config);
  if (!baseUrl) return normalizeDisplayUrl(window.location.origin);
  if (baseUrl.startsWith("/")) {
    return normalizeDisplayUrl(
      new URL(baseUrl, window.location.origin).toString(),
    );
  }
  return normalizeDisplayUrl(baseUrl);
}

export function formatEngineHost(engineUrl: string): string {
  try {
    const { host, pathname } = new URL(engineUrl);
    const path = pathname === "/" ? "" : normalizeDisplayUrl(pathname);
    return `${host}${path}`;
  } catch {
    return engineUrl;
  }
}
