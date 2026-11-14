export type RuntimeVersionObservation = {
  state: "observed" | "last_observed";
  version?: string | null;
  commit?: string | null;
  source?: "plugin" | "local" | "daemon" | "worker";
  observedAtMs?: number;
};

export type RuntimeVersions = {
  plugin?: RuntimeVersionObservation | null;
  selected?: RuntimeVersionObservation | null;
  installed?: RuntimeVersionObservation | null;
  staged?: RuntimeVersionObservation | null;
  daemon?: RuntimeVersionObservation | null;
  worker?: RuntimeVersionObservation | null;
  requestedVersion?: string | null;
};

export function boundedVersionString(value: unknown): string | undefined {
  if (typeof value !== "string") return undefined;
  const trimmed = value.trim();
  if (
    !trimmed ||
    trimmed.length > 160 ||
    Array.from(trimmed).some(
      (character) =>
        character.charCodeAt(0) < 32 || character.charCodeAt(0) === 127,
    )
  )
    return undefined;
  return trimmed;
}

export function sanitizeObservation(
  value: unknown,
): RuntimeVersionObservation | null {
  if (!value || typeof value !== "object") return null;
  const input = value as Record<string, unknown>;
  if (input.state !== "observed" && input.state !== "last_observed")
    return null;
  const observation: RuntimeVersionObservation = { state: input.state };
  observation.version = boundedVersionString(input.version) ?? null;
  observation.commit = boundedVersionString(input.commit) ?? null;
  if (
    input.source === "plugin" ||
    input.source === "local" ||
    input.source === "daemon" ||
    input.source === "worker"
  )
    observation.source = input.source;
  if (
    typeof input.observedAtMs === "number" &&
    Number.isSafeInteger(input.observedAtMs) &&
    input.observedAtMs >= 0 &&
    input.observedAtMs <= 8640000000000000
  )
    observation.observedAtMs = input.observedAtMs;
  return observation;
}

export function sanitizeRuntimeVersions(
  value: unknown,
): RuntimeVersions | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const input = value as Record<string, unknown>;
  const snapshot: RuntimeVersions = {};
  for (const key of [
    "plugin",
    "selected",
    "installed",
    "staged",
    "daemon",
    "worker",
  ] as const) {
    if (Object.prototype.hasOwnProperty.call(input, key))
      snapshot[key] = sanitizeObservation(input[key]);
  }
  if (Object.prototype.hasOwnProperty.call(input, "requestedVersion"))
    snapshot.requestedVersion =
      boundedVersionString(input.requestedVersion) ?? null;
  return snapshot;
}
