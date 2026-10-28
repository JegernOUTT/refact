import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  configureBuddyFrontendErrorReporter,
  reportBuddyFrontendError,
  resetBuddyFrontendErrorReportCache,
} from "./reportBuddyFrontendError";

const config = { host: "ide" as const, lspPort: 8001, apiKey: "first-key" };
const args = {
  source: "window_error" as const,
  error: "failure token=private",
};

beforeEach(() => {
  resetBuddyFrontendErrorReportCache();
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("frontend error reporter wiring", () => {
  it("reads the configured getter at report time and uses the real transport", async () => {
    let state = { config };
    configureBuddyFrontendErrorReporter(() => state);
    state = { config: { ...config, lspPort: 8123, apiKey: "current-key" } };
    const fetchMock = vi
      .fn<typeof fetch>()
      .mockResolvedValue(new Response("{}", { status: 200 }));
    vi.stubGlobal("fetch", fetchMock);

    await reportBuddyFrontendError(args);

    expect(fetchMock).toHaveBeenCalledWith(
      "http://127.0.0.1:8123/v1/buddy/diagnostics/collect",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({
          error: "[frontend:window_error] failure token=[REDACTED]",
          source_file: "frontend/window_error",
          tool_name: "window_error",
        }),
      }),
    );
    expect(
      new Headers(fetchMock.mock.calls[0][1]?.headers).get("Authorization"),
    ).toBe("Bearer current-key");
  });

  it("preserves explicit dependencies, redaction and timed deduplication", async () => {
    let now = 1000;
    const post = vi.fn().mockResolvedValue(undefined);
    const deps = { getState: () => ({ config }), post, now: () => now };
    await reportBuddyFrontendError(args, deps);
    await reportBuddyFrontendError(args, deps);
    expect(post).toHaveBeenCalledTimes(1);
    expect(post).toHaveBeenCalledWith(
      config,
      "first-key",
      expect.objectContaining({
        error: "[frontend:window_error] failure token=[REDACTED]",
      }),
    );
    now += 10001;
    await reportBuddyFrontendError(args, deps);
    expect(post).toHaveBeenCalledTimes(2);
  });

  it("skips unavailable endpoints and swallows transport failures", async () => {
    const post = vi.fn().mockRejectedValue(new Error("offline"));
    await reportBuddyFrontendError(args, {
      getState: () => ({ config: { ...config, backendReady: false } }),
      post,
    });
    expect(post).not.toHaveBeenCalled();
    await expect(
      reportBuddyFrontendError(args, {
        getState: () => ({ config }),
        post,
      }),
    ).resolves.toBeUndefined();
    expect(post).toHaveBeenCalledTimes(1);
  });
});
