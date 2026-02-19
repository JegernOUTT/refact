export type ConfigPatch = {
  path: (string | number)[];
  value: unknown;
};

const DANGEROUS_KEYS = new Set(["__proto__", "constructor", "prototype"]);

function isDangerousKey(key: string | number): boolean {
  return typeof key === "string" && DANGEROUS_KEYS.has(key);
}

export function applyPatch(
  obj: Record<string, unknown>,
  patch: ConfigPatch,
): Record<string, unknown> {
  if (patch.path.some(isDangerousKey)) {
    return obj;
  }

  if (patch.path.length === 0) {
    if (isPlainObject(patch.value)) {
      return sanitizeObject(patch.value) as Record<string, unknown>;
    }
    return obj;
  }

  const result = { ...obj };
  let current: Record<string, unknown> = result;

  for (let i = 0; i < patch.path.length - 1; i++) {
    const key = patch.path[i];
    const nextKey = patch.path[i + 1];
    const existing = current[key];

    if (Array.isArray(existing)) {
      current[key] = (existing as unknown[]).slice();
    } else if (isPlainObject(existing)) {
      current[key] = { ...existing };
    } else {
      current[key] = typeof nextKey === "number" ? [] : {};
    }
    current = current[key] as Record<string, unknown>;
  }

  const lastKey = patch.path[patch.path.length - 1];
  if (patch.value === undefined) {
    Reflect.deleteProperty(current, lastKey);
  } else {
    current[lastKey] = sanitizeObject(patch.value);
  }

  return result;
}

export function applyPatches(
  obj: Record<string, unknown>,
  patches: ConfigPatch[],
): Record<string, unknown> {
  return patches.reduce((acc, patch) => applyPatch(acc, patch), obj);
}

export function getNestedValue<T>(
  obj: Record<string, unknown>,
  path: string[],
): T | undefined {
  let current: unknown = obj;
  for (const key of path) {
    if (
      current === null ||
      current === undefined ||
      typeof current !== "object"
    ) {
      return undefined;
    }
    current = (current as Record<string, unknown>)[key];
  }
  return current as T;
}

export function isPlainObject(
  value: unknown,
): value is Record<string, unknown> {
  return (
    typeof value === "object" &&
    value !== null &&
    !Array.isArray(value) &&
    Object.getPrototypeOf(value) === Object.prototype
  );
}

export function sanitizeObject(obj: unknown): unknown {
  if (!isPlainObject(obj)) {
    if (Array.isArray(obj)) {
      return obj.map(sanitizeObject);
    }
    return obj;
  }

  const result: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(obj)) {
    if (key === "__proto__" || key === "constructor" || key === "prototype") {
      continue;
    }
    result[key] = sanitizeObject(value);
  }
  return result;
}

const SUBAGENT_KNOWN_KEYS = new Set([
  "schema_version",
  "id",
  "title",
  "description",
  "specific",
  "expose_as_tool",
  "has_code",
  "tool",
  "subchat",
  "messages",
  "prompts",
  "gather_files",
  "tools",
  "base",
  "match_models",
]);

export function extractSubagentExtra(
  config: Record<string, unknown>,
): Record<string, unknown> {
  const extra: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(config)) {
    if (!SUBAGENT_KNOWN_KEYS.has(key) && !DANGEROUS_KEYS.has(key)) {
      extra[key] = value;
    }
  }
  return extra;
}

export function computeExtraPatches(
  oldExtra: Record<string, unknown>,
  newExtra: Record<string, unknown>,
): ConfigPatch[] {
  const patches: ConfigPatch[] = [];
  const allKeys = new Set([...Object.keys(oldExtra), ...Object.keys(newExtra)]);

  for (const key of allKeys) {
    if (DANGEROUS_KEYS.has(key) || SUBAGENT_KNOWN_KEYS.has(key)) continue;

    if (!(key in newExtra)) {
      patches.push({ path: [key], value: undefined });
    } else if (
      JSON.stringify(oldExtra[key]) !== JSON.stringify(newExtra[key])
    ) {
      patches.push({ path: [key], value: newExtra[key] });
    }
  }

  return patches;
}

export function safeArray<T>(
  value: unknown,
  guard: (v: unknown) => v is T,
): T[] {
  if (!Array.isArray(value)) return [];
  return value.filter(guard);
}

export function safeString(value: unknown): string {
  return typeof value === "string" ? value : "";
}

export function safeBoolean(value: unknown): boolean {
  return typeof value === "boolean" ? value : false;
}

export function safeNumber(value: unknown): number | undefined {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  return undefined;
}

export function safeObject(value: unknown): Record<string, unknown> {
  return isPlainObject(value) ? value : {};
}

export function isString(v: unknown): v is string {
  return typeof v === "string";
}

export type MessageTemplate = {
  role: string;
  content: string;
};

export function isMessageTemplate(v: unknown): v is MessageTemplate {
  return (
    isPlainObject(v) &&
    typeof v.role === "string" &&
    typeof v.content === "string"
  );
}

export function safeMessageArray(value: unknown): MessageTemplate[] {
  if (!Array.isArray(value)) return [];
  return value.filter(isMessageTemplate);
}

export function safeSelectionRange(value: unknown): [number, number] | null {
  if (!Array.isArray(value) || value.length !== 2) return null;
  const min: unknown = value[0];
  const max: unknown = value[1];
  if (typeof min !== "number" || typeof max !== "number") return null;
  if (!Number.isFinite(min) || !Number.isFinite(max)) return null;
  return [min, max];
}

export function parseIntSafe(value: string): number | undefined {
  if (!value) return undefined;
  const n = Number.parseInt(value, 10);
  return Number.isFinite(n) ? n : undefined;
}

export function parseFloatSafe(value: string): number | undefined {
  if (!value) return undefined;
  const n = Number.parseFloat(value);
  return Number.isFinite(n) ? n : undefined;
}

export function isToolConfirmRule(
  v: unknown,
): v is { match: string; action: string } {
  return (
    isPlainObject(v) &&
    typeof v.match === "string" &&
    typeof v.action === "string"
  );
}

export function safeToolConfirmRules(
  value: unknown,
): { match: string; action: string }[] {
  if (!Array.isArray(value)) return [];
  return value.filter(isToolConfirmRule);
}

const ID_PATTERN = /^[a-z0-9_-]+$/;

export function validateConfigId(id: string): string | null {
  if (!id.trim()) return "ID is required";
  if (id.includes("/") || id.includes("\\") || id.includes(".."))
    return "ID contains invalid characters";
  if (!ID_PATTERN.test(id))
    return "ID must contain only lowercase letters, digits, underscore, or hyphen";
  return null;
}
