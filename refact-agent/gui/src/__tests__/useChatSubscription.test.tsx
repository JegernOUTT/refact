import { describe, it, expect, vi, afterEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { Provider } from "react-redux";
import { configureStore } from "@reduxjs/toolkit";
import { useChatSubscription } from "../hooks/useChatSubscription";
import { chatReducer } from "../features/Chat/Thread/reducer";
import {
  reducer as configReducer,
  updateConfig,
} from "../features/Config/configSlice";

const createTestStore = () => {
  return configureStore({
    reducer: {
      chat: chatReducer,
      config: configReducer,
    },
  });
};

const mockFetch = vi.fn();

const wrapper = ({ children }: { children: React.ReactNode }) => (
  <Provider store={createTestStore()}>{children}</Provider>
);

describe("useChatSubscription", () => {
  afterEach(() => {
    mockFetch.mockReset();
    vi.restoreAllMocks();
    vi.useRealTimers();
  });

  it("should return disconnected status when disabled", () => {
    const { result } = renderHook(
      () => useChatSubscription("test-chat", { enabled: false }),
      { wrapper },
    );

    expect(result.current.status).toBe("disconnected");
    expect(result.current.isConnected).toBe(false);
    expect(result.current.isConnecting).toBe(false);
  });

  it("should return disconnected status when chatId is null", () => {
    const { result } = renderHook(
      () => useChatSubscription(null, { enabled: true }),
      { wrapper },
    );

    expect(result.current.status).toBe("disconnected");
  });

  it("should return disconnected status when chatId is undefined", () => {
    const { result } = renderHook(
      () => useChatSubscription(undefined, { enabled: true }),
      { wrapper },
    );

    expect(result.current.status).toBe("disconnected");
  });

  it("should have connect and disconnect functions", () => {
    const { result } = renderHook(
      () => useChatSubscription("test-chat", { enabled: false }),
      { wrapper },
    );

    expect(typeof result.current.connect).toBe("function");
    expect(typeof result.current.disconnect).toBe("function");
  });

  it("should have lastSeq as string", () => {
    const { result } = renderHook(
      () => useChatSubscription("test-chat", { enabled: false }),
      { wrapper },
    );

    expect(typeof result.current.lastSeq).toBe("string");
    expect(result.current.lastSeq).toBe("0");
  });

  it("should have null error initially", () => {
    const { result } = renderHook(
      () => useChatSubscription("test-chat", { enabled: false }),
      { wrapper },
    );

    expect(result.current.error).toBeNull();
  });

  it("connects to a remote web lspUrl when lspPort is zero", async () => {
    global.fetch = mockFetch as unknown as typeof fetch;
    mockFetch.mockResolvedValue({
      ok: true,
      body: {
        getReader: () => ({
          read: vi.fn().mockResolvedValue({ done: true }),
        }),
      },
    });
    const store = createTestStore();
    store.dispatch(
      updateConfig({
        host: "web",
        lspPort: 0,
        lspUrl: "https://remote.example.com/v1/ping",
      }),
    );
    const remoteWrapper = ({ children }: { children: React.ReactNode }) => (
      <Provider store={store}>{children}</Provider>
    );

    renderHook(() => useChatSubscription("remote-chat"), {
      wrapper: remoteWrapper,
    });

    await waitFor(() => {
      expect(mockFetch).toHaveBeenCalledWith(
        "https://remote.example.com/v1/chats/subscribe?chat_id=remote-chat",
        expect.objectContaining({ method: "GET" }),
      );
    });
  });
});
