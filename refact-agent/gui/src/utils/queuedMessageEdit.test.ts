import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  QUEUED_MESSAGE_EDIT_EVENT,
  handOffQueuedMessageEdit,
  mergeQueuedMessageText,
} from "./queuedMessageEdit";
import { getDraftMessage, saveDraftMessage } from "./threadStorage";

describe("queuedMessageEdit", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  it("merges queued text after existing draft text", () => {
    expect(mergeQueuedMessageText("", "queued")).toBe("queued");
    expect(mergeQueuedMessageText("  ", "queued")).toBe("queued");
    expect(mergeQueuedMessageText("draft", "queued")).toBe("draft\n\nqueued");
    expect(mergeQueuedMessageText("draft", " ")).toBe("draft");
  });

  it("persists the edit into the target thread draft without a composer", () => {
    saveDraftMessage("thread-a", "existing");
    saveDraftMessage("thread-b", "other");

    handOffQueuedMessageEdit("thread-a", "queued text");

    expect(getDraftMessage("thread-a")).toBe("existing\n\nqueued text");
    expect(getDraftMessage("thread-b")).toBe("other");
  });

  it("dispatches a thread-scoped event", () => {
    const listener = vi.fn();
    window.addEventListener(QUEUED_MESSAGE_EDIT_EVENT, listener);

    handOffQueuedMessageEdit("thread-a", "queued");

    window.removeEventListener(QUEUED_MESSAGE_EDIT_EVENT, listener);
    expect(listener).toHaveBeenCalledTimes(1);
    const event = listener.mock.calls[0][0] as CustomEvent;
    expect(event.detail).toEqual({ chatId: "thread-a", composers: [] });
  });

  it("ignores empty edits", () => {
    const listener = vi.fn();
    window.addEventListener(QUEUED_MESSAGE_EDIT_EVENT, listener);

    handOffQueuedMessageEdit("thread-a", "   ");
    handOffQueuedMessageEdit("", "queued");

    window.removeEventListener(QUEUED_MESSAGE_EDIT_EVENT, listener);
    expect(listener).not.toHaveBeenCalled();
    expect(getDraftMessage("thread-a")).toBe("");
  });
});
