export type ConfigPatch = {
  path: (string | number)[];
  value: unknown;
};

const DANGEROUS_KEYS = new Set(["__proto__", "constructor", "prototype"]);

function isDangerousKey(key: string | number): boolean {
  return typeof key === "string" && DANGEROUS_KEYS.has(key);
}

export function applyPatch(obj: Record<string, unknown>, patch: ConfigPatch): Record<string, unknown> {
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

export function applyPatches(obj: Record<string, unknown>, patches: ConfigPatch[]): Record<string, unknown> {
  return patches.reduce((acc, patch) => applyPatch(acc, patch), obj);
}

export function getNestedValue<T>(obj: Record<string, unknown>, path: string[]): T | undefined {
  let current: unknown = obj;
  for (const key of path) {
    if (current === null || current === undefined || typeof current !== "object") {
      return undefined;
    }
    current = (current as Record<string, unknown>)[key];
  }
  return current as T;
}

export function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value) && Object.getPrototypeOf(value) === Object.prototype;
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
  "schema_version", "id", "title", "description", "specific",
  "expose_as_tool", "has_code", "tool", "subchat", "messages",
  "prompts", "gather_files", "tools", "base", "match_models"
]);

export function extractSubagentExtra(config: Record<string, unknown>): Record<string, unknown> {
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
  newExtra: Record<string, unknown>
): ConfigPatch[] {
  const patches: ConfigPatch[] = [];
  const allKeys = new Set([...Object.keys(oldExtra), ...Object.keys(newExtra)]);

  for (const key of allKeys) {
    if (DANGEROUS_KEYS.has(key) || SUBAGENT_KNOWN_KEYS.has(key)) continue;

    if (!(key in newExtra)) {
      patches.push({ path: [key], value: undefined });
    } else if (JSON.stringify(oldExtra[key]) !== JSON.stringify(newExtra[key])) {
      patches.push({ path: [key], value: newExtra[key] });
    }
  }

  return patches;
}

export function safeArray<T>(value: unknown, guard: (v: unknown) => v is T): T[] {
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
