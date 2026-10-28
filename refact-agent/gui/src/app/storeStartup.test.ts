import { afterEach, describe, expect, it, vi } from "vitest";

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("store startup", () => {
  it("reports UI errors with the dispatching store's endpoint and credentials", async () => {
    const { setUpStore, store } = await import("./store");
    const { setError } = await import("../features/Errors/errorsSlice");
    const secondaryStore = setUpStore({
      config: {
        ...store.getState().config,
        host: "ide",
        lspPort: 8125,
        apiKey: "secondary-key",
      },
    });
    const fetchMock = vi
      .fn<typeof fetch>()
      .mockResolvedValue(new Response("{}", { status: 200 }));
    vi.stubGlobal("fetch", fetchMock);

    secondaryStore.dispatch(setError("secondary store error"));

    expect(fetchMock).toHaveBeenCalledWith(
      "http://127.0.0.1:8125/v1/buddy/diagnostics/collect",
      expect.objectContaining({ method: "POST" }),
    );
    expect(
      new Headers(fetchMock.mock.calls[0][1]?.headers).get("Authorization"),
    ).toBe("Bearer secondary-key");
  });

  it("initializes the real store when middleware is imported first", async () => {
    vi.resetModules();
    const { listenerMiddleware } = await import("./middleware");
    const { store, persistor } = await import("./store");
    try {
      expect(listenerMiddleware.middleware).toBeTypeOf("function");
      expect(store.getState().config).toBeDefined();
      expect(() => store.dispatch({ type: "startup/probe" })).not.toThrow();
      const { updateConfig } = await import("../features/Config/configSlice");
      const { reportBuddyFrontendError } = await import(
        "../features/Buddy/reportBuddyFrontendError"
      );
      store.dispatch(
        updateConfig({ host: "ide", lspPort: 8124, apiKey: "singleton-key" }),
      );
      const post = vi.fn().mockResolvedValue(undefined);
      await reportBuddyFrontendError(
        { source: "window_error", error: "startup reporter probe" },
        { post },
      );
      expect(post).toHaveBeenCalledWith(
        store.getState().config,
        "singleton-key",
        expect.objectContaining({
          error: "[frontend:window_error] startup reporter probe",
        }),
      );
    } finally {
      persistor.pause();
    }
  });
});
