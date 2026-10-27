import { getDraftMessage, saveDraftMessage } from "./threadStorage";

export const QUEUED_MESSAGE_EDIT_EVENT = "refact-queued-message-edit";

export type QueuedMessageEditDetail = {
  chatId: string;
  composers: { value: string; apply: (value: string) => void }[];
};

export function mergeQueuedMessageText(existing: string, value: string) {
  if (!existing.trim()) return value;
  if (!value.trim()) return existing;
  return `${existing}\n\n${value}`;
}

export function handOffQueuedMessageEdit(chatId: string, value: string): void {
  if (!chatId || !value.trim()) return;
  const detail: QueuedMessageEditDetail = { chatId, composers: [] };
  window.dispatchEvent(new CustomEvent(QUEUED_MESSAGE_EDIT_EVENT, { detail }));
  const drafts = detail.composers.length
    ? detail.composers.map((composer) => composer.value)
    : [getDraftMessage(chatId)];
  const existing = [...new Set(drafts)]
    .filter((draft) => draft.trim())
    .join("\n\n");
  const next = mergeQueuedMessageText(existing, value);
  saveDraftMessage(chatId, next);
  detail.composers.forEach((composer) => composer.apply(next));
}
