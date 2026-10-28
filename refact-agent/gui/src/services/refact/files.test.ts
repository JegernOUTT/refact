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

  it("opens a manual path with an authenticated POST body", async () => {
    const fetchMock = vi.fn<typeof fetch>(() =>
      Promise.resolve(
        new Response(
          JSON.stringify({
            path: "/tmp/external.txt",
            content: "hello",
            language: null,
            size: 5,
            truncated: false,
            line_start: null,
            line_end: null,
            mtime_ms: 1,
            can_edit: false,
            image_mime: null,
            image_data: null,
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        ),
      ),
    );
    vi.stubGlobal("fetch", fetchMock);
    const store = createTestStore({
      host: "ide",
      lspPort: 8123,
      apiKey: "secret-key",
    });

    const request = store.dispatch(
      filesApi.endpoints.readFile.initiate({
        path: "/tmp/external.txt",
        manual: true,
      }),
    );
    const result = await request;
    request.unsubscribe();

    expect(fetchMock).toHaveBeenCalledTimes(1);
    const sent = fetchMock.mock.calls[0][0] as Request;
    const url = new URL(sent.url);
    expect(sent.method).toBe("POST");
    expect(url.pathname).toBe("/v1/files/open");
    expect(url.search).toBe("");
    expect(sent.url).not.toContain("secret-key");
    expect(sent.headers.get("Authorization")).toBe("Bearer secret-key");
    expect(await sent.json()).toEqual({ path: "/tmp/external.txt" });
    expect(result.data).toMatchObject({ can_edit: false });
  });

  it("keeps workspace reads on the GET read endpoint", async () => {
    const fetchMock = vi.fn<typeof fetch>(() =>
      Promise.resolve(
        new Response(
          JSON.stringify({
            path: "/workspace/a.ts",
            content: "",
            language: null,
            size: 0,
            truncated: false,
            line_start: null,
            line_end: null,
            mtime_ms: 1,
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        ),
      ),
    );
    vi.stubGlobal("fetch", fetchMock);
    const store = createTestStore({ host: "ide", lspPort: 8123, apiKey: null });

    const request = store.dispatch(
      filesApi.endpoints.readFile.initiate({ path: "/workspace/a.ts" }),
    );
    await request;
    request.unsubscribe();

    const sent = fetchMock.mock.calls[0][0] as Request;
    const url = new URL(sent.url);
    expect(sent.method).toBe("GET");
    expect(url.pathname).toBe("/v1/files/read");
    expect(url.searchParams.get("path")).toBe("/workspace/a.ts");
  });
});
