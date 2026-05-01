import { LspChatMode, ReasoningEffort } from "../features/Chat/Thread/types";
import { SystemPrompts } from "../services/refact/prompts";

const MODE_PARAMS_KEY_PREFIX = "refact_mode_params_";
const DRAFT_MESSAGES_KEY = "refact_draft_messages";
const MAX_DRAFT_MESSAGES = 50;

export interface PersistedModeParams {
  model?: string;
  boost_reasoning?: boolean;
  reasoning_effort?: ReasoningEffort;
  thinking_budget?: number;
  temperature?: number;
  frequency_penalty?: number;
  max_tokens?: number;
  parallel_tool_calls?: boolean;
  increase_max_tokens?: boolean;
  include_project_info?: boolean;
  system_prompt?: SystemPrompts;
  checkpoints_enabled?: boolean;
  follow_ups_enabled?: boolean;
}

export interface PersistedThreadParams extends PersistedModeParams {
  mode?: LspChatMode;
}

type DraftMessagesStorage = Partial<
  Record<
    string,
    {
      content: string;
      timestamp: number;
    }
  >
>;

function getModeKey(mode: LspChatMode): string {
  return `${MODE_PARAMS_KEY_PREFIX}${mode}`;
}

export function saveModeParams(
  mode: LspChatMode,
  params: Partial<PersistedModeParams>,
): void {
  try {
    if (typeof localStorage === "undefined") return;
    const existing = getModeParams(mode);
    const merged = { ...existing, ...params };
    localStorage.setItem(getModeKey(mode), JSON.stringify(merged));
  } catch {
    // Silent fail
  }
}

export function getModeParams(mode: LspChatMode): Partial<PersistedModeParams> {
  try {
    if (typeof localStorage === "undefined") return {};
    const stored = localStorage.getItem(getModeKey(mode));
    if (!stored) return {};
    return JSON.parse(stored) as Partial<PersistedModeParams>;
  } catch {
    return {};
  }
}

export function getLastThreadParams(
  mode?: LspChatMode,
): Partial<PersistedThreadParams> {
  const defaultMode = mode ?? "agent";
  const modeParams = getModeParams(defaultMode);
  return { ...modeParams, mode: defaultMode };
}

export function saveLastThreadParams(
  params: Partial<PersistedThreadParams>,
): void {
  const mode = params.mode ?? "agent";
  const { mode: _, ...modeParams } = params;
  saveModeParams(mode, modeParams);
}

function loadDraftMessagesStorage(): DraftMessagesStorage {
  try {
    if (typeof localStorage === "undefined") return {};
    const stored = localStorage.getItem(DRAFT_MESSAGES_KEY);
    if (!stored) return {};
    return JSON.parse(stored) as DraftMessagesStorage;
  } catch {
    return {};
  }
}

function saveDraftMessagesStorage(storage: DraftMessagesStorage): void {
  try {
    if (typeof localStorage === "undefined") return;
    const entries = Object.entries(storage).filter(
      (entry): entry is [string, { content: string; timestamp: number }] =>
        entry[1] !== undefined,
    );
    if (entries.length > MAX_DRAFT_MESSAGES) {
      const sorted = entries.sort((a, b) => b[1].timestamp - a[1].timestamp);
      const pruned = Object.fromEntries(sorted.slice(0, MAX_DRAFT_MESSAGES));
      localStorage.setItem(DRAFT_MESSAGES_KEY, JSON.stringify(pruned));
    } else {
      localStorage.setItem(DRAFT_MESSAGES_KEY, JSON.stringify(storage));
    }
  } catch {
    // Silent fail
  }
}

export function saveDraftMessage(threadId: string, content: string): void {
  try {
    if (!threadId) return;
    const storage = loadDraftMessagesStorage();
    if (!content.trim()) {
      const { [threadId]: _, ...rest } = storage;
      saveDraftMessagesStorage(rest);
    } else {
      storage[threadId] = { content, timestamp: Date.now() };
      saveDraftMessagesStorage(storage);
    }
  } catch {
    // Silent fail
  }
}

export function getDraftMessage(threadId: string): string {
  try {
    if (!threadId) return "";
    const storage = loadDraftMessagesStorage();
    return storage[threadId]?.content ?? "";
  } catch {
    return "";
  }
}

export function clearDraftMessage(threadId: string): void {
  try {
    if (!threadId) return;
    const storage = loadDraftMessagesStorage();
    const { [threadId]: _, ...rest } = storage;
    saveDraftMessagesStorage(rest);
  } catch {
    // Silent fail
  }
}

export function clearAllDraftMessages(): void {
  try {
    if (typeof localStorage === "undefined") return;
    localStorage.removeItem(DRAFT_MESSAGES_KEY);
  } catch {
    // Silent fail
  }
}

export function pruneStaleDraftMessages(): void {
  try {
    const storage = loadDraftMessagesStorage();
    const sevenDaysAgo = Date.now() - 7 * 24 * 60 * 60 * 1000;
    const pruned: DraftMessagesStorage = {};
    let didPrune = false;
    for (const [threadId, draft] of Object.entries(storage)) {
      if (!draft) {
        didPrune = true;
        continue;
      }
      if (draft.timestamp > sevenDaysAgo) {
        pruned[threadId] = draft;
      } else {
        didPrune = true;
      }
    }
    if (didPrune) {
      saveDraftMessagesStorage(pruned);
    }
  } catch {
    // Silent fail
  }
}
