import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act } from "react-dom/test-utils";
import { createDefaultChatState, render } from "../utils/test-utils";
import { ChatThreadProvider } from "../features/Chat/Thread";
import { useDraftMessage } from "./useDraftMessage";
import { handOffQueuedMessageEdit } from "../utils/queuedMessageEdit";
import { getDraftMessage, saveDraftMessage } from "../utils/threadStorage";

let latestSetValue: ((value: string) => void) | null = null;

function DraftProbe({ label }: { label: string }) {
  const { value, setValue } = useDraftMessage();
  if (label === "current") latestSetValue = setValue;
  return <div data-testid={label}>{value}</div>;
}

function renderPanes(showCurrent = true) {
  const chat = createDefaultChatState();
  const currentId = chat.current_thread_id;
  const currentRuntime = chat.threads[currentId];
  chat.threads["thread-b"] = {
    ...currentRuntime,
    thread: { ...currentRuntime.thread, id: "thread-b" },
  };
  chat.open_thread_ids = [currentId, "thread-b"];
  const ui = (visible: boolean) => (
    <>
      {visible && (
        <ChatThreadProvider chatId={currentId}>
          <DraftProbe label="current" />
        </ChatThreadProvider>
      )}
      <ChatThreadProvider chatId="thread-b">
        <DraftProbe label="other" />
      </ChatThreadProvider>
    </>
  );
  const result = render(ui(showCurrent), { preloadedState: { chat } });
  return {
    currentId,
    show: (visible: boolean) => result.rerender(ui(visible)),
  };
}

function text(label: string) {
  return document.querySelector(`[data-testid="${label}"]`)?.textContent;
}

describe("useDraftMessage queued edit handoff", () => {
  beforeEach(() => {
    localStorage.clear();
    latestSetValue = null;
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("restores into an unmounted thread composer when it mounts", () => {
    const { currentId, show } = renderPanes(false);

    act(() => handOffQueuedMessageEdit(currentId, "queued text"));

    expect(text("other")).toBe("");
    show(true);
    expect(text("current")).toBe("queued text");
    expect(getDraftMessage("thread-b")).toBe("");
  });

  it("appends to the live draft and ignores stale pending saves", () => {
    vi.useFakeTimers();
    const { currentId } = renderPanes();

    act(() => latestSetValue?.("typed"));
    act(() => handOffQueuedMessageEdit(currentId, "queued"));
    act(() => {
      vi.advanceTimersByTime(1000);
    });

    expect(text("current")).toBe("typed\n\nqueued");
    expect(getDraftMessage(currentId)).toBe("typed\n\nqueued");
    expect(text("other")).toBe("");
  });

  it("keeps queued text in every same-thread composer", () => {
    const chat = createDefaultChatState();
    const chatId = chat.current_thread_id;
    saveDraftMessage(chatId, "saved");
    render(
      <>
        <ChatThreadProvider chatId={chatId}>
          <DraftProbe label="first" />
        </ChatThreadProvider>
        <ChatThreadProvider chatId={chatId}>
          <DraftProbe label="second" />
        </ChatThreadProvider>
      </>,
      { preloadedState: { chat } },
    );

    act(() => handOffQueuedMessageEdit(chatId, "queued"));

    expect(text("first")).toBe("saved\n\nqueued");
    expect(text("second")).toBe("saved\n\nqueued");
    expect(getDraftMessage(chatId)).toBe("saved\n\nqueued");
  });

  it("preserves divergent live drafts and synchronizes same-thread composers", () => {
    vi.useFakeTimers();
    const chat = createDefaultChatState();
    const chatId = chat.current_thread_id;
    render(
      <>
        <ChatThreadProvider chatId={chatId}>
          <DraftProbe label="current" />
        </ChatThreadProvider>
        <ChatThreadProvider chatId={chatId}>
          <DraftProbe label="second" />
        </ChatThreadProvider>
      </>,
      { preloadedState: { chat } },
    );
    act(() => latestSetValue?.("typed"));
    act(() => handOffQueuedMessageEdit(chatId, "queued"));
    act(() => {
      vi.advanceTimersByTime(1000);
    });
    expect(text("current")).toBe("typed\n\nqueued");
    expect(text("second")).toBe("typed\n\nqueued");
    expect(getDraftMessage(chatId)).toBe("typed\n\nqueued");
  });

  it("preserves an existing saved draft of the remounted composer", () => {
    const { currentId, show } = renderPanes();
    show(false);
    saveDraftMessage(currentId, "saved");

    act(() => handOffQueuedMessageEdit(currentId, "queued"));
    show(true);

    expect(text("current")).toBe("saved\n\nqueued");
  });
});
