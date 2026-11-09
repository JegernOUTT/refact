import type { RootState } from "../../app/store";
import type { StagedAppContext } from "../../features/Apps/appsSlice";

export const APP_CONTEXT_MAX_PER_MESSAGE = 8;

const MAX_SOURCE_REFS = 8;
const MAX_REF_CHARS = 256;
const MAX_LABEL_CHARS = 120;
const MAX_FIELD_CHARS = 128;
const EMPTY_STAGED: readonly StagedAppContext[] = [];
const TICKET_ID_PATTERN =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

export type FrozenAppContext = {
  id: string;
  app_id: string;
  binding_id: string;
  view_id?: string;
  revision: string;
  label: string;
  text: string;
  text_length: number;
  captured_at?: string;
  source_refs: string[];
};

export type DeferredReason = "limit" | "invalid_ticket";

export type DeferredAppContext = {
  context: FrozenAppContext;
  reason: DeferredReason;
};

export type AppContextPlan = {
  included: FrozenAppContext[];
  deferred: DeferredAppContext[];
};

export function selectStagedAppContextsById(
  state: Pick<RootState, "apps">,
  chatId: string,
): readonly StagedAppContext[] {
  return state.apps.stagedByChat[chatId] ?? EMPTY_STAGED;
}

function singleLine(value: unknown, max: number): string {
  if (typeof value !== "string") return "";
  let out = "";
  for (const char of value) {
    const code = char.codePointAt(0) ?? 0;
    out += code < 32 || code === 127 ? " " : char;
  }
  return out.replace(/\s+/g, " ").trim().slice(0, max);
}

function isValidStaged(context: StagedAppContext): boolean {
  return (
    typeof context.id === "string" &&
    context.id.length > 0 &&
    typeof context.app_id === "string" &&
    typeof context.binding_id === "string" &&
    typeof context.revision === "string" &&
    typeof context.text === "string"
  );
}

function readSourceRefs(context: StagedAppContext): string[] {
  const refs: string[] = [];
  for (const item of context.source_refs ?? []) {
    const ref = singleLine(item.path, MAX_REF_CHARS);
    if (ref && !refs.includes(ref) && refs.length < MAX_SOURCE_REFS) {
      refs.push(ref);
    }
  }
  return refs;
}

function optionalField(value: string | undefined): string | undefined {
  const field = singleLine(value, MAX_FIELD_CHARS);
  return field || undefined;
}

function freezeOne(context: StagedAppContext): FrozenAppContext {
  return {
    id: context.id,
    app_id: singleLine(context.app_id, MAX_FIELD_CHARS),
    binding_id: singleLine(context.binding_id, MAX_FIELD_CHARS),
    view_id: optionalField(context.view_id),
    revision: singleLine(context.revision, MAX_FIELD_CHARS),
    label:
      singleLine(context.title, MAX_LABEL_CHARS) ||
      singleLine(context.app_id, MAX_LABEL_CHARS),
    text: context.text,
    text_length: context.text.length,
    captured_at: optionalField(context.captured_at),
    source_refs: readSourceRefs(context),
  };
}

export function isTicketId(id: string): boolean {
  return TICKET_ID_PATTERN.test(id);
}

export function appContextKey(
  chatId: string,
  id: string,
  revision: string,
): string {
  return `${chatId}\u0000${id}\u0000${revision}`;
}

export function planAppContexts(
  staged: readonly StagedAppContext[],
  skipKeys?: ReadonlySet<string>,
  chatId = "",
): AppContextPlan {
  const included: FrozenAppContext[] = [];
  const deferred: DeferredAppContext[] = [];
  for (const context of staged) {
    if (!isValidStaged(context)) continue;
    if (skipKeys?.has(appContextKey(chatId, context.id, context.revision))) {
      continue;
    }
    const frozen = freezeOne(context);
    if (!isTicketId(context.id)) {
      deferred.push({ context: frozen, reason: "invalid_ticket" });
    } else if (included.length >= APP_CONTEXT_MAX_PER_MESSAGE) {
      deferred.push({ context: frozen, reason: "limit" });
    } else {
      included.push(frozen);
    }
  }
  return { included, deferred };
}

export function sentAppContextIds(
  current: readonly StagedAppContext[],
  sent: readonly FrozenAppContext[],
): string[] {
  return sent
    .filter((frozen) =>
      current.some(
        (context) =>
          context.id === frozen.id &&
          context.revision === frozen.revision &&
          context.text.length === frozen.text_length,
      ),
    )
    .map((frozen) => frozen.id);
}

const armedSubmissions = new Map<string, FrozenAppContext[]>();
const inFlightCounts = new Map<string, number>();

export function runWithArmedAppContexts<T>(
  chatId: string,
  contexts: FrozenAppContext[],
  run: () => T,
): T {
  if (contexts.length === 0) return run();
  armedSubmissions.set(chatId, contexts);
  try {
    return run();
  } finally {
    armedSubmissions.delete(chatId);
  }
}

export function takeAppContextSubmission(chatId: string): FrozenAppContext[] {
  const contexts = armedSubmissions.get(chatId) ?? [];
  armedSubmissions.delete(chatId);
  return contexts;
}

export function markAppContextsInFlight(
  chatId: string,
  contexts: readonly FrozenAppContext[],
): () => void {
  const keys = contexts.map((context) =>
    appContextKey(chatId, context.id, context.revision),
  );
  for (const key of keys) {
    inFlightCounts.set(key, (inFlightCounts.get(key) ?? 0) + 1);
  }
  return () => {
    for (const key of keys) {
      const next = (inFlightCounts.get(key) ?? 1) - 1;
      if (next <= 0) inFlightCounts.delete(key);
      else inFlightCounts.set(key, next);
    }
  };
}

export function inFlightAppContextKeys(): ReadonlySet<string> {
  return new Set(inFlightCounts.keys());
}
