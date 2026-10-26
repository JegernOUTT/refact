import type { SchemaFieldDef } from "./SchemaField";

export type SchemaConditionMap = Record<string, string[]>;

export type SchemaConditions = {
  f_visible_when?: SchemaConditionMap;
  f_visible_when_any?: SchemaConditionMap[];
};

export type SchemaOptionDef = SchemaConditions & {
  value: string;
  label: string;
};

export type OAuthMethodDef = SchemaConditions & {
  id: string;
  label: string;
  description?: string;
};

export type ParsedOAuthSchema = SchemaConditions & {
  supported: boolean;
  warning?: string;
  methods?: OAuthMethodDef[];
};

export type ClaudeLoginKind = "subscription" | "console" | "native";

const CLAUDE_NATIVE_BACKENDS = ["bedrock", "vertex", "foundry"];

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

export function effectiveClaudeAuthType(
  settings: Record<string, unknown>,
): string {
  return typeof settings.auth_type === "string" && settings.auth_type !== ""
    ? settings.auth_type
    : "api_key";
}

export function claudeLoginKind(
  baseProvider: string,
  backend: string | undefined,
  authType: string | undefined,
): ClaudeLoginKind | undefined {
  if (baseProvider !== "claude_code") return undefined;
  if (backend === undefined || backend === "oauth") return "subscription";
  if (backend === "anthropic") {
    return authType === "oauth" ? "console" : undefined;
  }
  return CLAUDE_NATIVE_BACKENDS.includes(backend) ? "native" : undefined;
}

function asRecord(raw: unknown): Record<string, unknown> | null {
  return typeof raw === "object" && raw !== null && !Array.isArray(raw)
    ? (raw as Record<string, unknown>)
    : null;
}

export function parseConditionMap(
  raw: unknown,
): SchemaConditionMap | undefined {
  const record = asRecord(raw);
  if (!record) return undefined;
  return Object.fromEntries(
    Object.entries(record).filter(
      (entry): entry is [string, string[]] =>
        Array.isArray(entry[1]) &&
        entry[1].every((value: unknown) => typeof value === "string"),
    ),
  );
}

export function parseConditionList(
  raw: unknown,
): SchemaConditionMap[] | undefined {
  if (!Array.isArray(raw)) return undefined;
  return raw.filter(
    (condition: unknown): condition is SchemaConditionMap =>
      typeof condition === "object" &&
      condition !== null &&
      Object.values(condition).every(
        (allowed: unknown) =>
          Array.isArray(allowed) &&
          allowed.every((value: unknown) => typeof value === "string"),
      ),
  );
}

function parseConditions(raw: Record<string, unknown>): SchemaConditions {
  const conditions: SchemaConditions = {};
  const when = parseConditionMap(raw.f_visible_when);
  const whenAny = parseConditionList(raw.f_visible_when_any);
  if (when) conditions.f_visible_when = when;
  if (whenAny) conditions.f_visible_when_any = whenAny;
  return conditions;
}

export function parseSchemaOptions(
  raw: unknown,
): SchemaOptionDef[] | undefined {
  if (!Array.isArray(raw)) return undefined;
  const options: SchemaOptionDef[] = [];
  for (const item of raw) {
    const option = asRecord(item);
    if (
      !option ||
      typeof option.value !== "string" ||
      typeof option.label !== "string"
    )
      continue;
    options.push({
      value: option.value,
      label: option.label,
      ...parseConditions(option),
    });
  }
  return options;
}

export function parseOAuthSchema(raw: unknown): ParsedOAuthSchema | undefined {
  const oauth = asRecord(raw);
  if (!oauth) return undefined;
  let methods: OAuthMethodDef[] | undefined;
  if (Array.isArray(oauth.methods)) {
    methods = [];
    for (const item of oauth.methods) {
      const method = asRecord(item);
      if (!method || typeof method.id !== "string") continue;
      methods.push({
        id: method.id,
        label: typeof method.label === "string" ? method.label : method.id,
        description:
          typeof method.description === "string"
            ? method.description
            : undefined,
        ...parseConditions(method),
      });
    }
  }
  return {
    supported: oauth.supported === true,
    warning: typeof oauth.warning === "string" ? oauth.warning : undefined,
    methods,
    ...parseConditions(oauth),
  };
}

function hasConditions(item: SchemaConditions): boolean {
  return (
    item.f_visible_when !== undefined || item.f_visible_when_any !== undefined
  );
}

export function matchesSchemaConditions(
  item: SchemaConditions,
  fields: SchemaFieldDef[],
  values: Record<string, unknown>,
): boolean {
  const matches = (condition: SchemaConditionMap) =>
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
    matches(item.f_visible_when ?? {}) &&
    (item.f_visible_when_any === undefined ||
      item.f_visible_when_any.some(matches))
  );
}

export function isFieldVisible(
  field: SchemaFieldDef,
  fields: SchemaFieldDef[],
  values: Record<string, unknown>,
): boolean {
  return matchesSchemaConditions(field, fields, values);
}

export function visibleFieldOptions(
  field: SchemaFieldDef,
  fields: SchemaFieldDef[],
  values: Record<string, unknown>,
  baseProvider: string,
): SchemaFieldDef["f_options"] {
  if (!field.f_options) return undefined;
  const options: SchemaOptionDef[] = field.f_options;
  return options.filter(
    (option) =>
      !(
        baseProvider === "claude_code" &&
        field.key === "auth_type" &&
        option.value === "oauth" &&
        effectiveClaudeBackend(baseProvider, values) !== "anthropic"
      ) && matchesSchemaConditions(option, fields, values),
  );
}

export function isOAuthLoginAvailable(
  oauth: ParsedOAuthSchema | undefined,
  fields: SchemaFieldDef[],
  values: Record<string, unknown>,
): boolean {
  if (oauth?.supported !== true) return false;
  if (!matchesSchemaConditions(oauth, fields, values)) return false;
  const methods = oauth.methods ?? [];
  if (!methods.some(hasConditions)) return true;
  return methods.some(
    (method) =>
      !hasConditions(method) || matchesSchemaConditions(method, fields, values),
  );
}
