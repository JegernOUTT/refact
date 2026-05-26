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
      http.get("http://127.0.0.1:8001/v1/ping", () => {
        return HttpResponse.text("pong");
      }),
      http.get("http://127.0.0.1:8001/v1/caps", () => {
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
});
