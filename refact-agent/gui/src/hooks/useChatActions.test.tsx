import { Provider } from "react-redux";
import { renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import type { AppStore, RootState } from "../app/store";
import type { ChatThreadRuntime } from "../features/Chat/Thread/types";

type UseChatActions = typeof import("./useChatActions").useChatActions;
type SetUpStore = typeof import("../app/store").setUpStore;
type CreateDefaultChatState =
  typeof import("../utils/test-utils").createDefaultChatState;

const commandMocks = {
  sendChatCommand: vi.fn(() => Promise.resolve()),
  sendUserMessage: vi.fn(() => Promise.resolve()),
  retryFromIndex: vi.fn(() => Promise.resolve()),
  regenerate: vi.fn(() => Promise.resolve()),
  updateChatParams: vi.fn(() => Promise.resolve()),
  abortGeneration: vi.fn(() => Promise.resolve()),
  respondToToolConfirmation: vi.fn(() => Promise.resolve()),
  respondToToolConfirmations: vi.fn(() => Promise.resolve()),
  updateMessage: vi.fn(() => Promise.resolve()),
  removeMessage: vi.fn(() => Promise.resolve()),
  cancelQueuedItem: vi.fn(() => Promise.resolve(true)),
};

let useChatActions: UseChatActions;
let setUpStore: SetUpStore;
let createDefaultChatState: CreateDefaultChatState;

beforeEach(async () => {
  vi.resetModules();
  Object.values(commandMocks).forEach((mock) => mock.mockClear());
  vi.doMock("../services/refact/chatCommands", () => commandMocks);
  ({ useChatActions } = await import("./useChatActions"));
  ({ setUpStore } = await import("../app/store"));
  ({ createDefaultChatState } = await import("../utils/test-utils"));
});

afterEach(() => {
  vi.doUnmock("../services/refact/chatCommands");
});

function makeRuntime(id: string): ChatThreadRuntime {
  const chat = createDefaultChatState();
  const runtime = Object.values(chat.threads)[0];
  runtime.thread.id = id;
  runtime.thread.model = "model-a";
  runtime.thread.mode = "agent";
  return runtime;
}

function makeStore(): AppStore {
  const threadA = makeRuntime("thread-A");
  const threadB = makeRuntime("thread-B");
  threadA.send_immediately = true;
  threadA.attached_images = [
    { name: "a.png", type: "image/png", content: "a" },
  ];
  threadB.send_immediately = true;
  threadB.attached_images = [
    { name: "b.png", type: "image/png", content: "b" },
  ];
  threadB.manual_preview_items = [
    {
      kind: "file",
      label: "preview",
      context_file: {
        file_name: "file.ts",
        file_content: "content",
        line1: 1,
        line2: 1,
        usefulness: 1,
      },
    },
  ];
  threadB.manual_preview_ran = true;

  return setUpStore({
    chat: {
      ...createDefaultChatState(),
      current_thread_id: "thread-A",
      open_thread_ids: ["thread-A", "thread-B"],
      threads: {
        "thread-A": threadA,
        "thread-B": threadB,
      },
    },
  } as unknown as Partial<RootState>);
}

function renderUseChatActions(store: AppStore) {
  return renderHook<ReturnType<UseChatActions>, unknown>(
    () => useChatActions("thread-B"),
    {
      wrapper: ({ children }) => <Provider store={store}>{children}</Provider>,
    },
  );
}

describe("useChatActions", () => {
  test("submit targets the explicit thread and cleans only that thread", async () => {
    const store = makeStore();
    const { result } = renderUseChatActions(store);

    await result.current.submit("hello");

    expect(commandMocks.updateChatParams).toHaveBeenCalledWith(
      "thread-B",
      expect.any(Object),
      expect.any(Object),
      undefined,
    );
    expect(commandMocks.sendUserMessage).toHaveBeenCalledWith(
      "thread-B",
      expect.any(Array),
      expect.any(Object),
      undefined,
      true,
      [
        {
          file_name: "file.ts",
          file_content: "content",
          line1: 1,
          line2: 1,
          usefulness: 1,
        },
      ],
      true,
    );

    const state = store.getState();
    expect(state.chat.threads["thread-A"]?.send_immediately).toBe(true);
    expect(state.chat.threads["thread-A"]?.attached_images).toHaveLength(1);
    expect(state.chat.threads["thread-B"]?.send_immediately).toBe(false);
    expect(state.chat.threads["thread-B"]?.attached_images).toHaveLength(0);
    expect(state.chat.threads["thread-B"]?.manual_preview_items).toHaveLength(
      0,
    );
  });

  test("commands target the explicit thread", async () => {
    const store = makeStore();
    const { result } = renderUseChatActions(store);

    await result.current.abort();
    await result.current.retryFromIndex(1, "retry");
    await result.current.regenerate();
    await result.current.cancelQueued("queued-1");

    expect(commandMocks.abortGeneration).toHaveBeenCalledWith(
      "thread-B",
      expect.any(Object),
      undefined,
    );
    expect(commandMocks.retryFromIndex).toHaveBeenCalledWith(
      "thread-B",
      1,
      "retry",
      expect.any(Object),
      undefined,
    );
    expect(commandMocks.regenerate).toHaveBeenCalledWith(
      "thread-B",
      expect.any(Object),
      undefined,
    );
    expect(commandMocks.cancelQueuedItem).toHaveBeenCalledWith(
      "thread-B",
      "queued-1",
      expect.any(Object),
      undefined,
    );
  });
});
