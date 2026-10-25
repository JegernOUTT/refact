import type { SchemaFieldDef } from "./SchemaField";

export function effectiveClaudeBackend(
  baseProvider: string,
  settings: Record<string, unknown>,
): string | undefined {
  if (baseProvider === "anthropic") return "anthropic";
  if (baseProvider !== "claude_code") return undefined;
  return typeof settings.backend === "string" && settings.backend !== ""
    ? settings.backend
    : "oauth";
}

export function isFieldVisible(
  field: SchemaFieldDef,
  fields: SchemaFieldDef[],
  values: Record<string, unknown>,
): boolean {
  const matches = (condition: Record<string, string[]>) =>
    Object.entries(condition).every(([key, allowed]) =>
      allowed.includes(
        String(
          values[key] ??
            fields.find((candidate) => candidate.key === key)?.f_default ??
            "",
        ),
      ),
    );
  return (
    matches(field.f_visible_when ?? {}) &&
    (field.f_visible_when_any === undefined ||
      field.f_visible_when_any.some(matches))
  );
}
