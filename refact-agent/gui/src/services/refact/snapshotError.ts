export type SnapshotFailure = {
  kind: "transport" | "http" | "visibility" | "malformed" | "reset";
  status?: number | "CUSTOM_ERROR";
  data?: { code: string };
};

export function publicSnapshotCode(value: unknown): string | undefined {
  if (typeof value !== "object" || value === null || !("code" in value))
    return undefined;
  const code = value.code;
  return typeof code === "string" &&
    [
      "unauthorized",
      "scope_denied",
      "policy_unavailable",
      "not_found_or_not_visible",
      "invalid_argument",
      "invalid_cursor",
      "cursor_expired",
      "unavailable",
    ].includes(code)
    ? code
    : undefined;
}

export class SnapshotVisibilityError extends Error {
  readonly failure: SnapshotFailure;

  constructor(message: string, data: string) {
    super(message);
    let code = "policy_unavailable";
    try {
      const value: unknown = JSON.parse(data);
      code = publicSnapshotCode(value) ?? code;
    } catch {
      code = "policy_unavailable";
    }
    this.failure = { kind: "visibility", data: { code } };
  }
}
