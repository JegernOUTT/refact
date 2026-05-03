export enum EVENT_NAMES_FROM_SETUP {
  OPEN_EXTERNAL_URL = "open_external_url",
}

export interface ActionFromSetup {
  type: EVENT_NAMES_FROM_SETUP;
  payload?: Record<string, unknown>;
}

const SETUP_EVENT_NAMES = new Set<string>([
  EVENT_NAMES_FROM_SETUP.OPEN_EXTERNAL_URL,
]);

export function isActionFromSetup(action: unknown): action is ActionFromSetup {
  if (!action) return false;
  if (typeof action !== "object") return false;
  if (!("type" in action)) return false;
  if (typeof action.type !== "string") return false;
  return SETUP_EVENT_NAMES.has(action.type);
}

export interface OpenExternalUrl extends ActionFromSetup {
  type: EVENT_NAMES_FROM_SETUP.OPEN_EXTERNAL_URL;
  payload: { url: string };
}

export function isOpenExternalUrl(action: unknown): action is OpenExternalUrl {
  if (!action) return false;
  if (typeof action !== "object") return false;
  if (!("type" in action)) return false;
  if (action.type !== EVENT_NAMES_FROM_SETUP.OPEN_EXTERNAL_URL) return false;
  if (!("payload" in action)) return false;
  if (!action.payload) return false;
  if (typeof action.payload !== "object") return false;
  if (!("url" in action.payload)) return false;
  return typeof action.payload.url === "string";
}
