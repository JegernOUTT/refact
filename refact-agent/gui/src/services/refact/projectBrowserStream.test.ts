import { afterEach, describe, expect, it, vi } from "vitest";
import { subscribeProjectBrowserFrames } from "./projectBrowserStream";

const streamUrl =
  "https://example.test/project/example/v1/browser/project/frames/subscribe";
const streamMime = "application/x-refact-browser-frames";
const target = { runtime_id: "runtime-1", tab_id: "tab-2" };
const metadata = {
  ...target,
  mime: "image/jpeg",
  width: 640,
  height: 360,
  url: "https://example.test/page",
  title: "Page ✓",
  viewport: { width: 1280, height: 720, device_scale_factor: 2 },
};
const frameHeader = { type: "frame", ...metadata };
const jpeg = Uint8Array.of(0xff, 0xd8, 0x00, 0x80, 0xff, 0xd9);
const nextJpeg = Uint8Array.of(0xff, 0xd8, 0xff, 0x00, 0x2a, 0xff, 0xd9);

function concat(...parts: Uint8Array[]) {
  const result = new Uint8Array(
    parts.reduce((length, part) => length + part.byteLength, 0),
  );
  let offset = 0;
  for (const part of parts) {
    result.set(part, offset);
    offset += part.byteLength;
  }
  return result;
}

function lengths(headerLength: number, payloadLength: number) {
  const prefix = new Uint8Array(8);
  const view = new DataView(prefix.buffer);
  view.setUint32(0, headerLength, false);
  view.setUint32(4, payloadLength, false);
  return prefix;
}

function encodeRecord(header: unknown, payload = new Uint8Array(0)) {
  const json = new TextEncoder().encode(JSON.stringify(header));
  return concat(lengths(json.byteLength, payload.byteLength), json, payload);
}

function mockStream({
  contentType = streamMime,
  status = 200,
}: { contentType?: string | null; status?: number } = {}) {
  let controller!: ReadableStreamDefaultController<Uint8Array>;
  let startReading!: () => void;
  const reading = new Promise<void>((resolve) => {
    startReading = resolve;
  });
  const cancel = vi.fn();
  const body = new ReadableStream<Uint8Array>(
    {
      start(streamController) {
        controller = streamController;
      },
      pull: startReading,
      cancel,
    },
    { highWaterMark: 0 },
  );
  const response = new Response(body, {
    status,
    headers: contentType === null ? undefined : { "Content-Type": contentType },
  });
  const fetchMock = vi.fn<typeof fetch>().mockResolvedValue(response);
  vi.stubGlobal("fetch", fetchMock);
  return { controller, reading, cancel, fetchMock };
}

function subscribe(apiKey?: string | null) {
  const abort = new AbortController();
  const onFrame = vi.fn<Parameters<typeof subscribeProjectBrowserFrames>[4]>();
  const subscription = subscribeProjectBrowserFrames(
    streamUrl,
    apiKey,
    target,
    abort.signal,
    onFrame,
  );
  return { abort, onFrame, subscription };
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

describe("subscribeProjectBrowserFrames", () => {
  it("posts the target with bearer auth and preserves split/coalesced frames until EOF", async () => {
    const { controller, reading, fetchMock } = mockStream();
    const { subscription, onFrame } = subscribe("test-token");
    const settled = vi.fn();
    void subscription.then(settled, settled);
    const second = { ...metadata, title: "Second frame" };
    const third = { ...metadata, title: "Third frame", width: 600 };
    const firstRecord = encodeRecord(frameHeader, jpeg);
    const utf8Split = firstRecord.indexOf(0xe2) + 1;
    const splits = [0, 1, 5, 8, utf8Split, firstRecord.length - 2];
    for (let i = 1; i < splits.length; i++) {
      controller.enqueue(firstRecord.subarray(splits[i - 1], splits[i]));
    }
    controller.enqueue(
      concat(
        firstRecord.subarray(firstRecord.length - 2),
        encodeRecord({ type: "heartbeat" }),
        encodeRecord({ type: "frame", ...second }, nextJpeg),
        encodeRecord({ type: "frame", ...third }, jpeg),
      ),
    );

    await reading;
    expect(settled).not.toHaveBeenCalled();
    expect(fetchMock.mock.calls[0][1]?.signal?.aborted).toBe(false);
    controller.close();

    await expect(subscription).rejects.toThrow();
    expect(onFrame.mock.calls.map(([frame]) => frame)).toMatchObject([
      { ...metadata, bytes: jpeg },
      { ...second, bytes: nextJpeg },
      { ...third, bytes: jpeg },
    ]);
    expect(onFrame.mock.calls[0][0].bytes).toBeInstanceOf(Uint8Array);
    expect(fetchMock).toHaveBeenCalledOnce();
    const [url, options] = fetchMock.mock.calls[0];
    const headers = new Headers(options?.headers);
    expect(url).toBe(streamUrl);
    expect(options?.method).toBe("POST");
    expect(JSON.parse(String(options?.body))).toEqual(target);
    expect(headers.get("Content-Type")).toBe("application/json");
    expect(headers.get("Accept")).toBe(streamMime);
    expect(headers.get("Authorization")).toBe("Bearer test-token");
  });

  it.each([null, undefined])("omits authorization for key %s", async (key) => {
    const { controller, fetchMock } = mockStream();
    const { subscription } = subscribe(key);
    controller.close();

    await expect(subscription).rejects.toThrow();
    expect(
      new Headers(fetchMock.mock.calls[0][1]?.headers).has("Authorization"),
    ).toBe(false);
  });

  it.each([null, "application/json", `${streamMime}-invalid`])(
    "rejects response MIME %s without delivering a frame",
    async (contentType) => {
      const { controller, cancel } = mockStream({ contentType });
      const { subscription, onFrame } = subscribe();
      controller.enqueue(encodeRecord(frameHeader, jpeg));

      await expect(subscription).rejects.toThrow();
      expect(onFrame).not.toHaveBeenCalled();
      expect(cancel).toHaveBeenCalledOnce();
    },
  );

  it.each([
    ["an empty header", lengths(0, jpeg.length)],
    ["an oversized header", lengths(32 * 1024 + 1, jpeg.length)],
    ["an oversized payload", lengths(100, 4 * 1024 * 1024 + 1)],
    ["a short JPEG payload", encodeRecord(frameHeader, jpeg.subarray(0, 3))],
    ["an invalid JPEG marker", encodeRecord(frameHeader, new Uint8Array(6))],
    ["a heartbeat with payload", encodeRecord({ type: "heartbeat" }, jpeg)],
    [
      "a terminal error with payload",
      encodeRecord(
        { type: "error", status: 409, detail: "Runtime changed" },
        jpeg,
      ),
    ],
    ["an unknown record type", encodeRecord({ type: "unknown" })],
    ["a non-object header", encodeRecord(null)],
    ["invalid header JSON", concat(lengths(1, 0), Uint8Array.of(0x7b))],
    ["a bad frame width", encodeRecord({ ...frameHeader, width: "640" }, jpeg)],
    ["a bad viewport", encodeRecord({ ...frameHeader, viewport: null }, jpeg)],
    [
      "a bad frame MIME",
      encodeRecord({ ...frameHeader, mime: "image/png" }, jpeg),
    ],
    [
      "a foreign runtime",
      encodeRecord({ ...frameHeader, runtime_id: "other" }, jpeg),
    ],
    ["a foreign tab", encodeRecord({ ...frameHeader, tab_id: "other" }, jpeg)],
  ] as const)(
    "rejects %s and cancels the open reader",
    async (_name, record) => {
      const { controller, cancel } = mockStream();
      const { subscription, onFrame } = subscribe();
      controller.enqueue(record);

      await expect(subscription).rejects.toThrow();
      await expect(subscription).rejects.not.toHaveProperty("status");
      expect(onFrame).not.toHaveBeenCalled();
      expect(cancel).toHaveBeenCalledOnce();
    },
  );

  it.each([
    ["prefix", 7],
    ["header", 12],
    ["payload", encodeRecord(frameHeader, jpeg).length - 1],
  ] as const)("rejects EOF in a truncated %s", async (_name, length) => {
    const { controller } = mockStream();
    const { subscription, onFrame } = subscribe();
    controller.enqueue(encodeRecord(frameHeader, jpeg).subarray(0, length));
    controller.close();

    await expect(subscription).rejects.toThrow();
    expect(onFrame).not.toHaveBeenCalled();
  });

  it.each([409, 410])(
    "preserves terminal HTTP status %s and cancels the response body",
    async (status) => {
      const { cancel } = mockStream({
        contentType: "application/json",
        status,
      });
      const { subscription, onFrame } = subscribe();

      await expect(subscription).rejects.toMatchObject({ status });
      expect(onFrame).not.toHaveBeenCalled();
      expect(cancel).toHaveBeenCalledOnce();
    },
  );

  it.each([409, 410])(
    "stops at terminal in-band status %s and cancels the reader",
    async (status) => {
      const { controller, cancel } = mockStream();
      const { subscription, onFrame } = subscribe();
      const detail = "Target changed";
      controller.enqueue(
        concat(
          encodeRecord(frameHeader, jpeg),
          encodeRecord({ type: "error", status, detail }),
          encodeRecord(frameHeader, nextJpeg),
        ),
      );

      await expect(subscription).rejects.toMatchObject({ status });
      expect(onFrame).toHaveBeenCalledOnce();
      expect(onFrame.mock.calls[0][0].bytes).toEqual(jpeg);
      expect(cancel).toHaveBeenCalledOnce();
    },
  );

  it("propagates abort to fetch and cancels a pending read", async () => {
    const { reading, cancel, fetchMock } = mockStream();
    const { abort, subscription, onFrame } = subscribe();
    const settled = vi.fn();
    const completion = subscription.then(settled, settled);
    await reading;
    const signal = fetchMock.mock.calls[0][1]?.signal;
    expect(signal?.aborted).toBe(false);
    expect(settled).not.toHaveBeenCalled();

    abort.abort();
    await completion;

    expect(signal?.aborted).toBe(true);
    expect(cancel).toHaveBeenCalledOnce();
    expect(onFrame).not.toHaveBeenCalled();
  });

  it("does not publish a coalesced frame after abort from a callback", async () => {
    const { controller, cancel } = mockStream();
    const { abort, subscription, onFrame } = subscribe();
    onFrame.mockImplementationOnce(() => abort.abort());
    controller.enqueue(
      concat(
        encodeRecord(frameHeader, jpeg),
        encodeRecord(frameHeader, nextJpeg),
      ),
    );

    await subscription.catch(() => undefined);
    expect(onFrame).toHaveBeenCalledOnce();
    expect(cancel).toHaveBeenCalledOnce();
  });

  it("times out after 30 seconds without stream bytes", async () => {
    vi.useFakeTimers();
    const { reading, cancel, fetchMock } = mockStream();
    const { subscription, onFrame } = subscribe();
    const rejected = expect(subscription).rejects.toThrow(/timed out/i);
    await reading;
    await vi.advanceTimersByTimeAsync(29_999);
    expect(cancel).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(1);
    await rejected;
    expect(fetchMock.mock.calls[0][1]?.signal?.aborted).toBe(true);
    expect(cancel).toHaveBeenCalledOnce();
    expect(onFrame).not.toHaveBeenCalled();
  });
});
