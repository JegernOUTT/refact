import React from "react";
import { Provider } from "react-redux";
import { renderHook, waitFor } from "@testing-library/react";
import { act } from "react-dom/test-utils";
import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";

import { setUpStore } from "../app/store";
import { setBackendStatus } from "../features/Connection";
import { useGetPing } from "../hooks/useGetPing";
import { server } from "../utils/mockServer";

function createWrapper() {
  const store = setUpStore({
    config: {
      apiKey: "test",
      lspPort: 8001,
      themeProps: {},
      host: "vscode",
    },
  });
  store.dispatch(setBackendStatus({ status: "online" }));

  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <Provider store={store}>{children}</Provider>
  );

  return { store, wrapper };
}

function queuePingResponses(responses: ("ok" | "error")[]) {
  let requests = 0;
  server.use(
    http.get("*/v1/ping", () => {
      const response = responses[Math.min(requests, responses.length - 1)];
      requests += 1;
      if (response === "ok") return HttpResponse.text("pong");
      return HttpResponse.text("nope", { status: 500 });
    }),
  );
  return () => requests;
}

describe("useGetPing", () => {
  it("requires two consecutive ping failures before marking the backend offline", async () => {
    const getRequestCount = queuePingResponses(["error", "error"]);
    const { store, wrapper } = createWrapper();

    const { result } = renderHook(() => useGetPing(), { wrapper });

    await waitFor(() => expect(getRequestCount()).toBe(1));
    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(store.getState().connection.backendStatus).toBe("online");

    await act(async () => {
      await result.current.refetch();
    });

    await waitFor(() => expect(getRequestCount()).toBe(2));
    await waitFor(() =>
      expect(store.getState().connection.backendStatus).toBe("offline"),
    );
  });

  it("resets the ping failure counter after a successful ping", async () => {
    const getRequestCount = queuePingResponses([
      "error",
      "ok",
      "error",
      "error",
    ]);
    const { store, wrapper } = createWrapper();

    const { result } = renderHook(() => useGetPing(), { wrapper });

    await waitFor(() => expect(getRequestCount()).toBe(1));
    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(store.getState().connection.backendStatus).toBe("online");

    await act(async () => {
      await result.current.refetch();
    });

    await waitFor(() => expect(getRequestCount()).toBe(2));
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(store.getState().connection.backendStatus).toBe("online");

    await act(async () => {
      await result.current.refetch();
    });

    await waitFor(() => expect(getRequestCount()).toBe(3));
    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(store.getState().connection.backendStatus).toBe("online");

    await act(async () => {
      await result.current.refetch();
    });

    await waitFor(() => expect(getRequestCount()).toBe(4));
    await waitFor(() =>
      expect(store.getState().connection.backendStatus).toBe("offline"),
    );
  });
});
