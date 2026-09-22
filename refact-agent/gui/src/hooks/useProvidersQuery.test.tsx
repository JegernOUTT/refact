import React from "react";
import { Provider } from "react-redux";
import { http, HttpResponse } from "msw";
import { renderHook, waitFor } from "@testing-library/react";
import { act } from "react-dom/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { STUB_CAPS_RESPONSE } from "../__fixtures__/caps";
import {
  activeProvider,
  capsFailsOnceThenSucceeds,
} from "../__fixtures__/providers";
import { setUpStore } from "../app/store";
import { updateConfig } from "../features/Config/configSlice";
import { setBackendStatus } from "../features/Connection";
import { server } from "../utils/mockServer";
import { useProviderBootstrapState } from "./useProvidersQuery";

function setup({ offline }: { offline?: boolean } = {}) {
  const store = setUpStore({
    config: {
      apiKey: "test",
      lspPort: 8001,
      themeProps: {},
      host: "vscode",
    },
  });
  store.dispatch(
    updateConfig({ backendReady: true, connectionStatus: "ready" }),
  );
  store.dispatch(
    setBackendStatus(offline ? { status: "offline" } : { status: "online" }),
  );
  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <Provider store={store}>{children}</Provider>
  );
  return renderHook(() => useProviderBootstrapState(), { wrapper });
}

beforeEach(() => {
  vi.useRealTimers();
  server.use(http.get("*/v1/ping", () => HttpResponse.text("pong")));
});

describe("useProviderBootstrapState retry", () => {
  it("refetches both providers and caps", async () => {
    let providersHits = 0;
    let capsHits = 0;
    server.use(
      http.get("*/v1/providers", () => {
        providersHits += 1;
        return HttpResponse.json({ providers: [activeProvider] });
      }),
      http.get("*/v1/caps", () => {
        capsHits += 1;
        return HttpResponse.json({ detail: "caps exploded" }, { status: 500 });
      }),
    );

    const { result } = setup();

    await waitFor(() => {
      expect(result.current.status).toBe("provider_error");
    });

    const providersHitsBeforeRetry = providersHits;
    const capsHitsBeforeRetry = capsHits;

    act(() => {
      result.current.retry();
    });
    expect(result.current.isRetrying).toBe(true);

    await waitFor(() => {
      expect(result.current.isRetrying).toBe(false);
    });

    expect(providersHits).toBeGreaterThan(providersHitsBeforeRetry);
    expect(capsHits).toBeGreaterThan(capsHitsBeforeRetry);
    expect(result.current.status).toBe("provider_error");
  });

  it("reaches ready when a retried caps request succeeds", async () => {
    const caps = capsFailsOnceThenSucceeds();
    server.use(
      http.get("*/v1/providers", () =>
        HttpResponse.json({ providers: [activeProvider] }),
      ),
      caps.handler,
    );

    const { result } = setup();

    await waitFor(() => {
      expect(result.current.status).toBe("provider_error");
    });

    act(() => {
      result.current.retry();
    });

    await waitFor(() => {
      expect(result.current.status).toBe("ready");
    });
    expect(result.current.isRetrying).toBe(false);
    expect(caps.hits()).toBeGreaterThan(1);
  });

  it("does not throw and resets isRetrying when both queries are skipped", async () => {
    server.use(
      http.get("*/v1/ping", () => HttpResponse.error()),
      http.get("*/v1/providers", () =>
        HttpResponse.json({ providers: [activeProvider] }),
      ),
      http.get("*/v1/caps", () => HttpResponse.json(STUB_CAPS_RESPONSE)),
    );

    const { result } = setup({ offline: true });

    await waitFor(() => {
      expect(result.current.status).toBe("backend_offline");
    });

    expect(() => {
      act(() => {
        result.current.retry();
      });
    }).not.toThrow();

    await waitFor(() => {
      expect(result.current.isRetrying).toBe(false);
    });
    expect(result.current.status).toBe("backend_offline");
  });

  it("keeps status pure while a retry is in flight", async () => {
    server.use(
      http.get("*/v1/providers", () =>
        HttpResponse.json({ providers: [activeProvider] }),
      ),
      http.get("*/v1/caps", () => HttpResponse.json(STUB_CAPS_RESPONSE)),
    );

    const { result } = setup();

    await waitFor(() => {
      expect(result.current.status).toBe("ready");
    });

    act(() => {
      result.current.retry();
    });

    expect(result.current.isRetrying).toBe(true);
    expect(result.current.status).not.toBe("provider_error");

    await waitFor(() => {
      expect(result.current.isRetrying).toBe(false);
    });
    expect(result.current.status).toBe("ready");
  });
});
