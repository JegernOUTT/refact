function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function toolCallArgsToString(toolCallArgs: string): string {
  try {
    const json = JSON.parse(toolCallArgs) as unknown;
    if (Array.isArray(json)) {
      return json.map((value) => JSON.stringify(value)).join(", ");
    }
    if (isRecord(json)) {
      return Object.entries(json)
        .map(([k, v]) => `${k}=${JSON.stringify(v)}`)
        .join(", ");
    }
    return JSON.stringify(json);
  } catch {
    return toolCallArgs;
  }
}
