import React from "react";
import { describe, expect, it } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { Provider } from "react-redux";
import { http, HttpResponse } from "msw";

import { setUpStore } from "../app/store";
import { EMPTY_CAPS_RESPONSE, STUB_CAPS_RESPONSE } from "../__fixtures__/caps";
import { useGetCapsQuery } from "../hooks/useGetCapsQuery";
import { capsApi } from "../services/refact";
import { server } from "../utils/mockServer";
import { updateConfig } from "../features/Config/configSlice";

function createWrapper() {
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

describe("useGetCapsQuery", () => {
  it("keeps retrying when caps loads before chat models are ready", async () => {
    let capsRequests = 0;
    server.use(
      http.get("*/v1/ping", () => {
        return HttpResponse.text("pong");
      }),
      http.get("*/v1/caps", () => {
        capsRequests += 1;
        if (capsRequests === 1) {
          return HttpResponse.json(EMPTY_CAPS_RESPONSE);
        }
        return HttpResponse.json(STUB_CAPS_RESPONSE);
      }),
    );

    const { store, wrapper } = createWrapper();

    const { result } = renderHook(() => useGetCapsQuery(), { wrapper });

    await waitFor(() => {
      expect(Object.keys(result.current.data?.chat_models ?? {})).toHaveLength(
        Object.keys(STUB_CAPS_RESPONSE.chat_models).length,
      );
    });
    expect(capsRequests).toBeGreaterThanOrEqual(2);

    store.dispatch(capsApi.util.resetApiState());
  });

  it("uses canonical ping URLs for dev and engine-served web configs", async () => {
    const pingUrls: string[] = [];
    server.use(
      http.get("/v1/ping", ({ request }) => {
        pingUrls.push(new URL(request.url).pathname);
        return HttpResponse.text("pong");
      }),
      http.get("/v1/caps", () => HttpResponse.json(STUB_CAPS_RESPONSE)),
      http.get("*/v1/caps", () =>
        HttpResponse.json(STUB_CAPS_RESPONSE),
      ),
    );

    const store = setUpStore({
      config: {
        apiKey: "test",
        lspPort: 8001,
        themeProps: {},
        host: "web",
        dev: true,
        lspUrl: "http://localhost:5173/v1/ping/Refact",
      },
    });
    const wrapper = ({ children }: { children: React.ReactNode }) => (
      <Provider store={store}>{children}</Provider>
    );

    renderHook(() => useGetCapsQuery(), { wrapper });

    await waitFor(() => {
      expect(pingUrls).toEqual(["/v1/ping"]);
    });

    store.dispatch(updateConfig({ dev: false, engineServed: true }));

    await waitFor(() => {
      expect(pingUrls).toEqual(["/v1/ping", "/v1/ping"]);
    });

    store.dispatch(capsApi.util.resetApiState());
  });

  it("sanitizes stale ping paths when building local fallback ping URLs", async () => {
    let pingUrl = "";
    server.use(
      http.get("*/v1/ping", ({ request }) => {
        pingUrl = request.url;
        return HttpResponse.text("pong");
      }),
      http.get("*/v1/caps", () =>
        HttpResponse.json(STUB_CAPS_RESPONSE),
      ),
    );

    const store = setUpStore({
      config: {
        apiKey: "test",
        lspPort: 8001,
        themeProps: {},
        host: "vscode",
        lspUrl: "http://127.0.0.1:8001/v1/ping/Refact",
      },
    });
    const wrapper = ({ children }: { children: React.ReactNode }) => (
      <Provider store={store}>{children}</Provider>
    );

    renderHook(() => useGetCapsQuery(), { wrapper });

    await waitFor(() => {
      expect(pingUrl).toBe("http://127.0.0.1:8001/v1/ping");
    });

    store.dispatch(capsApi.util.resetApiState());
  });

  it("fetches ping and caps from a remote web lspUrl when lspPort is zero", async () => {
    const pingUrls: string[] = [];
    const capsUrls: string[] = [];
    server.use(
      http.get("https://remote.example.com/v1/ping", ({ request }) => {
        pingUrls.push(request.url);
        return HttpResponse.text("pong");
      }),
      http.get("https://remote.example.com/v1/caps", ({ request }) => {
        capsUrls.push(request.url);
        return HttpResponse.json(STUB_CAPS_RESPONSE);
      }),
    );

    const store = setUpStore({
      config: {
        apiKey: "test",
        lspPort: 0,
        themeProps: {},
        host: "web",
        lspUrl: "https://remote.example.com/v1/ping",
      },
    });
    const wrapper = ({ children }: { children: React.ReactNode }) => (
      <Provider store={store}>{children}</Provider>
    );

    const { result } = renderHook(() => useGetCapsQuery(), { wrapper });

    await waitFor(() => {
      expect(result.current.data?.chat_models).toEqual(
        STUB_CAPS_RESPONSE.chat_models,
      );
    });
    expect(pingUrls).toContain("https://remote.example.com/v1/ping");
    expect(capsUrls).toContain("https://remote.example.com/v1/caps");

    store.dispatch(capsApi.util.resetApiState());
  });

  it("resets endpoint-bound state when lspUrl changes without a port change", () => {
    const store = setUpStore({
      config: {
        apiKey: "test",
        lspPort: 8001,
        themeProps: {},
        host: "vscode",
        lspUrl: "http://127.0.0.1:8001",
      },
      sidebar: {
        subscriptionId: "test-sidebar",
        lspPort: 8001,
        sections: {
          workspace: { status: "ready", error: null },
          chats: { status: "ready", error: null },
          tasks: { status: "ready", error: null },
          buddy: { status: "ready", error: null },
        },
      },
    });

    store.dispatch(
      updateConfig({ lspUrl: "http://127.0.0.1:8002", lspPort: 8001 }),
    );

    expect(store.getState().sidebar.subscriptionId).toBeNull();
    expect(store.getState().sidebar.lspPort).toBe(8001);
    expect(store.getState().sidebar.sections).toMatchObject({
      workspace: { status: "loading" },
      chats: { status: "loading" },
      tasks: { status: "loading" },
      buddy: { status: "loading" },
    });
  });

  const endpointConfigCases = [
    ["host", { host: "jetbrains" as const }],
    ["dev", { dev: true }],
    ["engineServed", { engineServed: true }],
    ["lspPort", { lspPort: 8002 }],
  ] as const;

  it.each(endpointConfigCases)(
    "resets endpoint-bound state when %s changes",
    (_name, patch) => {
      const store = setUpStore({
        config: {
          apiKey: "test",
          lspPort: 8001,
          themeProps: {},
          host: "vscode",
          lspUrl: "http://127.0.0.1:8001",
        },
        sidebar: {
          subscriptionId: "test-sidebar",
          lspPort: 8001,
          sections: {
            workspace: { status: "ready", error: null },
            chats: { status: "ready", error: null },
            tasks: { status: "ready", error: null },
            buddy: { status: "ready", error: null },
          },
        },
      });

      store.dispatch(updateConfig(patch));

      expect(store.getState().sidebar.subscriptionId).toBeNull();
      expect(store.getState().sidebar.sections).toMatchObject({
        workspace: { status: "loading" },
        chats: { status: "loading" },
        tasks: { status: "loading" },
        buddy: { status: "loading" },
      });
    },
  );
});
