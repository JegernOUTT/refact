export const hookTrusts = [
  "global",
  "trusted_project",
  "untrusted_project",
  "installed_plugin",
] as const;
export type HookTrust = (typeof hookTrusts)[number];

export const hookOutcomes = [
  "allow",
  "deny",
  "rewrite",
  "timeout",
  "cancelled",
  "failed",
  "observed",
  "skipped",
] as const;
export type HookOutcome = (typeof hookOutcomes)[number];

export const hookVerdicts = [
  "allow",
  "deny",
  "rewrite",
  "observed",
  "recursion_blocked",
  "rewrite_accepted",
  "rewrite_refused",
] as const;
export type HookVerdict = (typeof hookVerdicts)[number];

export type HookRegistration = {
  event: string;
  kind: string;
  matcher?: string | null;
  command: string;
  source: string;
  trust: HookTrust;
  runs: boolean;
  blocked_reason?: string | null;
};

export type HookRun = {
  command: string;
  source: string;
  trust: HookTrust;
  outcome: HookOutcome;
  detail?: string | null;
  decided: boolean;
};

export type HookActivity = {
  event: string;
  kind: string;
  verdict: HookVerdict;
  decided_by?: string | null;
  reason?: string | null;
  tool?: string | null;
  runs: HookRun[];
  at_ms: number;
};

export type HookSurface = {
  enabled: boolean;
  registrations: HookRegistration[];
  recent: HookActivity[];
  registered: number;
  runnable: number;
  blocked: number;
  denials: number;
  max_recursion_depth: number;
  generated_at_ms: number;
};

const object = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);
const integer = (value: unknown): value is number =>
  typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
const optionalString = (value: unknown): value is string | null | undefined =>
  value === undefined || value === null || typeof value === "string";

export function isHookRegistration(value: unknown): value is HookRegistration {
  return (
    object(value) &&
    typeof value.event === "string" &&
    typeof value.kind === "string" &&
    optionalString(value.matcher) &&
    typeof value.command === "string" &&
    typeof value.source === "string" &&
    hookTrusts.some((trust) => trust === value.trust) &&
    typeof value.runs === "boolean" &&
    optionalString(value.blocked_reason)
  );
}

function isHookRun(value: unknown): value is HookRun {
  return (
    object(value) &&
    typeof value.command === "string" &&
    typeof value.source === "string" &&
    hookTrusts.some((trust) => trust === value.trust) &&
    hookOutcomes.some((outcome) => outcome === value.outcome) &&
    optionalString(value.detail) &&
    typeof value.decided === "boolean"
  );
}

export function isHookActivity(value: unknown): value is HookActivity {
  return (
    object(value) &&
    typeof value.event === "string" &&
    typeof value.kind === "string" &&
    hookVerdicts.some((verdict) => verdict === value.verdict) &&
    optionalString(value.decided_by) &&
    optionalString(value.reason) &&
    optionalString(value.tool) &&
    Array.isArray(value.runs) &&
    value.runs.every(isHookRun) &&
    integer(value.at_ms)
  );
}

export function parseHookSurface(value: unknown): HookSurface | null {
  if (
    !object(value) ||
    typeof value.enabled !== "boolean" ||
    !Array.isArray(value.registrations) ||
    !value.registrations.every(isHookRegistration) ||
    !Array.isArray(value.recent) ||
    !value.recent.every(isHookActivity) ||
    !integer(value.registered) ||
    !integer(value.runnable) ||
    !integer(value.blocked) ||
    !integer(value.denials) ||
    !integer(value.max_recursion_depth) ||
    !integer(value.generated_at_ms)
  )
    return null;
  if (value.registrations.length !== value.registered) return null;
  // A hook that cannot run must never appear as one that did: refuse a snapshot
  // that credits a run to a hook the engine says is blocked.
  const blockedCommands = new Set(
    value.registrations
      .filter((entry) => !entry.runs)
      .map((entry) => entry.command),
  );
  if (
    value.recent.some((activity) =>
      activity.runs.some((run) => blockedCommands.has(run.command)),
    )
  )
    return null;
  return {
    ...(value as HookSurface),
    registrations: value.registrations.map((entry) => ({
      ...entry,
      matcher: entry.matcher ?? undefined,
      blocked_reason: entry.blocked_reason ?? undefined,
    })),
    recent: value.recent.map((activity) => ({
      ...activity,
      decided_by: activity.decided_by ?? undefined,
      reason: activity.reason ?? undefined,
      tool: activity.tool ?? undefined,
      runs: activity.runs.map((run) => ({
        ...run,
        detail: run.detail ?? undefined,
      })),
    })),
  };
}

/** The hook whose answer deny precedence actually selected, if any. */
export function decidingRun(activity: HookActivity): HookRun | undefined {
  return activity.runs.find((run) => run.decided);
}

export function isDenial(activity: HookActivity): boolean {
  return (
    activity.verdict === "deny" ||
    activity.verdict === "recursion_blocked" ||
    activity.verdict === "rewrite_refused"
  );
}

export function untrustedRegistrations(
  surface: HookSurface,
): HookRegistration[] {
  return surface.registrations.filter((entry) => !entry.runs);
}
