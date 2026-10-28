import React from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { Provider } from "react-redux";
import { http, HttpResponse, delay } from "msw";

import { setUpStore } from "../app/store";
import { STUB_CAPS_RESPONSE } from "../__fixtures__/caps";
import { useGetCapsQuery } from "../hooks/useGetCapsQuery";
import { capsApi } from "../services/refact";
import { server } from "../utils/mockServer";
import { setBackendStatus } from "../features/Connection";
import { getErrorMessage } from "../features/Errors/errorsSlice";
import { sidebarSubscriptionStarted } from "../features/Sidebar/sidebarSlice";
import * as buddyErrors from "../features/Buddy/reportBuddyFrontendError";

function createStartupStore() {
  const store = setUpStore({
    config: {
      apiKey: "test",
      lspPort: 8001,
      themeProps: {},
      host: "vscode",
    },
  });
  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <Provider store={store}>{children}</Provider>
  );
  return { store, wrapper };
}

describe("backend startup", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("requests caps without waiting for the first ping and goes online on success", async () => {
    let capsRequests = 0;
    server.use(
      http.get("*/v1/ping", async () => {
        await delay(400);
        return HttpResponse.text("pong");
      }),
      http.get("*/v1/caps", () => {
        capsRequests += 1;
        return HttpResponse.json(STUB_CAPS_RESPONSE);
      }),
    );
    const { store, wrapper } = createStartupStore();
    expect(store.getState().connection.backendStatus).toBe("unknown");

    const { result } = renderHook(() => useGetCapsQuery(), { wrapper });

    await waitFor(() => expect(result.current.isSuccess).toBe(true), {
      timeout: 300,
    });
    expect(capsRequests).toBe(1);
    expect(store.getState().connection.backendStatus).toBe("online");
  });

  it("keeps an early caps result when the backend turns online", async () => {
    server.use(
      http.get("*/v1/ping", () => HttpResponse.text("pong")),
      http.get("*/v1/caps", () => HttpResponse.json(STUB_CAPS_RESPONSE)),
    );
    const { store } = createStartupStore();

    await store.dispatch(capsApi.endpoints.getCaps.initiate(undefined));
    store.dispatch(setBackendStatus({ status: "online" }));

    expect(
      capsApi.endpoints.getCaps.select(undefined)(store.getState()).data,
    ).toEqual(STUB_CAPS_RESPONSE);
  });

  it("still resets caps when the backend recovers from offline", async () => {
    server.use(
      http.get("*/v1/caps", () => HttpResponse.json(STUB_CAPS_RESPONSE)),
    );
    const { store } = createStartupStore();
    store.dispatch(setBackendStatus({ status: "online" }));
    await store.dispatch(capsApi.endpoints.getCaps.initiate(undefined));
    store.dispatch(setBackendStatus({ status: "offline" }));
    await store.dispatch(capsApi.endpoints.getCaps.initiate(undefined));

    store.dispatch(setBackendStatus({ status: "online" }));

    expect(
      capsApi.endpoints.getCaps.select(undefined)(store.getState()).data,
    ).toBeUndefined();
  });

  it("does not raise a caps error while the engine is still booting, and retries once online", async () => {
    const report = vi
      .spyOn(buddyErrors, "reportBuddyFrontendError")
      .mockResolvedValue(undefined);
    let capsRequests = 0;
    let engineUp = false;
    server.use(
      http.get("*/v1/ping", () =>
        engineUp
          ? HttpResponse.text("pong")
          : HttpResponse.text("booting", { status: 503 }),
      ),
      http.get("*/v1/caps", () => {
        capsRequests += 1;
        return engineUp
          ? HttpResponse.json(STUB_CAPS_RESPONSE)
          : HttpResponse.text("booting", { status: 503 });
      }),
    );
    const { store, wrapper } = createStartupStore();

    const { result } = renderHook(() => useGetCapsQuery(), { wrapper });

    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(store.getState().connection.backendStatus).toBe("unknown");
    expect(getErrorMessage(store.getState())).toBeNull();
    expect(report).not.toHaveBeenCalled();

    engineUp = true;
    store.dispatch(setBackendStatus({ status: "online" }));

    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(capsRequests).toBeGreaterThanOrEqual(2);
    expect(getErrorMessage(store.getState())).toBeNull();
  });

  it("treats a started sidebar subscription as the engine being online", () => {
    const { store } = createStartupStore();

    store.dispatch(
      sidebarSubscriptionStarted({ subscriptionId: "s-1", lspPort: 8001 }),
    );

    expect(store.getState().connection.backendStatus).toBe("online");
  });
});
