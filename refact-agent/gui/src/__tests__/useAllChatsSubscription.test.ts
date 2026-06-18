import { createElement, type ReactElement, type ReactNode } from "react";
import { Provider } from "react-redux";
import { renderHook, act as rtlAct } from "@testing-library/react";
import { afterEach, describe, it, expect, vi } from "vitest";
import {
  pickDesiredChatSubscriptions,
  useAllChatsSubscription,
} from "../hooks/useAllChatsSubscription";
import {
  connectionSlice,
  registerVisibleChatMount,
  unregisterVisibleChatMount,
  selectVisibleChatMountIds,
} from "../features/Connection";
import { setUpStore, type AppStore, type RootState } from "../app/store";

const ReduxProvider = Provider as unknown as (props: {
  store: AppStore;
  children?: ReactNode;
}) => ReactElement | null;

const syncAct = rtlAct as unknown as (callback: () => void) => void;
const asyncAct = rtlAct as unknown as (
  callback: () => Promise<void>,
) => Promise<void>;

function renderAllChatsSubscription(store: AppStore) {
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(ReduxProvider, { store }, children);

  return renderHook(() => useAllChatsSubscription(), { wrapper });
}

function createSubscriptionStore() {
  return setUpStore({
    config: {
      apiKey: "test",
      host: "vscode",
      lspPort: 8001,
      themeProps: {},
    },
  });
}

afterEach(() => {
  vi.useRealTimers();
  vi.clearAllMocks();
  vi.unstubAllGlobals();
});

describe("pickDesiredChatSubscriptions", () => {
  it("subscribes only chats visible on screen", () => {
    const result = pickDesiredChatSubscriptions({
      visibleThreadIds: ["chat-2", "chat-5", "chat-7"],
    });

    expect(result).toEqual(["chat-2", "chat-5", "chat-7"]);
  });

  it("deduplicates visible chats while preserving order", () => {
    const result = pickDesiredChatSubscriptions({
      visibleThreadIds: ["chat-2", "chat-5", "chat-2", "chat-7"],
    });

    expect(result).toEqual(["chat-2", "chat-5", "chat-7"]);
  });

  it("does not keep chat subscriptions while the page is hidden", () => {
    const result = pickDesiredChatSubscriptions({
      visibleThreadIds: ["chat-1", "chat-2"],
      documentVisible: false,
    });

    expect(result).toEqual([]);
  });

  it("derives desired subscriptions from registered visible chat mounts", () => {
    let connection = connectionSlice.reducer(
      undefined,
      registerVisibleChatMount({ chatId: "chat-a" }),
    );
    connection = connectionSlice.reducer(
      connection,
      registerVisibleChatMount({ chatId: "chat-b" }),
    );
    connection = connectionSlice.reducer(
      connection,
      registerVisibleChatMount({ chatId: "chat-b" }),
    );
    connection = connectionSlice.reducer(
      connection,
      unregisterVisibleChatMount({ chatId: "chat-a" }),
    );

    const visibleThreadIds = selectVisibleChatMountIds({
      connection,
    } as unknown as RootState);
    const result = pickDesiredChatSubscriptions({ visibleThreadIds });

    expect(result).toEqual(["chat-b"]);
  });

  it("clears a pending retry timer when a chat leaves the visible desired set", async () => {
    vi.useFakeTimers();
    const mockFetch = vi.fn().mockRejectedValue(new Error("network drop"));
    vi.stubGlobal("fetch", mockFetch);
    const store = createSubscriptionStore();

    syncAct(() => {
      store.dispatch(registerVisibleChatMount({ chatId: "chat-a" }));
    });
    const { unmount } = renderAllChatsSubscription(store);

    expect(mockFetch).toHaveBeenCalledTimes(1);

    await asyncAct(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(vi.getTimerCount()).toBe(1);

    syncAct(() => {
      store.dispatch(unregisterVisibleChatMount({ chatId: "chat-a" }));
    });

    expect(vi.getTimerCount()).toBe(0);

    syncAct(() => {
      vi.advanceTimersByTime(60_000);
    });

    expect(mockFetch).toHaveBeenCalledTimes(1);
    unmount();
  });
});
