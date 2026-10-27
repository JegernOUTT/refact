import { afterEach, describe, expect, it, vi } from "vitest";
import { workFixture } from "./workFixtures";
import { subscribeToWork } from "./workSubscription";

afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

const frame = (seq: number) =>
  `event: snapshot\r\ndata: ${JSON.stringify({ ...workFixture, seq })}\r\n\r\n`;

describe("Work subscription", () => {
  it("sends bearer auth, handles split CRLF, ignores duplicates and resets after a gap", async () => {
    let stream!: ReadableStreamDefaultController<Uint8Array>;
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      body: new ReadableStream<Uint8Array>({
        start(controller) {
          stream = controller;
        },
      }),
    });
    vi.stubGlobal("fetch", fetchMock);
    const snapshots = vi.fn();
    let finish!: () => void;
    const completed = new Promise<void>((resolve) => {
      finish = resolve;
    });
    const errors = vi.fn(() => finish());
    const stop = subscribeToWork("/v1/work/subscribe", "test-token", {
      timeoutMs: 30_000,
      onSnapshot: snapshots,
      onError: errors,
    });
    const text = frame(1) + frame(1) + frame(2) + frame(4);
    for (const part of text.split(/(?<=\r)/))
      stream.enqueue(new TextEncoder().encode(part));
    await completed;
    expect(snapshots).toHaveBeenCalledTimes(2);
    expect(errors).toHaveBeenCalledWith(
      "Work sequence changed; fresh snapshot required",
      { kind: "reset" },
    );
    expect(fetchMock).toHaveBeenCalledWith(
      "/v1/work/subscribe",
      expect.objectContaining({
        headers: {
          Accept: "text/event-stream",
          Authorization: "Bearer test-token",
        },
      }),
    );
    stop();
  });

  it("aborts cleanly on unmount without invoking callbacks", async () => {
    let resolve!: (value: { ok: boolean; body: null }) => void;
    const response = new Promise<{ ok: boolean; body: null }>((done) => {
      resolve = done;
    });
    const fetchMock = vi.fn().mockReturnValue(response);
    vi.stubGlobal("fetch", fetchMock);
    const errors = vi.fn();
    const snapshots = vi.fn();
    const stop = subscribeToWork("/v1/work/subscribe", null, {
      timeoutMs: 30_000,
      onSnapshot: snapshots,
      onError: errors,
    });
    stop();
    resolve({ ok: false, body: null });
    await response;
    await Promise.resolve();
    expect(errors).not.toHaveBeenCalled();
    expect(snapshots).not.toHaveBeenCalled();
  });

  it("stops a batched response when the snapshot callback unmounts", async () => {
    let stream!: ReadableStreamDefaultController<Uint8Array>;
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({
        ok: true,
        body: new ReadableStream<Uint8Array>({
          start(controller) {
            stream = controller;
          },
        }),
      }),
    );
    let finish!: () => void;
    const completed = new Promise<void>((resolve) => {
      finish = resolve;
    });
    const errors = vi.fn();
    const snapshots = vi.fn(() => {
      stop();
      finish();
    });
    const stop = subscribeToWork("/v1/work/subscribe", null, {
      timeoutMs: 30_000,
      onSnapshot: snapshots,
      onError: errors,
    });
    stream.enqueue(new TextEncoder().encode(frame(1) + frame(2)));
    await completed;
    await Promise.resolve();
    expect(snapshots).toHaveBeenCalledOnce();
    expect(errors).not.toHaveBeenCalled();
  });

  it("reports timeout once and never delivers late frames", async () => {
    vi.useFakeTimers();
    let stream!: ReadableStreamDefaultController<Uint8Array>;
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({
        ok: true,
        body: new ReadableStream<Uint8Array>({
          start(controller) {
            stream = controller;
          },
        }),
      }),
    );
    const errors = vi.fn();
    const snapshots = vi.fn();
    const stop = subscribeToWork("/v1/work/subscribe", null, {
      timeoutMs: 100,
      onSnapshot: snapshots,
      onError: errors,
    });
    await vi.advanceTimersByTimeAsync(100);
    stream.enqueue(new TextEncoder().encode(frame(1)));
    await vi.advanceTimersByTimeAsync(0);
    expect(errors).toHaveBeenCalledExactlyOnceWith("Work stream timed out", {
      kind: "transport",
    });
    expect(snapshots).not.toHaveBeenCalled();
    stop();
  });

  it("carries HTTP authorization status without publishing the response body", async () => {
    const privateBody = vi
      .fn()
      .mockResolvedValue({ message: "/private/secret", code: "scope_denied" });
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({ ok: false, status: 403, json: privateBody }),
    );
    let finish!: () => void;
    const completed = new Promise<void>((resolve) => {
      finish = resolve;
    });
    const errors = vi.fn(() => finish());
    const stop = subscribeToWork("/project/a/v1/work/subscribe", "test-token", {
      timeoutMs: 30_000,
      onSnapshot: vi.fn(),
      onError: errors,
    });
    await completed;
    expect(errors).toHaveBeenCalledExactlyOnceWith("Work stream unavailable", {
      kind: "http",
      status: 403,
    });
    expect(privateBody).not.toHaveBeenCalled();
    stop();
  });
});
