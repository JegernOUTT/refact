import React from "react";
import { describe, expect, it } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { Provider } from "react-redux";
import { http, HttpResponse } from "msw";

import { setUpStore } from "../app/store";
import { setBackendStatus } from "../features/Connection";
import { useGetConfiguredProvidersQuery } from "../hooks/useProvidersQuery";
import { providersApi } from "../services/refact";
import type { Config } from "../features/Config/configSlice";
import { server } from "../utils/mockServer";

function createWrapper(config?: Partial<Config>) {
  const store = setUpStore({
    config: {
      apiKey: "test",
      lspPort: 8001,
      themeProps: {},
      host: "vscode",
      ...config,
    },
  });

  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <Provider store={store}>{children}</Provider>
  );

  return { store, wrapper };
}

describe("useGetConfiguredProvidersQuery", () => {
  it("skips providers while backend is offline and fetches after it becomes online", async () => {
    let providersRequests = 0;
    server.use(
      http.get("http://127.0.0.1:8001/v1/providers", () => {
        providersRequests += 1;
        return HttpResponse.json({ providers: [] });
      }),
    );

    const { store, wrapper } = createWrapper();
    store.dispatch(setBackendStatus({ status: "offline" }));

    const { result } = renderHook(() => useGetConfiguredProvidersQuery(), {
      wrapper,
    });

    expect(result.current.isUninitialized).toBe(true);
    expect(providersRequests).toBe(0);

    store.dispatch(setBackendStatus({ status: "online" }));

    await waitFor(() => {
      expect(result.current.isSuccess).toBe(true);
    });
    expect(providersRequests).toBe(1);

    store.dispatch(providersApi.util.resetApiState());
  });

  it("uses relative providers URL for dev web configs", async () => {
    const providerUrls: string[] = [];
    server.use(
      http.get("/v1/providers", ({ request }) => {
        providerUrls.push(new URL(request.url).pathname);
        return HttpResponse.json({ providers: [] });
      }),
    );

    const { store, wrapper } = createWrapper({ host: "web", dev: true });
    store.dispatch(setBackendStatus({ status: "online" }));

    const { result } = renderHook(() => useGetConfiguredProvidersQuery(), {
      wrapper,
    });

    await waitFor(() => {
      expect(result.current.isSuccess).toBe(true);
    });
    expect(providerUrls).toEqual(["/v1/providers"]);

    store.dispatch(providersApi.util.resetApiState());
  });

  it("uses sanitized remote providers URL for standalone web configs", async () => {
    let providerUrl = "";
    server.use(
      http.get(
        "https://remote.example.test/refact/v1/providers",
        ({ request }) => {
          providerUrl = request.url;
          return HttpResponse.json({ providers: [] });
        },
      ),
    );

    const { store, wrapper } = createWrapper({
      host: "web",
      lspUrl: "https://remote.example.test/refact/v1/ping/Refact",
    });
    store.dispatch(setBackendStatus({ status: "online" }));

    const { result } = renderHook(() => useGetConfiguredProvidersQuery(), {
      wrapper,
    });

    await waitFor(() => {
      expect(result.current.isSuccess).toBe(true);
    });
    expect(providerUrl).toBe("https://remote.example.test/refact/v1/providers");

    store.dispatch(providersApi.util.resetApiState());
  });
});
