import React from "react";
import { renderHook, waitFor } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { Provider } from "react-redux";
import { afterEach, describe, expect, it } from "vitest";

import { setUpStore } from "../app/store";
import type { Config } from "../features/Config/configSlice";
import { useChatDeepLink } from "../hooks/useChatDeepLink";
import { server } from "../utils/mockServer";

const CHAT_ID = "chat-42";

const trajectory = {
  id: CHAT_ID,
  title: "Deep linked chat",
  created_at: "2026-07-18T00:00:00Z",
  updated_at: "2026-07-18T01:00:00Z",
  model: "claude",
  mode: "AGENT",
  tool_use: "agent",
  messages: [],
};

function makeConfig(overrides: Partial<Config> = {}): Config {
  return {
    host: "web",
    engineServed: true,
    lspPort: 8001,
    themeProps: {},
    ...overrides,
  };
}

function renderDeepLink(config: Config, ready = true) {
  const store = setUpStore({ config });
  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <Provider store={store}>{children}</Provider>
  );
  renderHook(() => useChatDeepLink(ready), { wrapper });
  return { store };
}

describe("useChatDeepLink", () => {
  afterEach(() => {
    window.history.replaceState(null, "", "/");
  });

  it("opens the linked chat and strips the param for engine-served web", async () => {
    let requests = 0;
    server.use(
      http.get(`*/v1/trajectories/${CHAT_ID}`, () => {
        requests += 1;
        return HttpResponse.json(trajectory);
      }),
    );
    window.history.replaceState(null, "", `/?chat=${CHAT_ID}&keep=1`);

    const { store } = renderDeepLink(makeConfig());

    await waitFor(() => {
      expect(store.getState().chat.threads[CHAT_ID]).toBeDefined();
    });
    expect(requests).toBe(1);
    expect(store.getState().chat.current_thread_id).toBe(CHAT_ID);
    expect(store.getState().chat.threads[CHAT_ID]?.thread.title).toBe(
      "Deep linked chat",
    );
    expect(store.getState().pages.some((page) => page.name === "chat")).toBe(
      true,
    );
    expect(window.location.search).toBe("?keep=1");
  });

  it("ignores the param on IDE hosts", async () => {
    let requests = 0;
    server.use(
      http.get(`*/v1/trajectories/${CHAT_ID}`, () => {
        requests += 1;
        return HttpResponse.json(trajectory);
      }),
    );
    window.history.replaceState(null, "", `/?chat=${CHAT_ID}`);

    const { store } = renderDeepLink(makeConfig({ host: "jetbrains" }));

    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(requests).toBe(0);
    expect(store.getState().chat.threads[CHAT_ID]).toBeUndefined();
    expect(window.location.search).toBe(`?chat=${CHAT_ID}`);
  });

  it("falls back to normal startup when the chat is unknown", async () => {
    server.use(
      http.get(`*/v1/trajectories/${CHAT_ID}`, () => {
        return HttpResponse.json({ detail: "not found" }, { status: 404 });
      }),
    );
    window.history.replaceState(null, "", `/?chat=${CHAT_ID}`);

    const { store } = renderDeepLink(makeConfig());

    await waitFor(() => {
      expect(window.location.search).toBe("");
    });
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(store.getState().chat.threads[CHAT_ID]).toBeUndefined();
    expect(store.getState().pages.some((page) => page.name === "chat")).toBe(
      false,
    );
  });

  it("waits for readiness before consuming the param", async () => {
    server.use(
      http.get(`*/v1/trajectories/${CHAT_ID}`, () => {
        return HttpResponse.json(trajectory);
      }),
    );
    window.history.replaceState(null, "", `/?chat=${CHAT_ID}`);

    const { store } = renderDeepLink(makeConfig(), false);

    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(window.location.search).toBe(`?chat=${CHAT_ID}`);
    expect(store.getState().chat.threads[CHAT_ID]).toBeUndefined();
  });
});
