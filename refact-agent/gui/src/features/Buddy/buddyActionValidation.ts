import type { BuddyAction } from "./types";

const PLACEHOLDER_MODEL_IDS = new Set(["your-provider/model-name"]);

function isPlainRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function hasOwnEntries(value: Record<string, unknown>): boolean {
  return Object.keys(value).length > 0;
}

function includesPlaceholderModelId(value: unknown): boolean {
  if (typeof value === "string") {
    return PLACEHOLDER_MODEL_IDS.has(value.trim());
  }
  if (Array.isArray(value)) return value.some(includesPlaceholderModelId);
  if (!isPlainRecord(value)) return false;
  return Object.values(value).some(includesPlaceholderModelId);
}

function validatePatch(patch: unknown): string | null {
  if (!isPlainRecord(patch) || !hasOwnEntries(patch)) {
    return "Buddy refused an empty draft change. No settings were changed.";
  }
  if (includesPlaceholderModelId(patch)) {
    return "Buddy refused a placeholder model id. Choose a concrete provider/model first.";
  }
  return null;
}

export function validateBuddyDraftAction(action: BuddyAction): string | null {
  switch (action.kind) {
    case "draft_agents_md_patch":
      return action.content.trim()
        ? null
        : "Buddy refused an empty AGENTS.md draft. No file was changed.";
    case "draft_defaults_change":
    case "draft_customization_change":
      return validatePatch(action.patch);
    default:
      return null;
  }
}
