import { ToolUse, LspChatMode } from "../features/Chat/Thread/types";
import { SystemPrompts } from "../services/refact/prompts";

const LAST_THREAD_PARAMS_KEY = "refact_last_thread_params";
const DRAFT_MESSAGES_KEY = "refact_draft_messages";
const MAX_DRAFT_MESSAGES = 50;

export interface PersistedThreadParams {
  model: string;
  tool_use: ToolUse;
  mode?: LspChatMode;
  boost_reasoning?: boolean;
  increase_max_tokens?: boolean;
  include_project_info?: boolean;
  context_tokens_cap?: number;
  system_prompt?: SystemPrompts;
  checkpoints_enabled?: boolean;
  follow_ups_enabled?: boolean;
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

export function saveLastThreadParams(
  params: Partial<PersistedThreadParams>,
): void {
  try {
    if (typeof localStorage === "undefined") return;
    const existing = getLastThreadParams();
    const merged = { ...existing, ...params };
    localStorage.setItem(LAST_THREAD_PARAMS_KEY, JSON.stringify(merged));
  } catch {
    // Silent fail - localStorage might be unavailable
  }
}

export function getLastThreadParams(): Partial<PersistedThreadParams> {
  try {
    if (typeof localStorage === "undefined") return {};
    const stored = localStorage.getItem(LAST_THREAD_PARAMS_KEY);
    if (!stored) return {};
    return JSON.parse(stored) as Partial<PersistedThreadParams>;
  } catch {
    return {};
  }
}

export function clearLastThreadParams(): void {
  try {
    if (typeof localStorage === "undefined") return;
    localStorage.removeItem(LAST_THREAD_PARAMS_KEY);
  } catch {
    // Silent fail
  }
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
