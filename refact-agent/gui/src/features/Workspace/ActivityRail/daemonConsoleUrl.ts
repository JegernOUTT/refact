import type { EngineApiConfig } from "../../../services/refact/apiUrl";

export function daemonConsoleUrl(
  config: EngineApiConfig,
  currentUrl: string,
): string | null {
  if (config.host !== "web") return null;
  try {
    const current = new URL(currentUrl);
    if (!/^\/p\/[^/]+(?:\/|$)/.test(current.pathname)) return null;
    if (!config.engineServed) return null;
    if (!["http:", "https:"].includes(current.protocol)) return null;
    return `${current.origin}/`;
  } catch {
    return null;
  }
}
