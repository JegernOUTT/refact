import { configureStore } from "@reduxjs/toolkit";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { EngineApiConfig } from "./apiUrl";
import { FILES_TREE_REQUEST_TIMEOUT_MS, filesApi } from "./files";

type TestConfigState = EngineApiConfig & {
  apiKey: string | null;
};

const createTestStore = (config: TestConfigState) =>
  configureStore({
    reducer: {
      config: (state: TestConfigState = config) => state,
      [filesApi.reducerPath]: filesApi.reducer,
    },
    middleware: (getDefaultMiddleware) =>
      getDefaultMiddleware().concat(filesApi.middleware),
  });

describe("filesApi", () => {
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("times out a workspace tree request that never settles", async () => {
    vi.useFakeTimers();
    const fetchMock = vi.fn<typeof fetch>((input, init) => {
      const request =
        input instanceof Request ? input : new Request(input, init);
      return new Promise<Response>((_resolve, reject) => {
        request.signal.addEventListener(
          "abort",
          () => reject(new DOMException("Aborted", "AbortError")),
          { once: true },
        );
      });
    });
    vi.stubGlobal("fetch", fetchMock);
    const store = createTestStore({
      host: "ide",
      lspPort: 8123,
      apiKey: null,
    });

    const request = store.dispatch(
      filesApi.endpoints.getFilesTree.initiate("/workspace"),
    );
    await vi.advanceTimersByTimeAsync(FILES_TREE_REQUEST_TIMEOUT_MS);
    const result = await request;
    request.unsubscribe();

    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(result.error).toMatchObject({ status: "TIMEOUT_ERROR" });
    expect(
      filesApi.endpoints.getFilesTree.select("/workspace")(store.getState()),
    ).toMatchObject({ isError: true, isLoading: false, status: "rejected" });
  });
});
