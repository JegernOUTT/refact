const fallbackInstallError = "Failed to install MCP server";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function stringifyErrorData(value: unknown): string | null {
  if (typeof value === "string") return value;
  if (isRecord(value) && typeof value.detail === "string") return value.detail;
  return null;
}

/**
 * Extracts a human-readable message from an RTK Query install error,
 * preferring the engine's `{ detail }` payload (e.g. the 422 missing
 * required env vars explanation).
 */
export function installErrorMessage(error: unknown): string {
  if (!isRecord(error)) return fallbackInstallError;
  if ("data" in error) {
    return stringifyErrorData(error.data) ?? fallbackInstallError;
  }
  if (typeof error.error === "string") return error.error;
  if (typeof error.message === "string") return error.message;
  return fallbackInstallError;
}

/** Stable key for "is this marketplace server installed" lookups. */
export function installedKey(sourceId: string, serverId: string): string {
  return `${sourceId}\u0000${serverId}`;
}
