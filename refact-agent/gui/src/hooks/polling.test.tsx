import React from "react";
import { Provider } from "react-redux";
import { renderHook, waitFor } from "@testing-library/react";
import { act } from "react-dom/test-utils";
import { setupListeners } from "@reduxjs/toolkit/query";
import { afterEach, describe, expect, it, vi } from "vitest";
import { http, HttpResponse } from "msw";
import { setUpStore } from "../app/store";
import { capsApi } from "../services/refact/caps";
import { modelsApi } from "../services/refact/models";
import { pingApi } from "../services/refact/ping";
import { organizationApi } from "../services/refact/organization";
import { setApiKey, updateConfig } from "../features/Config/configSlice";
import { setBackendStatus } from "../features/Connection";
import { useOrganizationData } from "../features/Dashboard/hooks/useOrganization";
import { STUB_CAPS_RESPONSE } from "../__fixtures__/caps";
import { server } from "../utils/mockServer";
import { useGetCapsQuery } from "./useGetCapsQuery";
import { useGetPing } from "./useGetPing";

vi.mock("../features/Dashboard/streamSelectors", () => {
  const items = [{ kind: "chat", id: "fixture-chat" }];
  return { selectStreamItems: () => items };
});

const FAKE_TIMERS = {
  toFake: [
    "setTimeout",
    "clearTimeout",
    "setInterval",
    "clearInterval",
    "Date",
  ] as const,
};
const useFakeTimers = () =>
  vi.useFakeTimers({ toFake: [...FAKE_TIMERS.toFake] });
const advance = (ms: number) =>
  act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });

const originalVisibility = Object.getOwnPropertyDescriptor(
  document,
  "visibilityState",
);
let visibility: DocumentVisibilityState = "visible";
function setVisibility(next: DocumentVisibilityState) {
  visibility = next;
  Object.defineProperty(document, "visibilityState", {
    configurable: true,
    get: () => visibility,
  });
  act(() => {
    document.dispatchEvent(new Event("visibilitychange"));
  });
}

const stores: ReturnType<typeof setUpStore>[] = [];
const cleanups: (() => void)[] = [];
function setup() {
  const store = setUpStore({
    config: { apiKey: "test", lspPort: 8001, host: "vscode", themeProps: {} },
  });
  stores.push(store);
  store.dispatch(setBackendStatus({ status: "online" }));
  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <Provider store={store}>{children}</Provider>
  );
  return { store, wrapper };
}
function withRtkWindowListeners(store: ReturnType<typeof setUpStore>) {
  cleanups.push(setupListeners(store.dispatch));
}
afterEach(() => {
  for (const cleanup of cleanups.splice(0)) cleanup();
  for (const store of stores.splice(0)) {
    store.dispatch(capsApi.util.resetApiState());
    store.dispatch(pingApi.util.resetApiState());
    store.dispatch(organizationApi.util.resetApiState());
  }
  vi.useRealTimers();
  visibility = "visible";
  if (originalVisibility)
    Object.defineProperty(document, "visibilityState", originalVisibility);
  else Reflect.deleteProperty(document, "visibilityState");
});

function countCaps() {
  const counter = { caps: 0 };
  server.use(
    http.get("*/v1/caps", () => {
      counter.caps++;
      return HttpResponse.json(STUB_CAPS_RESPONSE);
    }),
  );
  return counter;
}

describe("caps cadence", () => {
  it("fetches caps once across mounted consumers and again after invalidation", async () => {
    const counter = countCaps();
    server.use(http.get("*/v1/ping", () => HttpResponse.text("pong")));
    const { store, wrapper } = setup();
    const first = renderHook(() => useGetCapsQuery(), { wrapper });
    await waitFor(() => expect(first.result.current.isSuccess).toBe(true));
    const second = renderHook(() => useGetCapsQuery(), { wrapper });
    await waitFor(() => expect(second.result.current.isSuccess).toBe(true));
    expect(counter.caps).toBe(1);
    act(() => {
      store.dispatch(capsApi.util.invalidateTags(["CAPS"]));
    });
    await waitFor(() => expect(counter.caps).toBe(2));
    first.unmount();
    second.unmount();
    const third = renderHook(() => useGetCapsQuery(), { wrapper });
    expect(third.result.current.isSuccess).toBe(true);
    expect(counter.caps).toBe(2);
    third.unmount();
  });

  it("refetches caps after updateModel and after deleteModel", async () => {
    const counter = countCaps();
    server.use(
      http.get("*/v1/ping", () => HttpResponse.text("pong")),
      http.post("*/v1/model", () => HttpResponse.json({ success: true })),
      http.delete("*/v1/model", () => HttpResponse.json({ success: true })),
    );
    const { store, wrapper } = setup();
    const hook = renderHook(() => useGetCapsQuery(), { wrapper });
    await waitFor(() => expect(hook.result.current.isSuccess).toBe(true));
    expect(counter.caps).toBe(1);
    await act(async () => {
      await store.dispatch(
        modelsApi.endpoints.updateModel.initiate({
          provider: "openai",
          type: "chat",
          model: STUB_CAPS_RESPONSE.chat_models["gpt-4o"],
        }),
      );
    });
    await waitFor(() => expect(counter.caps).toBe(2));
    await act(async () => {
      await store.dispatch(
        modelsApi.endpoints.deleteModel.initiate({
          provider: "openai",
          type: "chat",
          model: "gpt-4o",
        }),
      );
    });
    await waitFor(() => expect(counter.caps).toBe(3));
    hook.unmount();
  });

  it("dedupes mounts inside the staleness window and refetches after it, on focus and on reconnect", async () => {
    useFakeTimers();
    const counter = countCaps();
    server.use(http.get("*/v1/ping", () => HttpResponse.text("pong")));
    const { wrapper } = setup();
    const first = renderHook(() => useGetCapsQuery(), { wrapper });
    await vi.waitFor(() => expect(first.result.current.isSuccess).toBe(true));
    const burst = [1, 2, 3].map(() =>
      renderHook(() => useGetCapsQuery(), { wrapper }),
    );
    await advance(10000);
    expect(counter.caps).toBe(1);
    for (const hook of burst) hook.unmount();
    await advance(21000);
    const late = renderHook(() => useGetCapsQuery(), { wrapper });
    await vi.waitFor(() => expect(counter.caps).toBe(2));
    act(() => {
      window.dispatchEvent(new Event("focus"));
    });
    await vi.waitFor(() => expect(counter.caps).toBe(3));
    act(() => {
      window.dispatchEvent(new Event("online"));
    });
    await vi.waitFor(() => expect(counter.caps).toBe(4));
    await advance(5000);
    expect(counter.caps).toBe(4);
    late.unmount();
    first.unmount();
  });

  it("refetches caps when ping recovers after a single failure", async () => {
    useFakeTimers();
    const counter = countCaps();
    let healthy = true;
    server.use(
      http.get("*/v1/ping", () =>
        healthy
          ? HttpResponse.text("pong")
          : HttpResponse.text("engine restarting", { status: 503 }),
      ),
    );
    const { store, wrapper } = setup();
    const hook = renderHook(() => useGetCapsQuery(), { wrapper });
    await vi.waitFor(() => expect(hook.result.current.isSuccess).toBe(true));
    await advance(100);
    expect(counter.caps).toBe(1);
    healthy = false;
    await advance(8000);
    expect(store.getState().connection.backendStatus).toBe("online");
    expect(counter.caps).toBe(1);
    healthy = true;
    await advance(2000);
    await vi.waitFor(() => expect(counter.caps).toBe(2));
    expect(store.getState().connection.backendStatus).toBe("online");
    hook.unmount();
  });
});

function deferred() {
  let release: () => void = () => undefined;
  const promise = new Promise<void>((resolve) => {
    release = resolve;
  });
  return { promise, release };
}

describe("ping cadence", () => {
  it("shares ping at 8s healthy and goes offline only after two failed polls at 2s", async () => {
    let requests = 0;
    let healthy = true;
    server.use(
      http.get("*/v1/ping", () => {
        requests++;
        return healthy
          ? HttpResponse.text("pong")
          : HttpResponse.text("engine restarting", { status: 503 });
      }),
    );
    useFakeTimers();
    const { store, wrapper } = setup();
    const first = renderHook(() => useGetPing(), { wrapper });
    const second = renderHook(() => useGetPing(), { wrapper });
    await vi.waitFor(() => expect(first.result.current.isSuccess).toBe(true));
    expect(requests).toBe(1);
    await advance(7900);
    expect(requests).toBe(1);
    await advance(200);
    expect(requests).toBe(2);
    healthy = false;
    await advance(8000);
    expect(requests).toBe(3);
    expect(store.getState().connection.backendStatus).toBe("online");
    await advance(2000);
    expect(requests).toBe(4);
    expect(store.getState().connection.backendStatus).toBe("offline");
    expect(store.getState().connection.backendError).toBe(
      "Ping failed with HTTP 503: engine restarting",
    );
    first.unmount();
    second.unmount();
  });

  it("counts overlapping failures of the same moment once", async () => {
    let requests = 0;
    let gate = deferred();
    server.use(
      http.get("*/v1/ping", async () => {
        requests++;
        await gate.promise;
        return HttpResponse.text("down", { status: 503 });
      }),
    );
    useFakeTimers();
    const { store, wrapper } = setup();
    const hook = renderHook(() => useGetPing(), { wrapper });
    await vi.waitFor(() => expect(requests).toBe(1));
    act(() => {
      void hook.result.current.refetch();
    });
    setVisibility("hidden");
    setVisibility("visible");
    gate.release();
    await vi.waitFor(() => expect(hook.result.current.isError).toBe(true));
    await advance(10);
    expect(store.getState().connection.backendStatus).toBe("online");
    gate = deferred();
    await advance(2000);
    gate.release();
    await vi.waitFor(() =>
      expect(store.getState().connection.backendStatus).toBe("offline"),
    );
    expect(requests).toBe(2);
    hook.unmount();
  });

  it("ignores a late failure from a previous endpoint", async () => {
    const oldGate = deferred();
    let oldRequests = 0;
    server.use(
      http.get("http://127.0.0.1:8001/v1/ping", async () => {
        oldRequests++;
        if (oldRequests > 1) await oldGate.promise;
        return HttpResponse.text("down", { status: 503 });
      }),
      http.get("http://127.0.0.1:8002/v1/ping", () =>
        HttpResponse.text("down", { status: 503 }),
      ),
    );
    useFakeTimers();
    const { store, wrapper } = setup();
    const hook = renderHook(() => useGetPing(), { wrapper });
    await vi.waitFor(() => expect(hook.result.current.isError).toBe(true));
    await advance(2000);
    expect(oldRequests).toBe(2);
    act(() => {
      store.dispatch(updateConfig({ lspPort: 8002 }));
    });
    await vi.waitFor(() => expect(hook.result.current.isError).toBe(true));
    oldGate.release();
    await advance(10);
    expect(store.getState().config.lspPort).toBe(8002);
    expect(store.getState().connection.backendStatus).toBe("online");
    hook.unmount();
  });

  it("does not ping while hidden and pings immediately when visible again", async () => {
    let requests = 0;
    server.use(
      http.get("*/v1/ping", () => {
        requests++;
        return HttpResponse.text("pong");
      }),
    );
    useFakeTimers();
    const { wrapper } = setup();
    const hook = renderHook(() => useGetPing(), { wrapper });
    await vi.waitFor(() => expect(hook.result.current.isSuccess).toBe(true));
    expect(requests).toBe(1);
    setVisibility("hidden");
    await advance(60000);
    expect(requests).toBe(1);
    setVisibility("visible");
    await advance(10);
    expect(requests).toBe(2);
    await advance(8000);
    expect(requests).toBe(3);
    hook.unmount();
  });
});

function organizationServer(authorized: () => boolean) {
  const counter = { taxonomy: 0, items: 0 };
  server.use(
    http.get("*/v1/organization/taxonomy", () => {
      counter.taxonomy++;
      return authorized()
        ? HttpResponse.json({ revision: 1, items: [], next_cursor: null })
        : HttpResponse.json({}, { status: 401 });
    }),
    http.post("*/v1/organization/items", () => {
      counter.items++;
      return authorized()
        ? HttpResponse.json({ revision: 1, assignments: [] })
        : HttpResponse.json({}, { status: 401 });
    }),
  );
  return counter;
}

describe("organization cadence", () => {
  it("suspends periodic and focus requests after 401 while retry still reaches the network", async () => {
    let authorized = false;
    const counter = organizationServer(() => authorized);
    useFakeTimers();
    const { store, wrapper } = setup();
    withRtkWindowListeners(store);
    const hook = renderHook(() => useOrganizationData(), { wrapper });
    await vi.waitFor(() => expect(hook.result.current.error).toBe(true));
    await advance(10);
    expect(counter).toEqual({ taxonomy: 2, items: 1 });
    await advance(120000);
    act(() => {
      window.dispatchEvent(new Event("focus"));
    });
    await advance(10);
    expect(counter).toEqual({ taxonomy: 2, items: 1 });
    act(() => {
      hook.result.current.retry();
    });
    await advance(10);
    expect(counter).toEqual({ taxonomy: 4, items: 2 });
    expect(hook.result.current.error).toBe(true);
    authorized = true;
    act(() => {
      hook.result.current.retry();
    });
    await vi.waitFor(() => expect(hook.result.current.error).toBe(false));
    expect(counter).toEqual({ taxonomy: 6, items: 3 });
    await advance(30000);
    expect(counter).toEqual({ taxonomy: 8, items: 4 });
    hook.unmount();
  });

  it("re-requests after resetApiState and API key change, then resumes polling", async () => {
    let authorized = false;
    const counter = organizationServer(() => authorized);
    useFakeTimers();
    const { store, wrapper } = setup();
    const hook = renderHook(() => useOrganizationData(), { wrapper });
    await vi.waitFor(() => expect(hook.result.current.error).toBe(true));
    await advance(10);
    expect(counter.taxonomy).toBe(2);
    act(() => {
      store.dispatch(setApiKey("refreshed"));
    });
    await advance(10);
    expect(counter).toEqual({ taxonomy: 4, items: 2 });
    authorized = true;
    act(() => {
      store.dispatch(organizationApi.util.resetApiState());
    });
    await vi.waitFor(() =>
      expect(hook.result.current.assignments.isSuccess).toBe(true),
    );
    await advance(10);
    expect(counter).toEqual({ taxonomy: 6, items: 3 });
    await advance(30000);
    expect(counter).toEqual({ taxonomy: 8, items: 4 });
    hook.unmount();
  });

  it("re-requests when the backend comes back online and on a fresh mount", async () => {
    const counter = organizationServer(() => false);
    useFakeTimers();
    const { store, wrapper } = setup();
    const hook = renderHook(() => useOrganizationData(), { wrapper });
    await vi.waitFor(() => expect(hook.result.current.error).toBe(true));
    await advance(10);
    act(() => {
      store.dispatch(setBackendStatus({ status: "offline" }));
    });
    act(() => {
      store.dispatch(setBackendStatus({ status: "online" }));
    });
    await advance(10);
    expect(counter).toEqual({ taxonomy: 4, items: 2 });
    hook.unmount();
    const remount = renderHook(() => useOrganizationData(), { wrapper });
    await advance(10);
    expect(counter).toEqual({ taxonomy: 6, items: 3 });
    remount.unmount();
  });

  it("shares one cache entry, one poller and one suspended state across consumers", async () => {
    let authorized = true;
    const counter = organizationServer(() => authorized);
    useFakeTimers();
    const { wrapper } = setup();
    const filterBar = renderHook(() => useOrganizationData(), { wrapper });
    const stream = renderHook(() => useOrganizationData(), { wrapper });
    await vi.waitFor(() =>
      expect(stream.result.current.assignments.isSuccess).toBe(true),
    );
    await advance(10);
    const afterMount = { ...counter };
    expect(afterMount.items).toBeLessThanOrEqual(2);
    await advance(30000);
    expect(counter).toEqual({
      taxonomy: afterMount.taxonomy + 2,
      items: afterMount.items + 1,
    });
    authorized = false;
    await advance(30000);
    expect(filterBar.result.current.error).toBe(true);
    expect(stream.result.current.error).toBe(true);
    const suspendedAt = { ...counter };
    await advance(90000);
    expect(counter).toEqual(suspendedAt);
    filterBar.unmount();
    stream.unmount();
  });
});
